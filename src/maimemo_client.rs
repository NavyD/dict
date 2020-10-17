use crate::config::*;
use chrono::{prelude::*, DateTime, NaiveDate};
use cookie_store::CookieStore;
use reqwest::{header::*, Client, Method, RequestBuilder};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
struct NotepadByHtml {
    pub id: usize,
    pub name: String,
    pub date: NaiveDate,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Notepad {
    id: String,
    is_private: u8,
    notepad_id: String,
    title: String,
    brief: String,
    created_time: String,
    updated_time: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseResult {
    error: String,
    valid: i32,
    total: usize,
    notepad: Option<Vec<Notepad>>,
}

pub struct MaimemoClient {
    client: Client,
    config: Config,
    cookie_store: CookieStore,
    user_token_name: String,
}

impl std::ops::Drop for MaimemoClient {
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

impl MaimemoClient {
    /// 用config构造一个client。如果config.cookie_path存在则加载，否则使用in memory的cookie store。
    pub fn new(config: Config) -> Self {
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
        // self.prepare_login().await?;
        let req_config = get_request_config(&self.config, req_name)
            .ok_or(format!("not found req config with req_name: {}", req_name))?;
        let url = req_config.get_url();
        let mut req_builder = request_builder(&self.client, req_config.get_method(), url)?;
        req_builder = fill_headers(
            req_builder,
            req_config
                .get_headers()
                .ok_or(format!("not found any headers in req name: {}", req_name))?,
        )?;
        req_builder = fill_request_cookies(&self.cookie_store, req_builder, url);
        req_builder = fill_form(
            req_builder,
            req_config
                .get_form()
                .ok_or(format!("not found any form in req name: {}", req_name))?,
        );
        debug!("request: {:?}", req_builder);
        let resp = req_builder.send().await.map_err(|e| format!("{:?}", e))?;
        debug!("response: {:?}", resp);
        // login failed
        if resp.status().as_u16() != 200
            || resp // find user token
                .cookies()
                .find(|c| c.name() == self.user_token_name)
                .is_none()
        {
            let s = format!("{:?}", resp);
            error!(
                "login failed. resp: {}\nbody:{}",
                s,
                resp.text().await.map_err(|e| format!("{:?}", e))?
            );
            Err("login failed".to_string())
        } else {
            // Check if the user token exists
            update_set_cookies(&mut self.cookie_store, &resp, url);
            if self.has_logged() {
                Ok(())
            } else {
                error!(
                    "update cookie store failed. not found cookie: {} in cookie_store",
                    self.user_token_name
                );
                Err("login failed. not found cookie store".to_string())
            }
        }
    }

    /// 获取notepad list
    pub async fn get_notepad_list(&mut self) -> Result<Vec<Notepad>, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        // self.show().await.map_err(|e| format!("{:?}", e))?;
        let req_name = "notepad-search";
        let req_config = get_request_config(&self.config, req_name)
            .ok_or(format!("not found req config with req_name: {}", req_name))?;
        let url = self
            .get_user_token_val()
            .map(|val| {
                let mut url = req_config.get_url().to_string();
                url.push_str(val);
                url
            })
            .ok_or(format!("not found url"))?;
        let payload = serde_json::json!({"keyword":null,"scope":"MINE","recommend":false,"offset":0,"limit":30,"total":-1});
        let resp = send_request(
            &self.client,
            &self.cookie_store,
            req_config,
            &url,
            Some(&payload),
        )
        .await?;
        if resp.status().as_u16() != 200 {
            let s = format!("{:?}", resp);
            error!(
                "login failed. resp: {}\nbody:{}",
                s,
                resp.text().await.map_err(|e| format!("{:?}", e))?
            );
            return Err("login failed".to_string());
        } else {
            update_set_cookies(&mut self.cookie_store, &resp, &url);
            let result = resp
                .json::<ResponseResult>()
                .await
                .map_err(|e| format!("{:?}", e))?;
            if result.valid != 1 || result.notepad.is_none() {
                warn!("get notepad failed: {:?}", result);
                return Err("get notepad failed".to_string());
            }
            Ok(result.notepad.unwrap())
        }
    }

    /// 获取notepad中单词文本
    pub async fn get_notepad(&self, notepad_id: &str) -> Result<String, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "notepad-detail";
        let req_config = get_request_config(&self.config, req_name)
            .ok_or(format!("not found req config with req_name: {}", req_name))?;
        let url = req_config.get_url().to_owned() + notepad_id;
        let resp = send_request(
            &self.client,
            &self.cookie_store,
            req_config,
            &url,
            None::<&str>,
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        if resp.status().as_u16() != 200 {
            debug!("Failed to fetch notepad response: {:?}", resp);
            return Err(format!("fetch notepad failed"));
        }
        Self::parse_notepad_text(&resp.text().await.map_err(|e| format!("{:?}", e))?)
    }

    /// 刷新下载notepad对应的captcha返回文件全路径
    pub async fn refresh_captcha(&self) -> Result<PathBuf, String> {
        if !self.has_logged() {
            return Err("not logged in".to_string());
        }
        let req_name = "service-captcha";
        let req_config = get_request_config(&self.config, req_name)
            .ok_or(format!("not found req config with req_name: {}", req_name))?;
        let url = req_config.get_url().to_owned() + &Local::now().timestamp_nanos().to_string();
        let resp = send_request(
            &self.client,
            &self.cookie_store,
            req_config,
            &url,
            None::<&str>,
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        if resp.status().as_u16() != 200 {
            debug!("Failed to fetch captcha response: {:?}", resp);
            return Err(format!("fetch captcha failed"));
        }
        let contents = resp
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|e| format!("{:?}", e))?;
        let path = fs::canonicalize(self.get_captcha_path()).map_err(|e| format!("{:?}", e))?;
        debug!(
            "writing the content of captcha to the file: {}",
            path.to_str().unwrap()
        );
        fs::write(path.to_owned(), contents).map_err(|e| format!("{:?}", e))?;
        Ok(path)
    }

    pub async fn save_notepad(&self, notepad: &Notepad, contents: String, captcha: &str) -> Result<(), String> {
        let req_name = "notepad-save";
        let req_config = get_request_config(&self.config, req_name)
        .ok_or(format!("not found req config with req_name: {}", req_name))?;
        let url = req_config.get_url();
        let mut form = std::collections::HashMap::new();
        form.insert("id".to_string(), notepad.notepad_id.to_string());
        form.insert("title".to_string(), notepad.title.to_string());
        form.insert("brief".to_string(), notepad.brief.to_string());
        form.insert("content".to_string(), contents);
        form.insert("is_private".to_string(), notepad.is_private.to_string());
        form.insert("captcha".to_string(), captcha.to_string());
        let resp = send_request(
            &self.client,
            &self.cookie_store,
            req_config,
            &url,
            Some(&form_map_to_body(&form)),
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        if resp.status().as_u16() != 200 {
            debug!("Failed to save_notepad response: {:?}", resp);
            return Err(format!("save_notepad failed"));
        }
        Ok(())
    }

    pub fn get_captcha_path(&self) -> &str {
        &self.config.get_captcha_path()
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

    

    /*
    /// 解析`https://www.maimemo.com/notepad/show` html页面并返回一个notepad list。
    /// 如果解析失败则返回一个string
    fn parse_notepads(html: String) -> Result<Vec<NotepadByHtml>, String> {
            let result_converter = |e: ParseError<_>| {
                format!(
                    "parse error. kind: {:?}, location: {:?}",
                    e.kind, e.location
                )
            };

            // let result_converter = |e| format!("parse error. kind");
            let notepad_list_selector = Selector::parse(".clearFix li").map_err(result_converter)?;
            // <a href="https://www.maimemo.com/notepad/detail/577168?scene=" class="edit">new_words</a>
            let a_selector = Selector::parse(".cloud-title a").map_err(result_converter)?;
            // <span class="series">编号:577168&nbsp;&nbsp;时间:2019-07-17</span>
            let series_selector = Selector::parse(".series").map_err(result_converter)?;

            let mut notepads = vec![];
            for li in Html::parse_fragment(&html).select(&notepad_list_selector) {
                let name = li
                    .select(&a_selector)
                    // there is only one name in the a tag
                    .next()
                    .map(|a| a.inner_html())
                    .ok_or(format!("not found name in the li: {} ", li.html()))?;
                // 编号:577168&nbsp;&nbsp;时间:2019-07-17
                let series = li
                    .select(&series_selector)
                    .map(|series_ele| series_ele.inner_html())
                    // there is only one series in the series class
                    .next()
                    .map(|s| {
                        // Separate id and time
                        s.split("&nbsp;&nbsp;")
                            // Separate the corresponding tag and data and combine the previously separated
                            .flat_map(|s| s.split(':'))
                            .map(|a| a.to_string())
                            // ["编号", "577168", "时间", "2019-07-17"]
                            .collect::<Vec<_>>()
                    })
                    .ok_or(format!("not found series in the li: {}", li.html()))?;
                debug!("original series: {:?}", series);
                // parse id and date
                let id = series[1].parse::<usize>().map_err(|e| format!("{:?}", e))?;
                let date = NaiveDate::parse_from_str(&series[3], "%Y-%m-%d")
                    .map_err(|e| format!("{:?}", e))?;
                debug!("name: {}, id: {}, date: {}", name, id, date);
                notepads.push(NotepadByHtml { id, name, date });
            }

            // let a = Html::parse_fragment(&html).select(&notepad_list_selector);
            Ok(notepads)
        }
    */
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONFIG_PATH: &str = "config.yml";
    const CONFIG_NAME: &str = "maimemo";

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
    #[test]
    fn drop_test() -> io::Result<()> {
        init_log(true);
        let config = load_config(CONFIG_PATH, CONFIG_NAME)?;
        MaimemoClient::new(config);
        Ok(())
    }

    #[test]
    fn parse_notepad_words_from_file() -> Result<(), String> {
        let path = "notepad.html";
        let html = std::fs::read_to_string(path).map_err(|e| format!("{}", e))?;
        let words = MaimemoClient::parse_notepad_text(&html)?;
        assert!(words.len() > 0);
        assert!(words.contains("墨墨学员_3430044的云词本1"));
        Ok(())
    }
}
