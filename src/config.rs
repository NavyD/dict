use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io;

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestConfig {
    url: String,
    method: String,
    headers: Option<HashMap<String, String>>,
    form: Option<HashMap<String, String>>,
}

impl RequestConfig {
    pub fn get_url(&self) -> &str {
        &self.url
    }

    pub fn get_method(&self) -> &str {
        &self.method
    }

    pub fn get_headers(&self) -> Option<&HashMap<String, String>> {
        self.headers.as_ref()
    }

    pub fn get_form(&self) -> Option<&HashMap<String, String>> {
        self.form.as_ref()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    username: String,
    password: String,
    captcha_path: String,
    cookie_path: Option<String>,
    requests: Option<HashMap<String, RequestConfig>>,
}

impl Config {
    pub fn get_username(&self) -> &str {
        &self.username
    }

    pub fn get_password(&self) -> &str {
        &self.password
    }

    pub fn get_cookie_path(&self) -> Option<&str> {
        self.cookie_path.as_ref().map(|s| s.as_str())
    }

    pub fn get_requests(&self) -> Option<&HashMap<String, RequestConfig>> {
        self.requests.as_ref()
    }

    pub fn get_captcha_path(&self) -> &str {
        &self.captcha_path
    }
}

/// 从path yaml中加载配置。返回一个name-config的Map。这个name表示顶层元素如：name=youdao,maimemo
///
/// ```yaml
/// youdao:
///     username: a
///     password: a
///
/// maimemo:
///     username: a
///     password: a
/// ```
pub fn load_configs(path: &str) -> io::Result<HashMap<String, Config>> {
    std::fs::read_to_string(path).map(|contents| {
        match serde_yaml::from_str::<HashMap<String, Config>>(&contents) {
            // find a config with name
            Ok(v) => v,
            Err(e) => panic!("{} yaml file parse error: {}", path, e),
        }
    })
}

/// 从path yaml中加载配置并通过name过滤出一个config
pub fn load_config(path: &str, name: &str) -> io::Result<Config> {
    load_configs(path).map(|configs| {
        configs
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
            .expect(&format!("not found config name: {}", name))
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_configs_file() -> io::Result<()> {
        let path = "config.yml";
        load_configs(path)?.values().for_each(|config| {
            assert!(!config.username.is_empty());
            assert!(!config.password.is_empty());
        });
        Ok(())
    }

    #[test]
    fn load_config_by_name() -> io::Result<()> {
        let path = "config.yml";
        let config = load_config(path, "maimemo")?;
        assert!(!config.username.is_empty());
        assert!(!config.password.is_empty());
        Ok(())
    }
}

#[derive(Debug, Eq, Serialize, Deserialize)]
pub struct Cookie {
    name: String,
    value: String,
    // expires: String,
}

impl Cookie {
    pub fn from_reqwest_cookie(reqwest_cookie: &reqwest::cookie::Cookie) -> Self {
        Self {
            name: reqwest_cookie.name().to_string(),
            value: reqwest_cookie.value().to_string(),
        }
    }
}

impl PartialEq for Cookie {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Hash for Cookie {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

/// 通过req_name从Config中获取一个request config
pub fn get_request_config<'a>(config: &'a Config, req_name: &str) -> Option<&'a RequestConfig> {
    config.get_requests().and_then(|reqs| {
        reqs.iter()
            .find(|(name, _)| *name == req_name)
            .map(|(_, v)| v)
    })
}
use reqwest::{header::*, Client, Method, RequestBuilder};
/// 通过url, method构造一个request builder
pub fn request_builder(client: &Client, method: &str, url: &str) -> Result<RequestBuilder, String> {
    let method = Method::from_bytes(method.as_bytes()).map_err(|e| format!("{:?}", e))?;
    Ok(client.request(method, url))
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
        let name = HeaderName::from_lowercase(key.as_bytes()).map_err(|e| format!("{:?}", e))?;
        let val = HeaderValue::from_str(val).map_err(|e| format!("{:?}", e))?;
        if let Some(old) = req_headers.insert(name, val) {
            debug!("replace old header: {}={}", key, old.to_str().unwrap());
        }
    }
    Ok(req_builder.headers(req_headers))
    // headers.iter().for_each(|(key, val)| {
    //     let name = HeaderName::from_lowercase(key.as_bytes()).expect(&format!("{} to HeaderName error", key));
    //     let val = HeaderValue::from_str(val).expect(&format!("{} to HeaderValue error", val));
    //     req_headers.insert(name, val);
    // });
    // req_builder.headers(req_headers)
}

pub fn fill_form(req_builder: RequestBuilder, form: &HashMap<String, String>) -> RequestBuilder {
    let form = form
        .iter()
        .map(|(key, val)| (key.as_str(), val.as_str()))
        .collect::<Vec<_>>();
    req_builder.form(&form)
}

pub fn form_map_to_body(form: &HashMap<String, String>) -> Vec<(&str, &str)> {
    form
        .iter()
        .map(|(key, val)| (key.as_str(), val.as_str()))
        .collect::<Vec<_>>()
}

/// 从response中获取`set-cookie`s更新到cookie_store中。如果出现cookie无法解析或store无法插入则不会出错，可见debug log
pub fn update_set_cookies(
    cookie_store: &mut cookie_store::CookieStore,
    resp: &reqwest::Response,
    req_url: &str,
) {
    let url = &reqwest::Url::parse(req_url).unwrap();
    let set_cookies = resp
        .headers()
        .iter()
        .filter(|(name, _)| *name == reqwest::header::SET_COOKIE)
        .map(|(_, v)| v.to_str().unwrap())
        .collect::<Vec<_>>();
    for cookie_str in set_cookies {
        debug!("inserting set-cookie: {}", cookie_str);
        if let Err(e) = cookie::Cookie::parse(cookie_str).map(|raw_cookie| {
            if let Err(e) = cookie_store.insert_raw(&raw_cookie, url) {
                debug!("unable to store Set-Cookie: {:?}", e);
            }
        }) {
            debug!("parse Set-Cookie val error {:?}", e);
        }
    }
}

pub fn fill_request_cookies(
    cookie_store: &cookie_store::CookieStore,
    mut req_builder: RequestBuilder,
    req_url: &str,
) -> RequestBuilder {
    let url = &reqwest::Url::parse(req_url).unwrap();
    let delimiter = "; ";
    let mut vals = "".to_string();
    for c in cookie_store.get_request_cookies(url) {
        vals.push_str(c.name());
        vals.push('=');
        vals.push_str(c.value());
        vals.push_str(delimiter);
    }
    let start = vals.len() - delimiter.len();
    vals.drain(start..vals.len());
    match HeaderValue::from_str(&vals) {
        Ok(v) => {
            debug!("fill cookies [{}] in req: {}", v.to_str().unwrap(), url);
            req_builder = req_builder.header(reqwest::header::COOKIE, v);
        }
        Err(e) => {
            debug!("unable to request cookie: {}. error: {:?}", vals, e);
        }
    }
    req_builder
}


pub async fn send_request_nobody<T: Serialize + ?Sized>(
    client: &Client,
    cookie_store: &cookie_store::CookieStore,
    req_config: &RequestConfig,
    url: &str,
) -> Result<reqwest::Response, String> {
    send_request(client, cookie_store, req_config, url, None::<&str>).await
}

pub async fn send_request<T: Serialize + ?Sized>(
    client: &Client,
    cookie_store: &cookie_store::CookieStore,
    req_config: &RequestConfig,
    url: &str,
    body: Option<&T>,
) -> Result<reqwest::Response, String> {
    let mut req_builder = request_builder(client, req_config.get_method(), url)?;

    req_builder = fill_headers(
        req_builder,
        req_config
            .get_headers()
            .ok_or(format!("not found any headers in req url: {}", url))?,
    )?;
    req_builder = fill_request_cookies(cookie_store, req_builder, url);
    // parse body with content-type
    if let Some(body) = body {
        let content_type = req_config.get_headers().and_then(|headers| {
            headers
                .iter()
                .find(|(k, v)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v)
        });
        debug!("The body of the {:?} being parsed", content_type);
        let is_type = |header_val: &str| -> bool {
            content_type.map_or(false, |content| content.contains(header_val))
        };
        req_builder = if is_type("application/x-www-form-urlencoded") {
            serde_urlencoded::to_string(body)
                .map(|body| {
                    debug!("Url encoded the body: {}", body);
                    req_builder.body(body)
                })
                .map_err(|e| format!("{:?}", e))?
        } else if is_type("application/json") {
            // req_builder.body(body.into())
            serde_json::to_vec(body)
                .map(|body| {
                    debug!("jsoned body: {:?}", body);
                    req_builder.body(body)
                })
                .map_err(|e| format!("{:?}", e))?
        } else {
            todo!("unsupported content type: {:?}", content_type)
        }
    }
    debug!("request: {:?}", req_builder);
    let resp = req_builder.send().await.map_err(|e| format!("{:?}", e))?;
    debug!("response: {:?}", resp);
    Ok(resp)
}
