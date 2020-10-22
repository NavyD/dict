use crate::config::*;
use cookie_store::CookieStore;
use reqwest::{self, Client};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self},
};

#[derive(Serialize, Deserialize, Debug)]
struct ResponseResult<T> {
    code: i32,
    msg: String,
    data: T,
}

#[derive(Serialize, Deserialize, Debug)]
struct Page<T> {
    total: usize,
    #[serde(rename = "itemList")]
    item_list: Vec<T>,
}
#[derive(Serialize, Deserialize, Debug, Ord, PartialOrd, Eq, PartialEq, Clone)]
pub struct WordItem {
    #[serde(rename = "itemId")]
    pub item_id: String,
    #[serde(rename = "bookId")]
    pub book_id: String,
    #[serde(rename = "bookName")]
    pub book_name: String,
    pub word: String,
    pub trans: String,
    pub phonetic: String,
    #[serde(rename = "modifiedTime")]
    pub modified_time: usize,
}

/// 通过`: `分离多行的k-v返回一个map。map只会返回成功分离的k-v。
///
/// # panic
///
/// 如果存在一行不能通过`: `分离为k-v形式，则panic
#[allow(dead_code)]
pub fn parse_headers(params: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    params
        .split('\n')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        // lines
        .collect::<Vec<&str>>()
        .iter()
        // line
        .map(|s| {
            let line = s.split(": ").collect::<Vec<&str>>();
            if line.len() != 2 {
                panic!("invalid header: {}", s);
            }
            // k, v
            (line[0], line[1])
        })
        .for_each(|(k, v)| {
            headers.insert(k.to_string(), v.to_string());
        });
    headers
}

pub struct YoudaoClient<'a> {
    client: Client,
    config: &'a AppConfig,
    cookie_store: CookieStore,
}
use crate::http_client::*;
impl<'a> YoudaoClient<'a> {
    /// 创建一个client
    ///
    /// # panic
    ///
    /// 如果Client无法创建
    pub fn new(config: &'a AppConfig) -> Result<Self, String> {
        Ok(Self {
            client: build_general_client()?,
            config,
            cookie_store: build_cookie_store(config.get_cookie_path())?,
        })
    }

    /// 使用username, password登录youdao. password必须是通过youdao网页端加密过的(hex_md5)，不能是明文密码
    pub async fn login(&mut self) -> Result<(), String> {
        self.prapre_login().await?;
        let req_name = "login";
        let savelogin = true;
        let form = [
            ("username", self.config.get_username()),
            ("password", self.config.get_password()),
            // 保存cookie
            ("savelogin", &(savelogin as i8).to_string()),
            // 由savelogin决定
            ("cf", &if savelogin { 7 } else { 3 }.to_string()),
            ("app", "web"),
            ("tp", "urstoken"),
            ("fr", "1"),
            (
                "ru",
                "http://dict.youdao.com/wordbook/wordlist?keyfrom=dict2.index#/",
            ),
            ("product", "DICT"),
            ("type", "1"),
            ("um", "true"),
            // 同意登录
            ("agreePrRule", "1"),
        ];
        debug!("sending request: {}", req_name);
        let resp = send_request(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            |url| url.to_string(),
            Some(&form),
        )
        .await?;
        debug!("login response: {:?}", resp);
        // 多次登录后可能引起无法登录的问题
        if resp
            .headers()
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case("set-cookie"))
            .is_none()
        {
            let error = format!("not found set-cookie in login resp: {:?}", resp);
            let body = resp.text().await.map_err(|e| format!("{:?}", e))?;
            debug!("{}, body: {}", error, body);
            Err("Frequent login may have been added to youdao blacklist, not found any set-cookie in login resp".to_string())
        } else if !self.has_logged() {
            let error = format!("Unable to find login related cookie. resp: {:?}", resp);
            debug!(
                "{}, cookie store: {:?}, body: {:?}",
                error,
                self.cookie_store,
                resp.text().await.map_err(|e| format!("{:?}", e))?
            );
            Err("login failed. not found login cookies".to_string())
        } else {
            Ok(())
        }
    }

    /// 缓存从youdao获取完整的单词本并清空之前存在的单词
    ///
    /// # panic
    ///
    /// 如果用户未登录
    pub async fn get_words(&mut self) -> Result<Vec<WordItem>, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let mut words = vec![];
        let req_name = "get-words";
        debug!("sending request: {}", req_name);
        let (limit, mut offset) = (1000, 0);
        loop {
            // let querys = ;
            let resp = send_request_nobody(
                &self.config,
                &self.client,
                &self.cookie_store,
                req_name,
                |url| {
                    url.to_string()
                        + "?limit="
                        + &limit.to_string()
                        + "&offset="
                        + &offset.to_string()
                },
            )
            .await?;
            let result = resp.json::<ResponseResult<Page<WordItem>>>().await.map_err(|e| format!("{:?}", e))?;
            let items = result.data.item_list;
            let len = items.len();
            items.into_iter().for_each(|item| words.push(item));
            if len < limit {
                return Ok(words)
            }
            offset += 1;
        }
    }

    pub fn has_logged(&self) -> bool {
        let domain = ".youdao.com";
        self.cookie_store
            .get(domain, "/", "OUTFOX_SEARCH_USER_ID")
            .is_some()
            && self.cookie_store.get(domain, "/", "DICT_PERS").is_some()
            && self.cookie_store.get(domain, "/", "DICT_SESS").is_some()
    }

    /// 获取youdao set-cookie: outfox_search_user_id，保证后续登录有效
    async fn prapre_login(&mut self) -> Result<(), String> {
        let req_name = "fetch-cookie-outfox-search-user-id";
        debug!("sending request with req name: {}", req_name);
        let resp = send_request_nobody(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            |url| url.to_string(),
        )
        .await?;
        update_set_cookies(&mut self.cookie_store, &resp);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONFIG_PATH: &'static str = "config.yml";

    #[tokio::test]
    async fn login_test() -> Result<(), String>{
        init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).map_err(|e| format!("{:?}", e))?;
        let mut client = YoudaoClient::new(&config.get_youdao())?;
        if !client.has_logged() {
            client.login().await?;
        }
        Ok(())
    }

    fn init_log() {
        pretty_env_logger::formatted_builder()
        // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
        .filter_level(log::LevelFilter::Debug)
        .init();
    }
}