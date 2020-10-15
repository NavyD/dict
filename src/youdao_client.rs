use reqwest::{self, Client};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

pub struct YoudaoClient {
    client: Client,
    is_loggedin: bool,
}

impl YoudaoClient {
    /// 创建一个client
    ///
    /// # panic
    ///
    /// 如果Client无法创建
    pub fn new() -> Self {
        let client = Client::builder()
            .cookie_store(true)
            .redirect( reqwest::redirect::Policy::none())
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            .build()
            .expect("build client error");
        Self {
            client,
            is_loggedin: false,
        }
    }
    /// 使用username, password登录youdao. password必须是通过youdao网页端加密过的(hex_md5)，不能是明文密码
    pub async fn login(&mut self, username: &str, password: &str) -> Result<(), reqwest::Error> {
        let outfox_search_user_id = self.get_cookie_outfox_search_user_id().await?;
        debug!(
            "Have obtained cookie: outfox_search_user_id={}",
            outfox_search_user_id
        );
        let url = "https://logindict.youdao.com/login/acc/login";
        // Content-Length: 237
        let savelogin = true;
        let resp = self.client.post(url)
            .header("Host", "logindict.youdao.com")
            .header("Connection", "keep-alive")
            .header("Cache-Control", "max-age=0")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Origin", "http://account.youdao.com")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
            .header("Sec-Fetch-Site", "cross-site")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-User", "?1")
            .header("Content-Length", "225")
            .header("Sec-Fetch-Dest", "document")
            .header("Referer", "http://account.youdao.com/")
            .header("Accept-Encoding", "gzip, deflate, br")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .form(&[
                ("username", username),
                ("password", password),
                // 保存cookie
                ("savelogin", &(savelogin as i8).to_string()),
                // 由savelogin决定
                ("cf", &if savelogin {7} else {3}.to_string()),
                ("app", "web"),
                ("tp", "urstoken"),
                ("fr", "1"),
                ("ru", "http://dict.youdao.com/wordbook/wordlist?keyfrom=dict2.index#/"),
                ("product", "DICT"),
                ("type", "1"),
                ("um", "true"),
                // 同意登录
                ("agreePrRule", "1"),
            ]).send()
            .await?;
        debug!("login response: {:?}", resp);
        // 多次登录后可能引起无法登录的问题
        if resp
            .headers()
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case("set-cookie"))
            .is_none()
        {
            panic!("Frequent login may have been added to youdao blacklist, not found any set-cookie in login resp");
        }
        if resp.status().as_u16() != 302 {
            panic!("Login response code is not 302 error");
        }
        self.is_loggedin = true;
        Ok(())
    }

    /// 缓存从youdao获取完整的单词本并清空之前存在的单词
    ///
    /// # panic
    ///
    /// 如果用户未登录
    pub async fn fetch_words(&mut self) -> Result<Vec<WordItem>, reqwest::Error> {
        if !self.is_loggedin {
            panic!("Operation not allowed without login");
        }
        let total = self.get_page_words(15, 0).await?.data.total;
        let page_size = 1000;
        let page_numbers = (total as f64 / page_size as f64).ceil() as usize;
        info!(
            "Found available page_numbers: {}, page_size={}, total={}",
            page_numbers, page_size, total
        );
        let mut words = vec![];
        for num in 0..page_numbers {
            debug!("fetching page: {}", num);
            self.get_page_words(page_size, num)
                .await?
                .data
                .item_list
                .into_iter()
                .for_each(|item| words.push(item));
            debug!("page: {} Push the item completed", num);
        }
        Ok(words)
    }


    async fn get_page_words(
        &self,
        page_size: usize,
        page_number: usize,
    ) -> Result<ResponseResult<Page<WordItem>>, reqwest::Error> {
        let url = format!(
            "http://dict.youdao.com/wordbook/webapi/words?limit={}&offset={}",
            page_size,
            page_number * page_size
        );
        self.client.get(&url)
            .header("Host", "dict.youdao.com")
            .header("Connection", "keep-alive")
            .header("Pragma", "no-cache")
            .header("Cache-Control", "no-cache")
            .header("Accept", "application/json, text/plain, */*")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            .header("Referer", "http://dict.youdao.com/wordbook/wordlist?keyfrom=dict2.index")
            .header("Accept-Encoding", "gzip, deflate")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await?
            .json::<ResponseResult<Page<WordItem>>>()
            .await
    }

    /// 获取youdao set-cookie: outfox_search_user_id，保证后续登录有效
    async fn get_cookie_outfox_search_user_id<'a>(&self) -> Result<String, reqwest::Error> {
        let url = "http://account.youdao.com/login?service=dict&back_url=http%3A%2F%2Fdict.youdao.com%2Fwordbook%2Fwordlist%3Fkeyfrom%3Ddict2.index%23%2F";
        let name = "OUTFOX_SEARCH_USER_ID";
        self.client.get(url)
            .header("Host", "account.youdao.com")
            .header("Connection", "keep-alive")
            .header("Upgrade-Insecure-Requests", "1")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
            .header("Accept-Encoding", "gzip, deflate")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await?
            .cookies()
            .find(|c| c.name() == name)
            .map(|c| c.value().to_string())
            .ok_or_else(|| panic!("not found set cookie: {} in headers", name))
    }
}
