use crate::client::*;
use crate::config::*;
use chrono::Local;
use cookie_store::CookieStore;
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fmt;

/// notepad包含必要的header info和内容detail
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Notepad {
    is_private: u8,
    notepad_id: String,
    title: String,
    brief: String,
    created_time: Option<String>,
    updated_time: Option<String>,
    contents: Option<String>,
}

impl Notepad {
    pub fn get_notepad_id(&self) -> &str {
        &self.notepad_id
    }

    pub fn set_contents(&mut self, contents: Option<String>) {
        self.contents = contents;
    }

    pub fn get_contents(&self) -> Option<&str> {
        self.contents.as_deref()
    }

    pub fn get_contents_mut(&mut self) -> Option<&mut String> {
        self.contents.as_mut()
    }
}

impl fmt::Display for Notepad {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut temp = self.clone();
        // 仅输出第一行 与 total length
        let contents = temp.contents.as_mut().unwrap();
        let total_len = contents.len();
        contents.drain(contents.find("\n").unwrap_or(total_len)..);
        contents.push_str("... total length: ");
        contents.push_str(&total_len.to_string());
        write!(f, "{}", serde_json::to_string_pretty(&temp).unwrap())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseResult {
    error: String,
    valid: i32,
    total: usize,
    notepad: Option<Vec<Notepad>>,
}

/// maimemo提供一些访问操作。
pub struct MaimemoClient {
    client: Client,
    config: AppConfig,
    cookie_store: CookieStore,
    user_token_name: String,
}

impl std::ops::Drop for MaimemoClient {
    /// 在退出时保存cookie store
    fn drop(&mut self) {
        if let Some(path) = self.config.get_cookie_path() {
            if let Err(e) = save_cookie_store(path, &self.cookie_store) {
                error!("save cookie store failed: {}", e);
            }
        }
    }
}

impl MaimemoClient {
    /// 用config构造一个client。如果config.cookie_path存在则加载，否则使用in memory的cookie store。
    pub fn new(config: AppConfig) -> Result<Self, String> {
        let cookie_store = build_cookie_store(config.get_cookie_path())?;
        Ok(Self {
            client: build_general_client()?,
            config,
            cookie_store: cookie_store,
            user_token_name: "userToken".to_string(),
        })
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
        let resp = send_request(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            |url| url.to_string(),
            Some(&form),
        )
        .await?;
        // login failed
        // Check if the user token exists
        update_set_cookies(&mut self.cookie_store, &resp);
        if !self.has_logged() {
            error!(
                "update cookie store failed. not found cookie: [{}] in cookie_store",
                self.user_token_name
            );
            Err("login failed. not found cookie store".to_string())
        } else {
            debug!("login successful");
            Ok(())
        }
    }

    /// 提供完整的notepad list调用get_notepad_list与get_notepad_contents
    pub async fn get_notepads(&mut self) -> Result<Vec<Notepad>, String> {
        let mut notepads = self.get_notepad_list().await?;
        for notepad in &mut notepads {
            let contents = self.get_notepad_contents(notepad.get_notepad_id()).await?;
            notepad.set_contents(Some(contents));
        }
        Ok(notepads)
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
        let resp = send_request(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            url_handler,
            Some(&payload),
        )
        .await?;
        let result = resp
            .json::<ResponseResult>()
            .await
            .map_err(|e| format!("{:?}", e))?;
        if let Some(notepad) = result.notepad {
            debug!("got notepad list. len: {}", notepad.len());
            Ok(notepad)
        } else {
            error!("get notepad failed: {:?}", result);
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
        let resp = send_request_nobody(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            url_handler,
        )
        .await?;
        Self::parse_notepad_text(&resp.text().await.map_err(|e| format!("{:?}", e))?)
    }

    /// 刷新下载notepad对应的captcha返回文件全路径。
    pub async fn refresh_captcha(&self) -> Result<Vec<u8>, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "service-captcha";
        let url_handler = |url: &str| url.to_owned() + &Local::now().timestamp_nanos().to_string();
        let resp = send_request_nobody(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            url_handler,
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        let contents = resp
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|e| format!("{:?}", e))?;
        Ok(contents)
    }

    /// 保存notepad
    ///
    /// 注意：maimemo要求先获取验证码，再保存。并且要求是同一机器发送的。在win host浏览器刷新验证码，
    /// 但在wsl2 保存则不会生效，很可能是对比的发送的数据包是否来自同一机器
    pub async fn save_notepad(&self, notepad: Notepad, captcha: String) -> Result<(), String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "notepad-save";
        if notepad.contents.is_none() {
            return Err("notepad contents is none".to_string());
        }
        // form
        let mut form = std::collections::HashMap::new();
        form.insert("id".to_string(), notepad.notepad_id);
        form.insert("title".to_string(), notepad.title);
        form.insert("brief".to_string(), notepad.brief);
        form.insert("content".to_string(), notepad.contents.unwrap());
        form.insert(
            "is_private".to_string(),
            (notepad.is_private == 1).to_string(),
        );
        form.insert("captcha".to_string(), captcha);
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
        let result: RespResult = send_request(
            &self.config,
            &self.client,
            &self.cookie_store,
            req_name,
            |url| url.to_string(),
            Some(&form),
        )
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

    /// 从response html body中取出单词文本
    fn parse_notepad_text(html: &str) -> Result<String, String> {
        if html.is_empty() {
            return Err("html is empty".to_string());
        }
        debug!("parsing notepad html");
        let id = "#content";
        let id_selector = Selector::parse(id).map_err(|e| format!("{:?}", e))?;
        let document = Html::parse_document(html);
        document
            .select(&id_selector)
            .next()
            .map(|e| e.inner_html())
            .ok_or_else(|| {
                error!("not found element {} in html: \n{}", id, html);
                format!("not found element {} in html", id)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONFIG_PATH: &str = "config.yml";
    #[tokio::test]
    async fn try_login() -> Result<(), String> {
        init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.maimemo.unwrap())?;
        client.login().await.map_err(|e| format!("{:?}", e))?;
        Ok(())
    }

    #[tokio::test]
    async fn get_notepad_list() -> Result<(), String> {
        init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.maimemo.unwrap())?;
        if !client.has_logged() {
            client.login().await?;
        }
        let notepads = client.get_notepad_list().await?;
        assert!(notepads.len() > 0);
        Ok(())
    }

    #[tokio::test]
    async fn get_notepad_contents() -> Result<(), String> {
        init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.maimemo.unwrap())?;
        if !client.has_logged() {
            client.login().await?;
        }
        let notepads = client.get_notepad_list().await?;
        // for notepad in notepads {
            let contents = client.get_notepad_contents(&notepads[0].notepad_id).await?;
            assert!(contents.len() > 0);
            assert!(contents.contains("\n"));
        // }
        Ok(())
    }

    #[allow(dead_code)]
    fn init_log() {
        pretty_env_logger::formatted_builder()
            // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
            .filter_module("dict", log::LevelFilter::Trace)
            .init();
    }

    #[tokio::test]
    async fn refresh_captcha() -> Result<(), String> {
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.maimemo.unwrap())?;
        if !client.has_logged() {
            client.login().await?;
        }
        let data = client.refresh_captcha().await?;
        assert!(data.len() > 0);
        // assert!(path.is_file());
        // let new_file = std::fs::read(path).map_err(|e| format!("{:?}", e))?;
        // if let Ok(old_file) = old_file {
        //     assert_ne!(old_file, new_file);
        // }
        Ok(())
    }
}
