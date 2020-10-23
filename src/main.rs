// https://users.rust-lang.org/t/cargo-build-shows-unresolved-import/45445/7
use std::path::{Path, PathBuf};

use youdao_dict_export::{
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

use std::io::{self, BufReader};
use std::str;
use tokio::io::AsyncBufReadExt;
use tokio::stream::StreamExt;
use tokio::{
    fs::{self as afs},
    io::{self as aio},
    prelude::*,
};

#[derive(StructOpt, Debug)]
#[structopt(name = "basic")]
struct Opt {
    /// 打印格式。当前支持：min=仅输出单词word
    #[structopt(default_value = "min", long)]
    print_mode: String,

    /// 过滤开始时间。格式支持`"%Y-%m-%d`。默认1970-01-01.
    #[structopt(long)]
    start_date: Option<String>,

    /// 过滤终止时间。格式支持`"%Y-%m-%d`。默认`today`
    #[structopt(long)]
    end_date: Option<String>,

    /// 在输出前过滤单词数量。offset>0表示顺序输出的单词数量；offset<0表示从最后开始过滤的；offset=0表示不过滤
    #[structopt(long, default_value = "0")]
    offset: isize,

    /// 是否从dict.youdao.com中重新加载单词数据。默认false
    #[structopt(short)]
    refetch: bool,

    /// 单词持久化文件。默认`$HOME/.youdao-words.json`
    #[structopt(parse(from_os_str), long)]
    words_file: Option<PathBuf>,

    /// 是否输出debug日志
    #[structopt(short)]
    verbose: bool,
}

#[derive(StructOpt, Debug)]
struct AppOpt {
    /// 是否输出debug日志
    #[structopt(long)]
    verbose: bool,

    /// config配置文件路径。如果为空则默认从$HOME/dict-config.yml文件加载
    #[structopt(long)]
    config_path: Option<String>,

    #[structopt(subcommand)]
    sub_cmd: Option<SubCommand>,
}
#[derive(StructOpt, Debug)]
enum SubCommand {
    Youdao {
        /// 从dict.youdao.com中重新加载单词数据。
        #[structopt(short)]
        refresh: bool,

        #[structopt(short)]
        list: bool,

        /// 过滤开始时间。格式支持`"%Y-%m-%d`。默认1970-01-01.
        #[structopt(long)]
        start_date: Option<String>,

        /// 过滤终止时间。格式支持`"%Y-%m-%d`。默认`today`
        #[structopt(long)]
        end_date: Option<String>,

        /// 在输出前过滤单词数量。offset>0表示顺序输出的单词数量；offset<0表示从最后开始过滤的；offset=0表示不过滤
        #[structopt(long, default_value = "0")]
        offset: isize,
    },
    Maimemo {
        #[structopt(short, long)]
        list: bool,

        #[structopt(short, long)]
        refresh: bool,

        #[structopt(long)]
        notepad_id: Option<String>,

        // dict youdao --offset 10 | dict maimemo --upload --id 213423
        #[structopt(short, long)]
        upload: bool,

        /// 在upload时自动插入时间戳
        #[structopt(short, required_if("upload", "true"))]
        timestamp: bool,

        #[structopt(short)]
        appending: bool,
    },
}

pub struct MaimemoApp {
    notepads: Vec<Notepad>,
    config: AppConfig,
    is_updated: bool,
}

impl std::ops::Drop for MaimemoApp {
    /// 当self被更新后保存notepads数据
    fn drop(&mut self) {
        if !self.is_updated {
            return;
        }
        if let Err(e) = save_json(&self.notepads, self.config.get_dictionary_path()) {
            error!("notepads persistence failed. {}", e);
        } else {
            info!("notepads persistence successful");
        }
    }
}

impl MaimemoApp {
    /// 从web maimemo上
    pub async fn from_web<'a>(config: AppConfig) -> Result<Self, String> {
        let notepads = {
            let mut client = MaimemoClient::new(&config)?;
            if !client.has_logged() {
                debug!("Signing in");
                client.login().await?;
            }
            let mut notepads = client.get_notepad_list().await?;
            debug!("got notepad list: {:?}", notepads);
            for notepad in &mut notepads {
                let contents = client
                    .get_notepad_contents(notepad.get_notepad_id())
                    .await?;
                notepad.set_contents(Some(contents));
            }
            notepads
        };
        Ok(MaimemoApp {
            notepads,
            config,
            is_updated: true,
        })
    }

    pub async fn from_file(config: AppConfig) -> Result<Self, String> {
        load_from_json_file(config.get_dictionary_path())
            .await
            .map(|notepads| Self {
                notepads,
                config,
                is_updated: false,
            })
    }

    pub async fn upload_notepad(
        &mut self,
        notepad_id: &str,
        is_appending: bool,
        timestamp: bool,
    ) -> Result<(), String> {
        let client = MaimemoClient::new(&self.config)?;
        if !client.has_logged() {
            return Err(format!("Not logged in"));
        }
        let new_notepad = self
            .get_uploaded_notepad(notepad_id, is_appending, timestamp)
            .await?;
        while let Err(e) = self.upload(&client, new_notepad.clone()).await {
            print!("upload error: {}. \nDo you want to try again [y]:", e);
            println!();
            let line = aio::BufReader::new(aio::stdin())
                .lines()
                .next_line()
                .await
                .map_err(|e| format!("{:?}", e))?
                .ok_or("line is none")?;
            if line != "y" {
                return Err(format!("User confirm exit: {}", line));
            }
        }
        if self
            .notepads
            .iter_mut()
            .find(|n| n.get_notepad_id() == notepad_id)
            .map(|n| *n = new_notepad)
            .is_none()
        {
            warn!("Failed to update local Notepad. please use -r refresh local data");
        }
        self.is_updated = true;
        debug!("upload notepad successful");
        Ok(())
    }

    async fn upload<'a>(&self, client: &MaimemoClient<'a>, notepad: Notepad) -> Result<(), String> {
        let captcha = self.read_captcha_from_stdin(&client).await?;
        // save notepad
        client.save_notepad(notepad, captcha).await
    }

    async fn read_captcha_from_stdin<'a>(
        &self,
        client: &MaimemoClient<'a>,
    ) -> Result<String, String> {
        let captcha_contents = client.refresh_captcha().await?;
        // Display captcha on the terminal
        let img = image::load_from_memory(&captcha_contents).map_err(|e| format!("{:?}", e))?;
        debug!("Exporting image content");
        viuer::print(&img, &viuer::Config::default()).expect("Image printing failed.");
        debug!("Waiting for input captcha");
        print!("please enter captcha: ");
        // read captcha on stdin
        let mut lines = aio::BufReader::new(aio::stdin()).lines();
        if let Some(line) = lines.next_line().await.map_err(|e| format!("{:?}", e))? {
            debug!("read line: {}", line);
            Ok(line)
        } else {
            error!("read line error");
            Err("read line error".to_string())
        }
    }

    async fn get_uploaded_notepad(
        &self,
        notepad_id: &str,
        is_appending: bool,
        timestamp: bool,
    ) -> Result<Notepad, String> {
        let mut notepad = self
            .notepads
            .iter()
            .find(|n| n.get_notepad_id() == notepad_id)
            .ok_or(format!("not found notepad_id: {}", notepad_id))?
            .clone();
        let contents = notepad.get_contents_mut().unwrap();
        if !is_appending {
            contents.clear();
        }
        if timestamp {
            let timestr = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            contents.push_str(&format!("\n\n# {} Auto insert\n", timestr));
        }
        debug!("Reading content from Stdin");
        Self::read_contents_from_stdin(contents).await?;
        Ok(notepad)
    }

    async fn read_contents_from_stdin(contents: &mut String) -> Result<(), String> {
        let mut lines = aio::BufReader::new(aio::stdin()).lines();
        while let Some(line) = lines.next().await {
            let line = line.map_err(|e| format!("{:?}", e))?;
            debug!("read line: {}", line);
            contents.push_str(&line);
            contents.push('\n');
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<&Notepad> {
        self.notepads.iter().collect()
    }

    pub fn list_contents(&self, notepad_id: &str) -> Option<&str> {
        self.notepads
            .iter()
            .find(|n| n.get_notepad_id() == notepad_id)
            .as_ref()
            .and_then(|n| n.get_contents())
    }
}

struct YoudaoApp {
    word_items: Vec<WordItem>,
    is_local: bool,
    config: AppConfig,
}

impl std::ops::Drop for YoudaoApp {
    fn drop(&mut self) {
        if self.is_local {
            return;
        }
        if let Err(e) = save_json(&self.word_items, self.config.get_dictionary_path()) {
            error!("word items persistence failed. {}", e);
        } else {
            debug!("word items persistence successful");
        }
    }
}

impl YoudaoApp {
    pub async fn from_file(config: AppConfig) -> Result<Self, String> {
        let word_items = load_from_json_file(config.get_dictionary_path()).await?;
        Ok(Self {
            word_items,
            is_local: true,
            config,
        })
    }

    pub async fn from_web(config: AppConfig) -> Result<Self, String> {
        let word_items = {
            let mut client = YoudaoClient::new(&config)?;
            // client
            if !client.has_logged() {
                debug!("Signing in");
                client.login().await?;
            }
            let word_items = client.get_words().await?;
            word_items
        };
        Ok(Self {
            word_items,
            is_local: false,
            config,
        })
    }

    /// 查询单词。可以通过date和排序后前后过滤数量
    ///
    /// 通过时间区间`[start_date, end_date]`过虑单词并以升序排列。时间格式：`"%Y-%m-%d %H:%M:%S`
    ///
    /// - [Remove an element from a vector](https://stackoverflow.com/a/40310140)
    pub fn list(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
        offset: isize,
    ) -> Vec<&WordItem> {
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
        words
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
            let (suffix, format) = (" 00:00:00", "%Y-%m-%d %H:%M:%S");
            match Utc.datetime_from_str(&(date.to_string() + suffix), format) {
                Ok(date) => Some(date),
                Err(e) => {
                    warn!("skip parse date {} error: {}", date, e);
                    None
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
            .filter_module("youdao_dict_export", log::LevelFilter::Debug)
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
        panic!(format!("not found config file in path: {}", config_path))
    };
    match opt.sub_cmd {
        Some(SubCommand::Youdao {
            list,
            refresh,
            end_date,
            start_date,
            offset,
        }) => {
            let config = config.youdao();
            let app = if refresh {
                match YoudaoApp::from_web(config).await {
                    Ok(app) => {
                        debug!("youdao dict refreh successful");
                        app
                    }
                    Err(e) => {
                        // 目录不存在。
                        if e.contains("NotFound") {
                            eprintln!("Youdao dict file not found, Please check that the directory exists: {}", e);
                            return;
                        }
                        panic!("YoudaoApp cannot be created through web: {}", e);
                    }
                }
            } else {
                match YoudaoApp::from_file(config).await {
                    Ok(app) => app,
                    Err(e) => {
                        if e.contains("NotFound") {
                            eprintln!(
                                "Youdao dict file not found, please use -r to refresh. {}",
                                e
                            );
                            return;
                        }
                        panic!(format!("YoudaoApp cannot be created through file: {}", e));
                    }
                }
            };
            if list {
                app.list(start_date.as_deref(), end_date.as_deref(), offset)
                    .iter()
                    .for_each(|item| println!("{}", item.word));
            }
        }
        Some(SubCommand::Maimemo {
            list,
            notepad_id,
            timestamp,
            upload,
            refresh,
            appending,
        }) => {
            let config = config.maimemo();
            let mut app = if refresh {
                match MaimemoApp::from_web(config).await {
                    Ok(app) => {
                        debug!("maimemo dict refreh successful");
                        app
                    }
                    Err(e) => {
                        // 目录不存在。
                        if e.contains("NotFound") {
                            eprintln!("Maimemo dict file not found, Please check that the directory exists: {}", e);
                            return;
                        }
                        panic!("MaimemoApp cannot be created through web: {}", e);
                    }
                }
            } else {
                match MaimemoApp::from_file(config).await {
                    Ok(app) => app,
                    Err(e) => {
                        // 目录不存在。
                        if e.contains("NotFound") {
                            eprintln!(
                                "Maimemo dict file not found, Please use -r to refresh. {}",
                                e
                            );
                            return;
                        }
                        panic!("MaimemoApp cannot be created through file: {}", e);
                    }
                }
            };
            if list {
                if let Some(notepad_id) = notepad_id {
                    if let Some(contents) = app.list_contents(&notepad_id) {
                        println!("{}", contents);
                    } else {
                        eprintln!("not found any contents with notepad_id: {}", notepad_id);
                    }
                } else {
                    app.list().iter().for_each(|item| println!("{}", item));
                }
                return;
            }

            if upload {
                if let Some(notepad_id) = notepad_id {
                    match app.upload_notepad(&notepad_id, appending, timestamp).await {
                        Err(e) => {
                            eprintln!("upload error: {}", e);
                        }
                        Ok(()) => println!("upload successful"),
                    }
                } else {
                    eprintln!("Please specify notepad id");
                }
                return;
            }
        }
        cmd => panic!("unsupported command: {:?}", cmd),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    /*
        #[test]
        fn filter_date_test() {
            let words = get_words();
            let mut res = words.to_vec();
            filter_date(&mut res, None, None);
            assert_eq!(words, res);

            let mut res = words.to_vec();
            filter_date(&mut res, Some("2019-8-01"), None);
            assert_eq!(res.len(), 2);
            // 过虑了最小时间
            assert!(res
                .iter()
                .find(|w| w.modified_time == 1564152487000)
                .is_none());

            let mut res = words.to_vec();
            filter_date(&mut res, None, Some("2019-8-01"));
            assert_eq!(res.len(), 1);
            // 还剩下最小时间
            assert!(res
                .iter()
                .find(|w| w.modified_time == 1564152487000)
                .is_some());

            let mut res = words.to_vec();
            // 刚好还剩下最小时间
            filter_date(&mut res, Some("2019-7-26"), Some("2019-8-01"));
            assert_eq!(res.len(), 1);
            assert!(res
                .iter()
                .find(|w| w.modified_time == 1564152487000)
                .is_some());

            let mut res = words.to_vec();
            // 全过滤
            filter_date(&mut res, Some("2019-3-11"), Some("2019-4-01"));
            assert!(res.is_empty());
        }

        #[test]
        fn filter_date_order() {
            let mut words = get_words();
            let mut res = words.to_vec();
            // 过虑最小时间
            filter_date(&mut res, Some("2019-8-26"), Some("2019-10-01"));
            assert_eq!(res.len(), 2);
            // 升序
            words.sort_unstable_by(|a, b| b.modified_time.cmp(&a.modified_time));
            assert_eq!(words[0..2], res[..]);
        }

        #[test]
        fn filter_offset_test() {
            let mut words = get_words();
            let len = words.len();
            let offset = 2;
            filter_offset(&mut words, offset);
            assert_eq!(words.len(), offset as usize);
            for i in 0..offset as usize {
                assert_eq!(get_words()[i], words[i]);
            }

            let mut words = get_words();
            let offset = -2;
            filter_offset(&mut words, offset);
            assert_eq!(words.len(), -offset as usize);
            for i in 0..-offset as usize {
                // 偏移后的下标(len as isize + offset) as usize + i
                assert_eq!(get_words()[(len as isize + offset) as usize + i], words[i]);
            }
            // assert_eq!(get_words()[(len - offset) as usize], words[len - offset]);
        }
    */
    fn get_words() -> Vec<WordItem> {
        // date1: Fri Jul 26 22:48:07 CST 2019
        // date2: Thu Sep 05 17:03:58 CST 2019
        // date3: Tue Sep 17 15:51:15 CST 2019
        let data = r#"[
        {
            "itemId": "9cef81095a2a7e35c169c990b37839eb",
            "bookId": "0",
            "bookName": "无标签",
            "word": "Accommodate",
            "trans": "vt. 容纳；使适应；供应；调解\nvi. 适应；调解",
            "phonetic": "[ə'kɒmədeɪt]",
            "modifiedTime": 1564152487000
        },
        {
            "itemId": "3223bb4547bd4bad4f17d31e207c6b3c",
            "bookId": "0",
            "bookName": "无标签",
            "word": "Acronym",
            "trans": "n. 首字母缩略词",
            "phonetic": "['ækrənɪm]",
            "modifiedTime": 1567674238000
        },
        {
            "itemId": "cf5b783279932d2d14d2241f0a166e76",
            "bookId": "0",
            "bookName": "无标签",
            "word": "Antenna",
            "trans": "n. [电讯] 天线；[动] 触角，[昆] 触须\nn. (Antenna)人名；(法)安泰纳",
            "phonetic": "[æn'tenə]",
            "modifiedTime": 1568706675000
        }]"#;

        serde_json::from_str(data).unwrap()
    }
}
