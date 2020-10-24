// https://users.rust-lang.org/t/cargo-build-shows-unresolved-import/45445/7
use std::path::Path;

use dict::{
    client::{
        maimemo_client::{MaimemoClient, Notepad},
        youdao_client::{WordItem, YoudaoClient},
    },
    config::*,
};

use chrono::{DateTime, TimeZone, Utc};
use structopt::StructOpt;
#[macro_use]
extern crate log;

use std::fs;
use std::io::{self, prelude::*, Write};
use std::str;

#[derive(StructOpt, Debug)]
struct AppOpt {
    /// 是否输出debug日志
    #[structopt(short, long)]
    verbose: bool,

    /// config配置文件路径。如果为空则默认从$HOME/dict-config.yml文件加载
    #[structopt(long)]
    config_path: Option<String>,

    #[structopt(subcommand)]
    sub_cmd: Option<SubCommand>,
}
#[derive(StructOpt, Debug)]
enum SubCommand {
    /// youdao
    Yd {
        /// 从dict.youdao.com中重新加载单词数据。
        #[structopt(short)]
        refresh: bool,

        /// 显示单词
        #[structopt(short, long)]
        list: bool,

        /// 过滤开始时间。格式支持`"%Y-%m-%d`。默认1970-01-01.
        #[structopt(short, long)]
        start_date: Option<String>,

        /// 过滤终止时间。格式支持`"%Y-%m-%d`。默认`today`
        #[structopt(short, long)]
        end_date: Option<String>,

        /// 在输出前过滤单词数量。offset>0表示顺序输出的单词数量；offset<0表示从最后开始过滤的；offset=0表示不过滤
        #[structopt(long, default_value = "0")]
        offset: isize,
    },
    /// maimemo
    Mm {
        /// 查询maimemo notepads。默认仅查询notepad info，不显示内容。
        /// 可以指定notepad_id显示内容
        #[structopt(short, long)]
        list: bool,

        /// 从maimemo中加载notepads到文件
        #[structopt(short, long)]
        refresh: bool,

        /// 可用于在list与upload时指定notepad
        #[structopt(long = "id")]
        notepad_id: Option<String>,

        // dict youdao --offset 10 | dict maimemo --upload --id 213423
        /// 从stdin中上传内容到maimemo
        #[structopt(short, long)]
        upload: bool,

        /// 在upload时自动插入时间戳
        #[structopt(short, long, required_if("upload", "true"))]
        timestamp: bool,

        /// 在upload时在之前的基础上增加而不是覆盖
        #[structopt(short, long, required_if("upload", "true"))]
        appending: bool,
    },
}

pub struct MaimemoApp<'a> {
    notepads: Vec<Notepad>,
    dictionary_path: String,
    is_updated: bool,
    client: MaimemoClient,
    input: io::BufReader<Box<dyn Read + 'a>>,
    output: io::BufWriter<Box<dyn Write + 'a>>,
}

impl<'a> std::ops::Drop for MaimemoApp<'a> {
    /// 当self被更新后保存notepads数据
    fn drop(&mut self) {
        if !self.is_updated {
            return;
        }
        if let Err(e) = save_json(&self.notepads, &self.dictionary_path) {
            error!("notepads persistence failed. {}", e);
        } else {
            info!(
                "notepads persistence successful path: {}",
                self.dictionary_path
            );
        }
    }
}

impl<'a> MaimemoApp<'a> {
    pub async fn new(
        config: AppConfig,
        is_local: bool,
        input: impl io::Read + 'a,
        output: impl io::Write + 'a,
    ) -> MaimemoApp<'a> {
        let dictionary_path = config.get_dictionary_path().to_string();
        let mut client = MaimemoClient::new(config)
            .unwrap_or_else(|e| panic!("new maimemo client failed: {}", e));

        let notepads = if is_local {
            load_from_json_file(&dictionary_path)
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "load maimemo dictionary error: {}, dictionary_path: {}",
                        e, dictionary_path
                    )
                })
        } else {
            // load from web
            if !client.has_logged() {
                debug!("Signing in");
                client
                    .login()
                    .await
                    .unwrap_or_else(|e| panic!("maimemo client login failed: {}", e));
            }
            client
                .get_notepads()
                .await
                .unwrap_or_else(|e| panic!("get notepads failed: {}", e))
        };
        Self {
            client,
            dictionary_path,
            notepads,
            is_updated: is_local,
            input: io::BufReader::new(Box::new(input)),
            output: io::BufWriter::new(Box::new(output)),
        }
    }
    /// 从web maimemo上加载notepads
    pub async fn with_stdio(config: AppConfig, is_local: bool) -> MaimemoApp<'a> {
        // 修复在stdin使用管道线时无法使用用户输入问题
        let path = "/dev/tty";
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap_or_else(|e| panic!("open file error: {}, path: {}", e, path));
        MaimemoApp::new(config, is_local, file, io::stdout()).await
    }

    /// 从stdin将指定notepad_id的内容更新到maimemo web上。当保存成功后更新
    /// notepads，并在self drop退出时保存
    ///
    /// 如果is_appending=true则在原notepad上添加，否则清楚仅用stdin中的内容
    ///
    /// 如果timestamp=true则自动插入时间戳
    ///
    /// # Errors
    ///
    /// 如果client未登录。
    pub async fn upload_notepad(
        &mut self,
        contents_read: impl io::Read,
        notepad_id: &str,
        is_appending: bool,
        timestamp: bool,
    ) {
        if !self.client.has_logged() {
            panic!("Not logged in. please use -r refresh");
        }
        let new_notepad = self
            .build_uploaded_notepad(contents_read, notepad_id, is_appending, timestamp)
            .await
            .unwrap_or_else(|e| panic!("build notepad error: {}", e));
        loop {
            let captcha = self
                .read_captcha()
                .await
                .unwrap_or_else(|e| panic!("read captcha error: {}", e));
            // save notepad
            if let Err(e) = self
                .client
                .save_notepad(new_notepad.clone(), captcha.clone())
                .await
            {
                debug!(
                    "upload failed. notepad: {}, captcha: {}",
                    new_notepad, captcha
                );
                print!("upload error: {}. \nDo you want to try again [y]:", e);
                let line = self
                    .read_line()
                    .unwrap_or_else(|e| panic!("read user input error: {}", e));
                if line == "y" {
                    debug!("exiting with input: {}", line);
                    return;
                }
            } else {
                break;
            }
        }
        if self
            .notepads
            .iter_mut()
            .find(|n| n.get_notepad_id() == notepad_id)
            .map(|n| *n = new_notepad)
            .is_none()
        {
            warn!(
                "Failed to update local Notepad. not found notepad_id: {}",
                notepad_id
            );
            panic!("save notepad successful, but Failed to update local Notepad. please use -r refresh local data")
        } else {
            self.is_updated = true;
            debug!("upload notepad successful for notepad_id: {}", notepad_id);
        }
    }

    fn read_line(&mut self) -> Result<String, String> {
        trace!("reading a line");
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) => {
                debug!("read has reached EOF. line: {}", line);
            }
            Ok(size) => {
                debug!("read {} bytes. line: {}", size, line);
            }
            Err(e) => {
                error!("read line: {}, error: {}", e, line);
                return Err(e.to_string());
            }
        }
        if line.is_empty() {
            error!("read line is empty");
            Err("read line is empty".to_string())
        } else {
            Ok(line)
        }
    }

    async fn read_captcha(&mut self) -> Result<String, String> {
        trace!("loading captcha from maimemo service");
        let captcha_contents = self.client.refresh_captcha().await?;
        // Display captcha on the terminal
        trace!("Printing image content");
        let img = image::load_from_memory(&captcha_contents).map_err(|e| format!("{:?}", e))?;
        viuer::print(
            &img,
            &viuer::Config {
                absolute_offset: false,
                ..viuer::Config::default()
            },
        )
        .expect("Image printing failed.");
        debug!("Waiting for input captcha");
        println!("please enter captcha: ");
        // read captcha on stdin
        self.read_line()
    }

    /// 从stdin中读取并构造出notepad。
    ///
    /// 如果is_appending=true则在原notepad上添加，否则清楚仅用stdin中的内容
    ///
    /// 如果timestamp=true则自动插入时间戳
    async fn build_uploaded_notepad(
        &mut self,
        contents_read: impl io::Read,
        notepad_id: &str,
        is_appending: bool,
        timestamp: bool,
    ) -> Result<Notepad, String> {
        trace!("Building a new notepad");
        let mut notepad = self
            .notepads
            .iter()
            .find(|n| n.get_notepad_id() == notepad_id)
            .ok_or(format!("not found notepad_id: {}", notepad_id))?
            .clone();
        let contents = notepad.get_contents_mut().unwrap();
        if !is_appending {
            debug!("Emptying original Notepad contents");
            contents.clear();
        }
        if timestamp {
            let timestr = chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let s = format!("\n# {} Auto insert\n", timestr);
            debug!("Inserting timestamp string: {}", s);
            contents.push_str(&s);
        }
        // read contents
        debug!("reading contents");
        io::BufReader::new(contents_read)
            .read_to_string(contents)
            .map_err(|e| {
                error!(
                    "read contents to string error: {}, contents: {}",
                    e, contents
                );
                format!("read contents to string error: {}", e)
            })?;
        debug!("read contents:\n{}", contents);
        Ok(notepad)
    }

    /// 打印所有notepad概要信息
    pub fn list(&mut self) {
        for n in &self.notepads {
            writeln!(self.output, "{}", n).unwrap_or_else(|e| panic!("write notepad error: {}", e))
        }
    }

    /// 输出指定id的notepad内容
    pub fn list_contents(&mut self, notepad_id: &str) {
        let contents = self
            .notepads
            .iter()
            .find(|n| n.get_notepad_id() == notepad_id)
            .unwrap_or_else(|| panic!("not found notepad for notepad_id: {}", notepad_id))
            .get_contents()
            .unwrap_or_else(|| panic!("not found contents for notepad_id: {}", notepad_id));
        writeln!(self.output, "{}", contents)
            .unwrap_or_else(|e| panic!("write notepad contents error: {}", e));
    }
}

#[allow(dead_code)]
struct YoudaoApp {
    word_items: Vec<WordItem>,
    is_local: bool,
    dictionary_path: String,
    client: YoudaoClient,
    output: io::BufWriter<Box<dyn Write>>,
}

impl std::ops::Drop for YoudaoApp {
    /// 退出时更新
    fn drop(&mut self) {
        if self.is_local {
            return;
        }
        if let Err(e) = save_json(&self.word_items, &self.dictionary_path) {
            error!("word items persistence failed. {}", e);
        } else {
            info!(
                "word items persistence successful path: {}",
                self.dictionary_path
            );
        }
    }
}

impl YoudaoApp {
    /// 从file中构造
    pub async fn from_file(config: AppConfig) -> Self {
        let dictionary_path = config.get_dictionary_path().to_string();
        let client =
            YoudaoClient::new(config).unwrap_or_else(|e| panic!("youdao client new failed. {}", e));
        let word_items = load_from_json_file(&dictionary_path)
            .await
            .unwrap_or_else(|e| panic!("youdao load json failed. {}", e));
        Self {
            word_items,
            is_local: true,
            client,
            dictionary_path,
            output: io::BufWriter::new(Box::new(io::stdout())),
        }
    }

    /// 从youdao web上获取words构造
    pub async fn from_web(config: AppConfig) -> Self {
        let dictionary_path = config.get_dictionary_path().to_string();
        let mut client =
            YoudaoClient::new(config).unwrap_or_else(|e| panic!("new youdaoclient error: {}", e));
        if !client.has_logged() {
            debug!("Signing in");
            if let Err(e) = client.login().await {
                error!("youdao login error: {}", e);
                panic!("youdao login error: {}", e);
            }
        }
        let word_items = match client.get_words().await {
            Ok(w) => w,
            Err(e) => {
                error!("youdao get words error: {}", e);
                panic!("youdao get words error: {}", e);
            }
        };
        Self {
            word_items,
            is_local: false,
            dictionary_path,
            client,
            output: io::BufWriter::new(Box::new(io::stdout())),
        }
    }

    /// 查询单词。可以通过date和排序后前后过滤数量
    ///
    /// 通过时间区间`[start_date, end_date]`过虑单词并以升序排列。时间格式：`"%Y-%m-%d`
    ///
    /// - [Remove an element from a vector](https://stackoverflow.com/a/40310140)
    pub fn list(&mut self, start_date: Option<&str>, end_date: Option<&str>, offset: isize) {
        let start = Self::parse_date(start_date);
        let end = Self::parse_date(end_date);
        let mut words = self
            .word_items
            .iter()
            .filter(|w| {
                let date = Utc.timestamp_millis(w.modified_time as i64);
                match (start, end) {
                    (None, None) => true,
                    (Some(start), Some(end)) => date >= start && date <= end,
                    (Some(start), None) => date >= start,
                    (None, Some(end)) => date <= end,
                }
            })
            .collect::<Vec<_>>();
        words.sort_unstable_by(|a, b| b.modified_time.cmp(&a.modified_time));
        Self::filter_offset(&mut words, offset);
        for item in words {
            if let Err(e) = writeln!(self.output, "{}", item.word) {
                error!("writeln error: {}, worditem: {:?}", e, item);
            }
        }
    }

    /// 取出offset个元素。如果offset<0，则从后取出offset个。如果offset==0则不会过滤任何元素
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// let mut words = vec![1,2,3];
    /// let offset = -2;
    /// filter_offset(&mut words, offset); // words: [2,3]
    /// ```
    fn filter_offset<T>(words: &mut Vec<&T>, offset: isize) {
        debug!("filter offset: {}", offset);
        if offset > 0 {
            let mut count = words.len() - offset as usize;
            while count > 0 {
                count -= 1;
                words.pop();
            }
        } else if offset < 0 {
            let start = (words.len() as isize + offset) as usize;
            words.drain(0..start);
            // replace
        }
    }

    /// 解析时间格式
    fn parse_date(date: Option<&str>) -> Option<DateTime<Utc>> {
        date.and_then(|date| {
            // 不能用date，只能用datetime
            let (suffix, format) = (" 00:00:00", "%Y-%m-%d %H:%M:%S");
            match Utc.datetime_from_str(&(date.to_string() + suffix), format) {
                Ok(date) => Some(date),
                Err(e) => {
                    panic!("parse date {} error: {}", date, e);
                }
            }
        })
    }
}

use std::env;
#[tokio::main]
async fn main() {
    let opt: AppOpt = AppOpt::from_args();
    // debug log
    if opt.verbose {
        pretty_env_logger::formatted_builder()
            // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
            .filter_module("dict", log::LevelFilter::Debug)
            .init();
    }
    let config_path = opt
        .config_path
        .as_ref()
        .map(|s| s.to_owned())
        // default $HOME/filename
        .unwrap_or_else(|| {
            let filename = "dict-config.yml";
            let home = env::var("HOME").expect("not found $HOME var");
            let path = Path::new(&home).join(filename);
            path.to_str().unwrap().to_string()
        });
    let mut config = if let Ok(config) = Config::from_yaml_file(&config_path) {
        config
    } else {
        eprintln!("not found config file in path: {}", config_path);
        return;
    };
    match opt.sub_cmd {
        Some(SubCommand::Yd {
            list,
            refresh,
            end_date,
            start_date,
            offset,
        }) => {
            let config = config.youdao();
            let mut app = if refresh {
                YoudaoApp::from_web(config).await
            } else {
                YoudaoApp::from_file(config).await
            };
            if list {
                app.list(start_date.as_deref(), end_date.as_deref(), offset);
            }
        }
        Some(SubCommand::Mm {
            list,
            notepad_id,
            timestamp,
            upload,
            refresh,
            appending,
        }) => {
            let config = config.maimemo();
            let mut app = MaimemoApp::with_stdio(config, !refresh).await;
            if list {
                if let Some(notepad_id) = notepad_id {
                    app.list_contents(&notepad_id);
                } else {
                    app.list()
                }
                return;
            }

            if upload {
                if let Some(notepad_id) = notepad_id {
                    app.upload_notepad(io::stdin(), &notepad_id, appending, timestamp)
                        .await;
                }

                return;
            }
        }
        cmd => panic!("unsupported command: {:?}", cmd),
    };
}

#[cfg(test)]
mod maimemo_tests {
    use super::*;
    const CONFIG_PATH: &'static str = "config.yml";

    async fn mocked_maimemo_data<'a>(
        is_local: bool,
    ) -> Result<(MaimemoApp<'a>, Vec<Notepad>), String> {
        pretty_env_logger::formatted_builder()
            .filter_module("dict", log::LevelFilter::Debug)
            .init();
        let config = Config::from_yaml_file(CONFIG_PATH)?;
        let notepads =
            load_from_json_file::<Vec<Notepad>>(config.get_maimemo().get_dictionary_path()).await?;
        let (input, output) = (io::Cursor::new(""), io::Cursor::new(Vec::new()));
        Ok((
            MaimemoApp::new(config.maimemo.unwrap(), is_local, input, output).await,
            notepads,
        ))
    }

    #[tokio::test]
    async fn list() -> Result<(), String> {
        let (mut app, notepads) = mocked_maimemo_data(true).await?;
        app.list();
        let mut data = vec![];
        notepads.iter().for_each(|n| {
            let s = n.to_string() + "\n";
            s.bytes().for_each(|b| data.push(b));
        });
        assert_eq!(data, app.output.buffer());
        Ok(())
    }

    #[tokio::test]
    async fn list_contents() -> Result<(), String> {
        let (mut app, notepads) = mocked_maimemo_data(true).await?;
        let notepad_id = "695835";
        app.list_contents(notepad_id);
        let mut data = vec![];
        notepads
            .iter()
            .find(|n| n.get_notepad_id() == notepad_id)
            .and_then(|n| n.get_contents())
            .map(|s| s.bytes().for_each(|b| data.push(b)));
        data.push('\n' as u8);
        assert_eq!(data, app.output.buffer());
        Ok(())
    }

    // #[tokio::test]
    // async fn maimemo_save() -> Result<(), String> {
    //     init_log();
    //     let config = Config::from_yaml_file(CONFIG_PATH)?;
    //     let mut app = MaimemoApp::from_file(config.maimemo.unwrap()).await;
    //     app.upload_notepad(io::Cursor::new("test"), "695835", false, false)
    //         .await;
    //     Ok(())
    // }
}
