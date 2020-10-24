use crate::config::*;
use crate::client::*;
use cookie_store::CookieStore;
use reqwest::{self, Client};
use serde::{Deserialize, Serialize};

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

pub struct YoudaoClient {
    client: Client,
    config: AppConfig,
    cookie_store: CookieStore,
}

impl std::ops::Drop for YoudaoClient {
    /// 在退出时保存cookie store
    fn drop(&mut self) {
        if let Some(path) = self.config.get_cookie_path() {
            if let Err(e) = save_cookie_store(path, &self.cookie_store) {
                error!("save cookie store failed: {}", e);
            }
        }
    }
}

impl YoudaoClient {
    /// 创建一个client
    ///
    /// # panic
    ///
    /// 如果Client无法创建
    pub fn new(config: AppConfig) -> Result<Self, String> {
        let cookie_store = build_cookie_store(config.get_cookie_path())?;
        Ok(Self {
            client: build_general_client()?,
            config,
            cookie_store,
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
        let resp = send_request(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            |url| url.to_string(),
            Some(&form),
        )
        .await?;
        update_set_cookies(&mut self.cookie_store, &resp);
        // 多次登录后可能引起无法登录的问题
        if resp
            .headers()
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case("set-cookie"))
            .is_none()
        {
            let error = format!("not found set-cookie in login resp: {:?}", resp);
            let body = resp.text().await.map_err(|e| format!("{:?}", e))?;
            error!("{}, body: {}", error, body);
            Err("Frequent login may have been added to youdao blacklist, not found any set-cookie in login resp".to_string())
        } else if !self.has_logged() {
            let error = format!("Unable to find login related cookie. resp: {:?}", resp);
            error!(
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

    /// 获取单词数量
    pub async fn get_words_total(&self) -> Result<usize, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "get-words";
        let (limit, offset) = (1, 0);
        let resp = send_request_nobody(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            |url| format!("{}?limit={}&offset={}", url, limit, offset),
        )
        .await?;
        let result = resp
            .json::<ResponseResult<Page<WordItem>>>()
            .await
            .map_err(|e| format!("{:?}", e))?;
        Ok(result.data.total)
    }

    /// 从youdao获取完整的单词本
    ///
    /// # panic
    ///
    /// 如果用户未登录
    pub async fn get_words(&mut self) -> Result<Vec<WordItem>, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        debug!("getting words total");
        let total = self.get_words_total().await?;
        debug!("got words total: {}", total);
        let mut words = vec![];
        let req_name = "get-words";
        let limit = 1000;
        let numbers = (total as f64 / limit as f64).ceil() as usize;
        for number in 0..numbers {
            let offset = limit * number;
            // let querys = ;
            debug!("Getting words with limit: {}, offset: {}", limit, offset);
            let resp = send_request_nobody(
                &self.config,
                &self.client,
                &self.cookie_store,
                req_name,
                |url| format!("{}?limit={}&offset={}", url, limit, offset),
            )
            .await?;
            let result = resp
                .json::<ResponseResult<Page<WordItem>>>()
                .await
                .map_err(|e| format!("{:?}", e))?;
            let items = result.data.item_list;
            debug!(
                "got youdao page words. code: {}, msg: {}, item size: {}",
                result.code,
                result.msg,
                items.len()
            );
            items.into_iter().for_each(|item| words.push(item));
        }
        debug!("got all words size: {}", words.len());
        if words.len() == total {
            Ok(words)
        } else {
            Err(format!("The number of words obtained is not the same as the total number! len: {}, total: {}", words.len(), total))
        }
    }

    /// 从cookie_store中查询是否存在登录的cookie
    pub fn has_logged(&self) -> bool {
        let domain = "youdao.com";
        self.cookie_store
            .get(domain, "/", "OUTFOX_SEARCH_USER_ID")
            .is_some()
            && self.cookie_store.get(domain, "/", "DICT_PERS").is_some()
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
    async fn login_test() -> Result<(), String> {
        // init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).map_err(|e| format!("{:?}", e))?;
        let mut client = YoudaoClient::new(config.youdao.unwrap())?;
        if !client.has_logged() {
            client.login().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn get_words_test() -> Result<(), String> {
        // init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).map_err(|e| format!("{:?}", e))?;
        let mut client = YoudaoClient::new(config.youdao.unwrap())?;
        if !client.has_logged() {
            client.login().await?;
        }
        let words = client.get_words().await?;
        assert!(words.len() > 0);
        Ok(())
    }

    #[allow(dead_code)]
    fn init_log() {
        pretty_env_logger::formatted_builder()
            // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
            .filter_module("youdao_dict_export::youdao_client", log::LevelFilter::Trace)
            .init();
    }
}
