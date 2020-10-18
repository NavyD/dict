use crate::config::*;
use chrono::Local;
use cookie_store::CookieStore;
use reqwest::{header::*, Client, Method, RequestBuilder};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::BufReader;
use std::path::PathBuf;

/// notepad包含必要的header info和内容detail
#[derive(Debug, Serialize, Deserialize)]
pub struct Notepad {
    is_private: u8,
    notepad_id: String,
    title: String,
    brief: String,
    created_time: Option<String>,
    updated_time: Option<String>,
    contents: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseResult {
    error: String,
    valid: i32,
    total: usize,
    notepad: Option<Vec<Notepad>>,
}

/// maimemo提供一些访问操作。
pub struct MaimemoClient<'a> {
    client: Client,
    config: &'a Maimemo,
    cookie_store: CookieStore,
    user_token_name: String,
}

impl<'a> std::ops::Drop for MaimemoClient<'a> {
    /// 在退出时保存cookie store
    fn drop(&mut self) {
        self.config
            .get_cookie_path()
            .and_then(
                |path| match OpenOptions::new().create(true).write(true).open(path) {
                    Ok(file) => {
                        info!("Saving cookies to path {}", path);
                        Some(file)
                    }
                    Err(e) => {
                        info!("Cookie store persistence failed open. {}", e);
                        None
                    }
                },
            )
            .map(|mut file| {
                if let Err(e) = self.cookie_store.save_json(&mut file) {
                    warn!("Cookie store persistence failed save. {:?}", e);
                }
            });
    }
}

impl<'a> MaimemoClient<'a> {
    /// 用config构造一个client。如果config.cookie_path存在则加载，否则使用in memory的cookie store。
    pub fn new(config: &'a Maimemo) -> Self {
        let client = Client::builder()
            .cookie_store(false)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build client error");
        let cookie_store = config.get_cookie_path().map_or_else(
            || {
                debug!("not found cookie store path. cookie store used in memory");
                CookieStore::default()
            },
            |path| {
                File::open(path).map_or_else(
                    |e| {
                        info!(
                            "cookie store used in memory. load cookie store path error: {:?}.",
                            e
                        );
                        CookieStore::default()
                    },
                    |file| {
                        debug!("loading cookie store from path: {}", path);
                        match CookieStore::load_json(BufReader::new(file)) {
                            Ok(cs) => {
                                cs.iter_any().for_each(|c| {
                                    debug!("loaded cookie: [{}={}]", c.name(), c.value())
                                });
                                cs
                            }
                            Err(e) => panic!("load cookie store error: {:?}", e),
                        }
                    },
                )
            },
        );
        Self {
            client,
            config,
            cookie_store,
            user_token_name: "userToken".to_string(),
        }
    }

    pub fn get_user_token_val(&self) -> Option<&str> {
        self.cookie_store
            .get("www.maimemo.com", "/", &self.user_token_name)
            .map(|c| c.value())
    }

    pub fn has_logged(&self) -> bool {
        self.get_user_token_val().is_some()
    }

    /// 登录并更新config.cookies
    pub async fn login(&mut self) -> Result<(), String> {
        let req_name = "login";

        let form = [
            ("email", self.config.get_username()),
            ("password", self.config.get_password()),
        ];
        let resp = self
            .send_request(req_name, |url| url.to_string(), Some(&form))
            .await?;
        // login failed
        if resp // find user token
            .cookies()
            .find(|c| c.name() == self.user_token_name)
            .is_none()
        {
            debug!("login error. userToken not found in resp: {:?}", resp);
            return Err("login error. userToken not found".to_string());
        }
        // Check if the user token exists
        update_set_cookies(&mut self.cookie_store, &resp);
        if !self.has_logged() {
            error!(
                "update cookie store failed. not found cookie: [{}] in cookie_store",
                self.user_token_name
            );
            return Err("login failed. not found cookie store".to_string());
        }
        Ok(())
    }

    /// 获取notepad list
    pub async fn get_notepad_list(&mut self) -> Result<Vec<Notepad>, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "notepad-search";
        // ?token={user_token}
        let url_handler = |url: &str| {
            let user_token = self.get_user_token_val().expect("not found user token");
            url.to_string() + user_token
        };
        let payload = serde_json::json!({"keyword":null,"scope":"MINE","recommend":false,"offset":0,"limit":30,"total":-1});
        let resp = self
            .send_request(req_name, url_handler, Some(&payload))
            .await?;
        // update_set_cookies(&mut self.cookie_store, &resp, &url);
        let result = resp
            .json::<ResponseResult>()
            .await
            .map_err(|e| format!("{:?}", e))?;
        if let Some(notepad) = result.notepad {
            Ok(notepad)
        } else {
            debug!("get notepad failed: {:?}", result);
            Err("get notepad failed".to_string())
        }
    }

    /// 获取notepad中单词文本
    pub async fn get_notepad_contents(&self, notepad_id: &str) -> Result<String, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "notepad-detail";
        let url_handler = |url: &str| url.to_string() + notepad_id;
        let resp = self.send_request_nobody(req_name, url_handler).await?;
        debug!("parsing notepad html");
        parse_notepad_text(&resp.text().await.map_err(|e| format!("{:?}", e))?)
    }

    /// 刷新下载notepad对应的captcha返回文件全路径。
    pub async fn refresh_captcha(&self) -> Result<PathBuf, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "service-captcha";
        let url_handler = |url: &str| url.to_owned() + &Local::now().timestamp_nanos().to_string();
        let resp = self
            .send_request_nobody(req_name, url_handler)
            .await
            .map_err(|e| format!("{:?}", e))?;
        let contents = resp
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|e| format!("{:?}", e))?;

        let path =
            fs::canonicalize(self.config.get_captcha_path()).map_err(|e| format!("{:?}", e))?;
        debug!(
            "writing the content of captcha to the file: {}",
            path.to_str().unwrap()
        );
        fs::write(path.to_owned(), contents).map_err(|e| format!("{:?}", e))?;
        Ok(path)
    }

    /// 保存notepad
    ///
    /// 注意：maimemo要求先获取验证码，再保存。并且要求是同一机器发送的。在win host浏览器刷新验证码，
    /// 但在wsl2 保存则不会生效，很可能是对比的发送的数据包是否来自同一机器
    pub async fn save_notepad(&self, notepad: Notepad, captcha: &str) -> Result<(), String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "notepad-save";
        if notepad.contents.is_none() {
            return Err("notepad contents is none".to_string());
        }
        // form
        let contents = notepad.contents.unwrap();
        let mut form = std::collections::HashMap::new();
        form.insert("id".to_string(), notepad.notepad_id.to_string());
        form.insert("title".to_string(), notepad.title.to_string());
        form.insert("brief".to_string(), notepad.brief.to_string());
        form.insert("content".to_string(), contents);
        form.insert(
            "is_private".to_string(),
            (notepad.is_private == 1).to_string(),
        );
        form.insert("captcha".to_string(), captcha.to_string());
        let form = form
            .iter()
            .map(|(key, val)| (key.as_str(), val.as_str()))
            .collect::<Vec<_>>();

        #[derive(Debug, Serialize, Deserialize)]
        struct RespResult {
            valid: i8,
            #[serde(rename = "errorCode")]
            error: Option<String>,
        }
        let result: RespResult = self
            .send_request(req_name, |url| url.to_string(), Some(&form))
            .await?
            .json::<RespResult>()
            .await
            .map_err(|e| format!("{:?}", e))?;
        if let Some(e) = &result.error {
            error!("save notepad failed: {:?}", result);
            return Err(format!("save notepad failed: {}", e));
        }
        debug!("save_notepad successful");
        Ok(())
    }

    async fn send_request_nobody<U: FnOnce(&'a str) -> String>(
        &'a self,
        req_name: &str,
        url_handler: U,
    ) -> Result<reqwest::Response, String> {
        self.send_request(req_name, url_handler, None::<&str>).await
    }

    /// 获取req_name对应的config发送一个request，并返回200成功的resp。
    /// `url_handler`可以处理url。
    ///
    /// 从config中读取url,method,headers与self.cookie_store中的cookie构造request
    ///
    /// 如果body不为空，则通过header content-type处理，当前支持：
    ///
    /// - json
    /// - form
    ///
    /// 如果response.status!=200则返回error
    ///
    async fn send_request<T: Serialize + ?Sized, U: FnOnce(&'a str) -> String>(
        &'a self,
        req_name: &str,
        url_handler: U,
        body: Option<&T>,
    ) -> Result<reqwest::Response, String> {
        let req_config = get_request_config(&self.config, req_name)
            .ok_or(format!("not found req config with req_name: {}", req_name))?;
        debug!("Found configuration for request: {}", req_name);

        let url = req_config.get_url();
        debug!("found the configured url: {}", url);
        let url = url_handler(url);
        debug!("new url: [{}] processed by url_handler", url);

        let method = Method::from_bytes(req_config.get_method().as_bytes())
            .map_err(|e| format!("{:?}", e))?;
        debug!("found the configured method: {}", method);

        let mut req_builder = self.client.request(method, &url);

        let headers = req_config
            .get_headers()
            .ok_or(format!("not found any headers in req url: {}", url))?;
        debug!("Fill in the request from the configured headers");
        req_builder = fill_headers(req_builder, headers)?;

        debug!("filling reqeust cookies");
        req_builder = fill_request_cookies(&self.cookie_store, req_builder, &url);

        if let Some(body) = body {
            let content_type = req_config
                .get_headers()
                .and_then(|headers| {
                    headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                        .map(|(_, v)| v)
                })
                .ok_or_else(|| {
                    debug!(
                        "not found content-type in request headers: {:?}",
                        req_config.get_headers()
                    );
                    format!("not found content-type in request")
                })?;
            req_builder = fill_body(req_builder, content_type, body)?;
            // req_builder = req_builder.form(body);
        }

        debug!("sending request: {:?}", req_builder);
        let resp = req_builder.send().await.map_err(|e| format!("{:?}", e))?;
        debug!("response received: {:?}", resp);
        let status = resp.status();
        if status.as_u16() != 200 {
            let msg = format!("Response code error: {}", status);
            debug!("{}", msg);
            return Err(msg);
        }
        Ok(resp)
    }
}

/// 从response html body中取出单词文本
fn parse_notepad_text(html: &str) -> Result<String, String> {
    let id = "#content";
    let id_selector = Selector::parse(id).map_err(|e| format!("{:?}", e))?;
    let document = Html::parse_document(html);
    document
        .select(&id_selector)
        .next()
        .map(|e| e.inner_html())
        .ok_or_else(|| {
            debug!("not found element {} in html: \n{}", id, html);
            format!("not found element {} in html", id)
        })
}

/// 通过content-type解析body到request builder中
fn fill_body<T: Serialize + ?Sized>(
    req_builder: RequestBuilder,
    content_type: &str,
    body: &T,
) -> Result<RequestBuilder, String> {
    // parse body with content-type
    debug!("Parsing body of content type: {}", content_type);
    if content_type.contains("application/x-www-form-urlencoded") {
        serde_urlencoded::to_string(body)
            .map(|body| {
                debug!("Url encoded the body: {}", body);
                req_builder.body(body)
            })
            .map_err(|e| format!("{:?}", e))
    } else if content_type.contains("application/json") {
        // req_builder.body(body.into())
        serde_json::to_vec(body)
            .map(|body| {
                debug!("jsoned body: {:?}", body);
                req_builder.body(body)
            })
            .map_err(|e| format!("{:?}", e))
    } else {
        todo!("unsupported content type: {:?}", content_type)
    }
}

pub fn fill_request_cookies(
    cookie_store: &cookie_store::CookieStore,
    req_builder: RequestBuilder,
    req_url: &str,
) -> RequestBuilder {
    let url = &reqwest::Url::parse(req_url).unwrap();
    let delimiter = "; ";
    let mut cookies = "".to_string();
    for c in cookie_store.get_request_cookies(url) {
        cookies = cookies + c.name() + "=" + c.value() + delimiter;
    }
    let start = cookies.len() - delimiter.len();
    cookies.drain(start..cookies.len());
    debug!("found reqeust cookie str: {}", cookies);
    match HeaderValue::from_str(&cookies) {
        Ok(v) => req_builder.header(reqwest::header::COOKIE, v),
        Err(e) => {
            debug!("unable to request cookie: {}. error: {:?}", cookies, e);
            req_builder
        }
    }
}

/// 从response中获取`set-cookie`s更新到cookie_store中。如果出现cookie无法解析或store无法插入则跳过
pub fn update_set_cookies(cookie_store: &mut cookie_store::CookieStore, resp: &reqwest::Response) {
    let set_cookies = resp
        .headers()
        .iter()
        .filter(|(name, _)| *name == reqwest::header::SET_COOKIE)
        .map(|(_, v)| v.to_str().unwrap())
        .collect::<Vec<_>>();
    debug!("Updating response cookies to cookie_store");
    for cookie_str in set_cookies {
        debug!("inserting set-cookie: {}", cookie_str);
        if let Err(e) = cookie::Cookie::parse(cookie_str).map(|raw_cookie| {
            if let Err(e) = cookie_store.insert_raw(&raw_cookie, resp.url()) {
                debug!("unable to store Set-Cookie: {:?}", e);
            }
        }) {
            debug!("parse Set-Cookie val error {:?}", e);
        }
    }
}

/// 将headers内容填充至req_builder中
///
/// 如果header中存在不合法的key,val返回一个str error
pub fn fill_headers(
    req_builder: RequestBuilder,
    headers: &HashMap<String, String>,
) -> Result<RequestBuilder, String> {
    let mut req_headers = HeaderMap::new();
    for (key, val) in headers {
        trace!("filling request header: {}={}", key, val);
        let name = HeaderName::from_lowercase(key.as_bytes()).map_err(|e| format!("{:?}", e))?;
        let val = HeaderValue::from_str(val).map_err(|e| format!("{:?}", e))?;
        if let Some(old) = req_headers.insert(name, val) {
            debug!("replace old header: {}={}", key, old.to_str().unwrap());
        }
    }
    Ok(req_builder.headers(req_headers))
}

/// 通过req_name从Config中获取一个request config
pub fn get_request_config<'a>(config: &'a Maimemo, req_name: &str) -> Option<&'a RequestConfig> {
    config.get_requests().and_then(|reqs| {
        reqs.iter()
            .find(|(name, _)| *name == req_name)
            .map(|(_, v)| v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONFIG_PATH: &str = "config.yml";

    #[tokio::test]
    async fn try_login() -> Result<(), String> {
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.get_maimemo());
        client.login().await.map_err(|e| format!("{:?}", e))?;
        Ok(())
    }

    #[tokio::test]
    async fn get_notepad_list() -> Result<(), String> {
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.get_maimemo());
        if !client.has_logged() {
            client.login().await?;
        }
        let notepads = client.get_notepad_list().await?;
        assert!(notepads.len() > 0);
        Ok(())
    }

    #[tokio::test]
    async fn get_notepad_contents() -> Result<(), String> {
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.get_maimemo());
        if !client.has_logged() {
            client.login().await?;
        }
        let notepads = client.get_notepad_list().await?;
        for notepad in notepads {
            let contents = client.get_notepad_contents(&notepad.notepad_id).await?;
            assert!(contents.len() > 0);
            assert!(contents.contains("\n"));
        }
        Ok(())
    }

    #[tokio::test]
    async fn refresh_captcha() -> Result<(), String> {
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.get_maimemo());
        if !client.has_logged() {
            client.login().await?;
        }
        let old_file = std::fs::read(config.get_maimemo().get_captcha_path());
        let path = client.refresh_captcha().await?;
        assert!(path.is_file());
        let new_file = std::fs::read(path).map_err(|e| format!("{:?}", e))?;
        if let Ok(old_file) = old_file {
            assert_ne!(old_file, new_file);
        }
        Ok(())
    }

    /*
        // passed
        #[tokio::test]
        async fn save_notepad() -> Result<(), String> {
            fn init_log(verbose: bool) {
                pretty_env_logger::formatted_builder()
                    // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
                    .filter_level(if verbose {
                        log::LevelFilter::Debug
                    } else {
                        log::LevelFilter::Warn
                    })
                    .init();
            }
            init_log(true);

            let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
            let mut client = MaimemoClient::new(config.get_maimemo());
            // if !client.has_logged() {
            //     client.login().await?;
            // }
            let contents = r#"
    #2020-10-18
    new
    words
    test
    rust
    "#;
            let notepad = Notepad {
                title: "test".to_string(),
                brief: "words".to_string(),
                is_private: 1,
                notepad_id: "695835".to_string(),
                contents: Some(contents.to_string()),
                created_time: None,
                updated_time: None,
            };
            client.save_notepad(notepad, "cdw24").await?;
            Ok(())
        }
        */
}
