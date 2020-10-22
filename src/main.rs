// https://users.rust-lang.org/t/cargo-build-shows-unresolved-import/45445/7
use std::path::{Path, PathBuf};

use youdao_dict_export::{config, maimemo_client::*, word_store::*, youdao_client::*, *};

use chrono::{TimeZone, Utc};
use structopt::StructOpt;
#[macro_use]
extern crate log;

use std::io::{self, Read, Write};
use std::process::*;
use std::str;

use tokio::{
    fs::{self as afs},
    io::{self as aio, AsyncReadExt},
};
use viuer::{print, Config};
use youdao_dict_export::{config::*, maimemo_client::*, word_store::*, youdao_client::*, *};

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

        #[structopt(long)]
        notepad_id: Option<String>,

        // dict youdao --offset 10 | dict maimemo --upload --id 213423
        #[structopt(short, long)]
        upload: bool,

        /// 在upload时自动插入时间戳
        #[structopt(short)]
        timestamp: bool,
    },
}

struct MaimemoApp {
    notepads: Vec<Notepad>,
}

impl MaimemoApp {
    pub async fn from_web<'a>(maimemo: &mut MaimemoClient<'a>) -> Result<Self, String> {
        if !maimemo.has_logged() {
            debug!("Signing in");
            maimemo.login().await?;
        }
        let mut notepads = maimemo.get_notepad_list().await?;
        debug!("got notepad list: {:?}", notepads);
        for notepad in &mut notepads {
            let contents = maimemo
                .get_notepad_contents(notepad.get_notepad_id())
                .await?;
            debug!(
                "Got Notepad contents: {}, notepad id: {}",
                contents,
                notepad.get_notepad_id()
            );
            notepad.set_contents(Some(contents));
        }
        Ok(MaimemoApp { notepads })
    }

    pub async fn from_file(path: &str) -> Result<Self, String> {
        load_from_json_file(path)
            .await
            .map(|notepads| Self { notepads })
    }

    pub async fn upload_notepad<'a>(
        &self,
        maimemo_client: &MaimemoClient<'a>,
        id: &str,
    ) -> Result<(), String> {
        let captcha_contents = maimemo_client.refresh_captcha().await?;
        // Display captcha on the terminal
        let img = image::load_from_memory(&captcha_contents).map_err(|e| format!("{:?}", e))?;
        debug!("Exporting image content");
        print(&img, &Config::default()).expect("Image printing failed.");
        debug!("Waiting for input captcha");
        print!("please enter captcha: ");
        // read stdin
        let mut buf = Vec::new();
        let mut stdin = aio::stdin();
        stdin
            .read_to_end(&mut buf)
            .await
            .map_err(|e| format!("{:?}", e))?;
        let captcha_input = std::str::from_utf8(&buf).map_err(|e| format!("{:?}", e))?;
        debug!("Read to input captcha: {}", captcha_input);
        // save notepad
        let notepad = self
            .notepads
            .iter()
            .find(|notepad| notepad.get_notepad_id() == id)
            .ok_or(format!("not found notepad with id: {}", id))?;
        debug!("saving notepad: {}", notepad);
        maimemo_client.save_notepad(notepad, captcha_input).await?;
        debug!("save notepad successful");
        Ok(())
    }

    pub fn list(&self, id: Option<&str>) {
        if let Some(id) = id {
            self.notepads
                .iter()
                .find(|notepad| notepad.get_notepad_id() == id)
                .map(|notepad| println!("{}", notepad));
        } else {
            self.notepads
                .iter()
                .for_each(|notepad| println!("{}", notepad));
        }
    }
}

/// 从json文件中加载
async fn load_from_json_file<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let path = afs::canonicalize(path)
        .await
        .map_err(|e| format!("{:?}", e))?;
    debug!("Loading json from path: {}", path.to_str().unwrap());
    let file = afs::File::open(path)
        .await
        .map_err(|e| format!("{:?}", e))?
        .try_into_std()
        .unwrap();
    serde_json::from_reader(file).map_err(|e| format!("{:?}", e))
}

struct YoudaoApp {
    word_items: Vec<WordItem>,
}

impl YoudaoApp {
    pub async fn from_file(path: &str) -> Result<Self, String> {
        load_from_json_file(path)
            .await
            .map(|word_items| Self { word_items })
    }

    // pub async fn from_web()

    /// 从opt中构造一个可用的WordStore。如果不存在words file则自动加载
    ///
    /// 如果opt.words_file不存在，则默认为`$HOME/.youdao-words.json`。
    ///
    /// 如果opt.refetch==true，则从youdao下载并持久化到words file中。否则从words file加载
 /*   async fn load_word_items(opt: &Opt) -> Result<WordStore, reqwest::Error> {
        let path = opt.words_file.as_ref().map_or_else(
            || {
                let home_path = std::env::var("HOME")
                    .map(|s| s + "/.youdao-words.json")
                    .expect("not found path $HOME");
                Path::new(&home_path).to_path_buf()
            },
            |file| file.to_path_buf(),
        );
        if opt.refetch {
            // get words from youdao
            let username = "dhjnavyd@163.com";
            let password = "4ff32ab339c507639b234bf2a2919182";
            info!("loading words from youdao client");
            let mut youdao = YoudaoClient::new();
            youdao.login(username, password).await?;
            debug!(
                "login successful. username: {}, password: {}",
                username, password
            );
            // fetching
            let words = youdao.fetch_words().await?;
            let ws = WordStore::new(words);
            // persist to file
            debug!("persisting words to file: {}", path.to_str().unwrap());
            if let Err(e) = ws.persist(path) {
                panic!("refetch persisting error: {}", e);
            }
            Ok(ws)
        } else {
            info!("loading words from file: {}", path.to_str().unwrap());
            Ok(WordStore::from_file(path).expect("word store from file error"))
        }
    }*/
    pub fn list() {}

    /// 取出offset个元素。如果offset<0，则从后取出offset个。如果offset==0则不会过滤任何元素
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// let mut words = vec![1,2,3];
    /// let offset = -2;
    /// filter_offset(&mut words, offset); // words: [2,3]
    /// ```
    fn filter_offset<T>(words: &mut Vec<T>, offset: isize) {
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

    /// 通过时间区间`[start_date, end_date]`过虑单词并以升序排列。如果end_date is None则以当前时间为准，包含end_date,
    /// 如果start_date is None，则以1970-01-01开始。
    ///
    /// 格式：`"%Y-%m-%d %H:%M:%S`
    ///
    /// - [Remove an element from a vector](https://stackoverflow.com/a/40310140)
    fn filter_date<T>(words: &mut Vec<WordItem>, start_date: Option<&str>, end_date: Option<&str>) {
        if start_date.is_none() && end_date.is_none() {
            return;
        }
        let (suffix, format) = (" 00:00:00", "%Y-%m-%d %H:%M:%S");
        let end_date = end_date.map_or_else(
            || Utc::now(),
            |date| {
                Utc.datetime_from_str(&(date.to_string() + suffix), format)
                    .expect("parse end_date error")
            },
        );
        let start_date = start_date.map_or_else(
            || {
                Utc.datetime_from_str("1970-01-01 00:00:00", format)
                    .unwrap()
            },
            |date| {
                Utc.datetime_from_str(&(date.to_string() + suffix), format)
                    .expect("parse end_date error")
            },
        );
        if start_date > end_date {
            panic!("from date: {} > end date: {}", start_date, end_date);
        }
        info!("filter start_date: {}, end_date: {}", start_date, end_date);
        // remove方式：
        // 不能在foreach中删除remove 导致out of bound
        words.retain(|w| {
            let date = Utc.timestamp_millis(w.modified_time as i64);
            date >= start_date && date <= end_date
        });
        words.sort_unstable_by(|a, b| b.modified_time.cmp(&a.modified_time));
    }
}

use std::env;
#[tokio::main]
async fn main() -> Result<(), String> {
    // let opt: Opt = Opt::from_args();
    // init_log(opt.verbose);

    // let mut ws = get_word_store(&opt).await.map_err(|e| format!("{:?}", e))?;
    // let words = ws.get_mut_words();

    // filter_date(words, opt.start_date.as_deref(), opt.end_date.as_deref());
    // filter_offset(words, opt.offset);
    // print_with_mode(words, &opt.print_mode);

    let opt: AppOpt = AppOpt::from_args();
    // debug log
    if opt.verbose {
        pretty_env_logger::formatted_builder()
            // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
            .filter_level(log::LevelFilter::Debug)
            .init();
    }
    let home_dir = env::var("HOME").map_err(|e| format!("{:?}", e))?;
    let config_path = opt.config_path.as_ref().unwrap_or(&home_dir);
    let config = config::Config::from_yaml_file(config_path).map_err(|e| format!("{:?}", e))?;

    match opt.sub_cmd {
        Some(SubCommand::Youdao {
            list,
            refresh,
            end_date,
            start_date,
            offset,
        }) => {}
        _ => {}
    };

    Ok(())
}

fn print_with_mode(words: &Vec<WordItem>, mode: &str) {
    info!(
        "starting print with mode: {}, word count: {}",
        mode,
        words.len()
    );
    if mode == "min" {
        words.iter().for_each(|w| println!("{}", w.word));
    } else {
        panic!("unsupported mode: {}", mode);
    }
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
