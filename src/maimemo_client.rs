use crate::*;

use chrono::{NaiveDate};
use reqwest::Client;
use scraper::{Html, Selector};
use cssparser::ParseError;

#[derive(Debug, PartialEq)]
pub struct Notepad {
    pub id: usize,
    pub name: String,
    pub date: NaiveDate,
}

pub struct MaimemoClient {
    client: Client,
    config: Config,
}

impl MaimemoClient {
    pub fn new(config: Config) -> Self {
        let client = Client::builder()
            .cookie_store(false)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build client error");
        Self { client, config }
    }

    /// 登录并更新config.cookies
    pub async fn login(&mut self) -> Result<(), reqwest::Error> {
        // self.prepare_login().await?;
        let url = "https://www.maimemo.com/auth/login";
        let resp = self.client.post(url)
            .header("accept", "application/json, text/javascript, */*; q=0.01")
            .header("accept-encoding", "gzip, deflate, br")
            .header("accept-language", "zh-CN,zh;q=0.9")
            .header("cache-control", "no-cache")
            // .header("content-length", "45")
            .header("content-type", "application/x-www-form-urlencoded; charset=UTF-8")
            // .header("cookie", "PHPSESSID=f57b3f6e88ffc06d905ae9cdff4ffa36; Hm_lvt_8d4c70ef9b50f1ed364481083d6a8636=1602831063; Hm_lpvt_8d4c70ef9b50f1ed364481083d6a8636=1602831063")
            .header("origin", "https://www.maimemo.com")
            .header("pragma", "no-cache")
            .header("referer", "https://www.maimemo.com/home/login")
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-origin")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            .header("x-requested-with", "XMLHttpRequest")
            .form(&[
                ("email", &self.config.username),
                ("password", &self.config.password),
            ])
            .send()
            .await?
            ;
        let user_token_name = "userToken";
        self.update_config(&resp);
        // login failed
        if resp
            .cookies()
            .find(|c| c.name() == user_token_name)
            .is_none()
        {
            let s = format!("{:?}", resp);
            error!("login failed. resp: {}\ntext:{}", s, resp.text().await?);
        }
        Ok(())
    }

    /// 获取notepads list
    pub async fn fetch_notepads(&mut self) -> Result<Vec<Notepad>, reqwest::Error> {
        let url = "https://www.maimemo.com/notepad/show";
        let req = self.client
            .get(url)
            .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
            .header("accept-encoding", "gzip, deflate, br")
            .header("accept-language", "zh-CN,zh;q=0.9")
            .header("cache-control", "no-cache")
            // .header("cookie", "PHPSESSID=f57b3f6e88ffc06d905ae9cdff4ffa36; Hm_lvt_8d4c70ef9b50f1ed364481083d6a8636=1602831063; userToken=e8c72bad7e803232fbe4e0d8189f2cdad0587830934ff600b030f9e353d4cf68; Hm_lpvt_8d4c70ef9b50f1ed364481083d6a8636=1602846830")
            .header("pragma", "no-cache")
            .header("referer", "https://www.maimemo.com/")
            .header("sec-fetch-dest", "document")
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-user", "?1")
            .header("upgrade-insecure-requests", "1")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            ;
        let resp = self.put_cookies_to_req(req).send().await?;
        if resp.status().as_u16() != 200 {
            error!("show notepad error. response: {:?}", resp);
        }
        self.update_config(&resp);
        let html = resp.text().await?;
        Self::parse_notepads(html).map_err(|e| panic!("parse notepads error: {}", e))
        // Ok(vec![])
    }

    /// 解析`https://www.maimemo.com/notepad/show` html页面并返回一个notepad list。
    /// 如果解析失败则返回一个string
    fn parse_notepads(html: String) -> Result<Vec<Notepad>, String> {
        
        let result_converter = |e: ParseError<_>| format!("parse error. kind: {:?}, location: {:?}", e.kind, e.location);

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
            notepads.push(Notepad { id, name, date });
        }

        // let a = Html::parse_fragment(&html).select(&notepad_list_selector);
        Ok(notepads)
    }

    /// 将config.cookies中的cookie放到request.header中
    fn put_cookies_to_req(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(cookies) = self.config.cookies.as_ref() {
            let mut val = "".to_string();
            cookies
                .iter()
                .for_each(|c| val = val.to_string() + &c.name + "=" + &c.value + "; ");
            req.header("cookie", val)
        } else {
            req
        }
    }

    /// 通过resp更新config中的cookies。如果cookie已存在则替换
    fn update_config(&mut self, resp: &reqwest::Response) {
        resp.cookies().for_each(|cookie| {
            // put cookie into the config
            if let Some(old_cookie) = self
                .config
                .cookies
                .as_mut()
                .and_then(|cookies| cookies.replace(Cookie::from_reqwest_cookie(&cookie)))
            {
                debug!(
                    "The old cookie has been updated: {}={}",
                    old_cookie.name, old_cookie.value
                );
            } else {
                debug!("add new cookie: {}={}", cookie.name(), cookie.value());
            }
        });
    }

    /// get PHPSESSID cookie
    async fn prepare_login(&mut self) -> Result<(), reqwest::Error> {
        let url = "https://www.maimemo.com/home/login";
        let resp = self.client.get(url)
            .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
            .header("accept-encoding", "gzip, deflate, br")
            .header("accept-language", "zh-CN,zh;q=0.9")
            .header("sec-fetch-dest", "document")
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-site", "none")
            .header("upgrade-insecure-requests", "1")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
            .send()
            .await?
            ;
        match (
            resp.status().as_u16(),
            resp.cookies().find(|c| c.name() == "PHPSESSID"),
        ) {
            (200, Some(phpsessid)) => {
                debug!("get maimemo cookie: PHPSESSID={}", phpsessid.name());
            }
            _ => panic!(
                "acquisition failed maimemo cookie: PHPSESSID. resp: {:?}",
                resp
            ),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_notepad_list_test() -> Result<(), String> {
        let html = r#"<ul class="clearFix" id="notepadList"><li><span class="cloud-title"><a href="https://www.maimemo.com/notepad/detail/577168?scene=" class="edit">new_words</a></span><span class="cloud-literary"><a href="https://www.maimemo.com/notepad/detail/577168?scene=" class="edit">新单词</a></span><div class="cloud-Fabulous"><span class="clearFix"><span class="zan"><i class="icon icon-font"></i>1</span><span class="name" title="">navyd</span></span><span class="series">编号:577168&nbsp;&nbsp;时间:2019-07-17</span>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;标签：其他</div><p class="formula" style="clear: both; margin-top: 12px; display: none">undefined</p></li><li><span class="cloud-title"><a href="https://www.maimemo.com/notepad/detail/695835?scene=" class="edit">11_19</a></span><span class="cloud-literary"><a href="https://www.maimemo.com/notepad/detail/695835?scene=" class="edit">words</a></span><div class="cloud-Fabulous"><span class="clearFix"><span class="zan"><i class="icon icon-font"></i>1</span><span class="name" title="">navyd</span></span><span class="series">编号:695835&nbsp;&nbsp;时间:2019-11-19</span>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;标签：</div><p class="formula" style="clear: both; margin-top: 12px; display: none">undefined</p></li><li><span class="cloud-title"><a href="https://www.maimemo.com/notepad/detail/325025?scene=" class="edit">墨墨学员_3430044的云词本1</a></span><span class="cloud-literary"><a href="https://www.maimemo.com/notepad/detail/325025?scene=" class="edit">无简介</a></span><div class="cloud-Fabulous"><span class="clearFix"><span class="zan"><i class="icon icon-font"></i>1</span><span class="name" title="">navyd</span></span><span class="series">编号:325025&nbsp;&nbsp;时间:2018-08-19</span>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;标签：</div><p class="formula" style="clear: both; margin-top: 12px; display: none">undefined</p></li></ul>"#;
        let notepads = MaimemoClient::parse_notepads(html.to_string())?;
        let id = 577168;
        assert_eq!(
            &Notepad {
                id,
                name: "new_words".to_string(),
                date: NaiveDate::from_ymd(2019, 7, 17)
            },
            notepads.iter().find(|n| n.id == id).unwrap()
        );
        Ok(())
    }

    #[test]
    fn parse_notepads_from_file()  -> Result<(), String> {
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
