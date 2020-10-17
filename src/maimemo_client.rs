use crate::config::*;
use chrono::{prelude::*, DateTime, NaiveDate};
use cookie_store::CookieStore;
use reqwest::{header::*, Client, Method, RequestBuilder};
use scraper::{Html, Selector};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};

#[derive(Debug, PartialEq)]
struct NotepadByHtml {
    pub id: usize,
    pub name: String,
    pub date: NaiveDate,
}

pub struct Notepad {
    id: String,
    is_private: bool,
    notepad_id: usize,
    title: String,
    created_time: DateTime<Local>,
    updated_time: DateTime<Local>,
}

struct ResponseResult {
    error: String,
    valid: i32,
    total: usize,
    notepad: Option<Vec<NotepadByHtml>>,
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

    pub fn has_logged(&self) -> bool {
        self.cookie_store
            .get("www.maimemo.com", "/", &self.user_token_name)
            .is_some()
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
        trace!("request: {:?}", req_builder);
        let resp = req_builder.send().await.map_err(|e| format!("{:?}", e))?;
        trace!("response: {:?}", resp);
        // login failed
        if resp.status().as_u16() != 200
            || resp
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
    fn parse_notepad_list_test() -> Result<(), String> {
        // let html = r#"<ul class="clearFix" id="notepadList"><li><span class="cloud-title"><a href="https://www.maimemo.com/notepad/detail/577168?scene=" class="edit">new_words</a></span><span class="cloud-literary"><a href="https://www.maimemo.com/notepad/detail/577168?scene=" class="edit">新单词</a></span><div class="cloud-Fabulous"><span class="clearFix"><span class="zan"><i class="icon icon-font"></i>1</span><span class="name" title="">navyd</span></span><span class="series">编号:577168&nbsp;&nbsp;时间:2019-07-17</span>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;标签：其他</div><p class="formula" style="clear: both; margin-top: 12px; display: none">undefined</p></li><li><span class="cloud-title"><a href="https://www.maimemo.com/notepad/detail/695835?scene=" class="edit">11_19</a></span><span class="cloud-literary"><a href="https://www.maimemo.com/notepad/detail/695835?scene=" class="edit">words</a></span><div class="cloud-Fabulous"><span class="clearFix"><span class="zan"><i class="icon icon-font"></i>1</span><span class="name" title="">navyd</span></span><span class="series">编号:695835&nbsp;&nbsp;时间:2019-11-19</span>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;标签：</div><p class="formula" style="clear: both; margin-top: 12px; display: none">undefined</p></li><li><span class="cloud-title"><a href="https://www.maimemo.com/notepad/detail/325025?scene=" class="edit">墨墨学员_3430044的云词本1</a></span><span class="cloud-literary"><a href="https://www.maimemo.com/notepad/detail/325025?scene=" class="edit">无简介</a></span><div class="cloud-Fabulous"><span class="clearFix"><span class="zan"><i class="icon icon-font"></i>1</span><span class="name" title="">navyd</span></span><span class="series">编号:325025&nbsp;&nbsp;时间:2018-08-19</span>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;标签：</div><p class="formula" style="clear: both; margin-top: 12px; display: none">undefined</p></li></ul>"#;
        // let notepads = MaimemoClient::parse_notepads(html.to_string())?;
        // let id = 577168;
        // assert_eq!(
        //     &NotepadByHtml {
        //         id,
        //         name: "new_words".to_string(),
        //         date: NaiveDate::from_ymd(2019, 7, 17)
        //     },
        //     notepads.iter().find(|n| n.id == id).unwrap()
        // );
        Ok(())
    }

    #[test]
    fn parse_notepads_from_file() -> Result<(), String> {
        // let result_converter = |e: ParseError<_>| format!("parse error. kind: {:?}, location: {:?}", e.kind, e.location);

        let result_converter = |e| format!("parse error. kind");
        let notepad_list_selector = Selector::parse("td.line-content").map_err(result_converter)?;
        let path = "notepads.html";
        let html = std::fs::read_to_string(path).map_err(|e| format!("{}", e))?;
        let document = Html::parse_document(&html);
        println!("{:?}", document.errors);
        for li in document.select(&notepad_list_selector) {
            println!("{:?}", li)
        }
        // assert_eq!(3, notepads.len());
        Ok(())
    }
}
