use crate::client::*;
use crate::config::*;
use chrono::Local;
use cookie_store::CookieStore;
use reqwest::{header::*, Client, Method, RequestBuilder};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::{
    fs::{self, OpenOptions},
    io::{self},
};

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
        let temp = Notepad {
            is_private: self.is_private,
            brief: self.brief.to_owned(),
            contents: None,
            created_time: self.created_time.to_owned(),
            notepad_id: self.notepad_id.to_owned(),
            title: self.title.to_owned(),
            updated_time: self.updated_time.to_owned(),
        };
        let s = serde_json::to_string_pretty(&temp).unwrap();
        write!(f, "contents len: {}\n{}", self.contents.as_ref().map_or(0, |c| c.len()), s)
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
pub struct MaimemoClient<'a> {
    client: Client,
    config: &'a AppConfig,
    cookie_store: CookieStore,
    user_token_name: String,
}

impl<'a> std::ops::Drop for MaimemoClient<'a> {
    /// 在退出时保存cookie store
    fn drop(&mut self) {
        if let Some(path) = self.config.get_cookie_path() {
            if let Err(e) = save_cookie_store(path, &self.cookie_store) {
                error!("save cookie store failed: {}", e);
            }
        }
    }
}

impl<'a> MaimemoClient<'a> {
    /// 用config构造一个client。如果config.cookie_path存在则加载，否则使用in memory的cookie store。
    pub fn new(config: &'a AppConfig) -> Result<Self, String> {
        Ok(Self {
            client: build_general_client()?,
            config,
            cookie_store: build_cookie_store(config.get_cookie_path())?,
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
        debug!("parsing notepad html");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONFIG_PATH: &str = "config.yml";
    #[tokio::test]
    async fn try_login() -> Result<(), String> {
        init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.get_maimemo())?;
        client.login().await.map_err(|e| format!("{:?}", e))?;
        Ok(())
    }

    #[tokio::test]
    async fn get_notepad_list() -> Result<(), String> {
        init_log();
        let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
        let mut client = MaimemoClient::new(config.get_maimemo())?;
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
        let mut client = MaimemoClient::new(config.get_maimemo())?;
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

    fn init_log() {
        pretty_env_logger::formatted_builder()
            // .format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()))
            .filter_module("youdao_dict_export::client", log::LevelFilter::Trace)
            .init();
    }

    // #[tokio::test]
    // async fn refresh_captcha() -> Result<(), String> {
    //     let config = Config::from_yaml_file(CONFIG_PATH).unwrap();
    //     let mut client = MaimemoClient::new(config.get_maimemo());
    //     if !client.has_logged() {
    //         client.login().await?;
    //     }
    //     let old_file = std::fs::read(config.get_maimemo().get_captcha_path());
    //     let path = client.refresh_captcha().await?;
    //     // assert!(path.is_file());
    //     // let new_file = std::fs::read(path).map_err(|e| format!("{:?}", e))?;
    //     // if let Ok(old_file) = old_file {
    //     //     assert_ne!(old_file, new_file);
    //     // }
    //     Ok(())
    // }

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
