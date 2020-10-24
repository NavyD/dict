pub mod maimemo_client;
pub mod youdao_client;

use crate::config::*;
use cookie_store::CookieStore;
use reqwest::{header::*, Client, Method, RequestBuilder};
use serde::Serialize;
use std::collections::HashMap;
use std::{fs, io};

/// cookie store持久化
pub fn save_cookie_store(path: &str, cookie_store: &CookieStore) -> Result<(), String> {
    info!("Saving cookies to path {}", path);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("{:?}", e))?;
    cookie_store
        .save_json(&mut file)
        .map_err(|e| format!("{:?}", e))?;
    debug!("saved cookie store");
    Ok(())
}

/// 从path中创建一个cs, 如果path is none,则使用内存上的cs
pub fn build_cookie_store(cookie_path: Option<&str>) -> Result<CookieStore, String> {
    let cookie_store = if let Some(cookie_path) = cookie_path {
        // let path = fs::canonicalize(path).map_err(|e| format!("path {} error: {:?}", path, e))?;
        // let path_str = path.to_str().unwrap().to_string();
        debug!("opening cookie store from path: {}", cookie_path);
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(cookie_path)
            .map_err(|e| format!("path {} error: {:?}", cookie_path, e))?;
        CookieStore::load_json(io::BufReader::new(file)).map_err(|e| format!("{:?}", e))?
    } else {
        debug!("not found cookie store path. cookie store used in memory");
        CookieStore::default()
    };
    cookie_store
        .iter_unexpired()
        .for_each(|c| debug!("loaded unexpirted cookie: [{}={}]", c.name(), c.value()));
    Ok(cookie_store)
}

/// 一个不使用cookie store，重定向的client
pub fn build_general_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(false)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("{:?}", e))
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
                trace!("Url encoded the body: {}", body);
                req_builder.body(body)
            })
            .map_err(|e| format!("{:?}", e))
    } else if content_type.contains("application/json") {
        // req_builder.body(body.into())
        serde_json::to_vec(body)
            .map(|body| {
                trace!("jsoned body: {:?}", body);
                req_builder.body(body)
            })
            .map_err(|e| format!("{:?}", e))
    } else {
        todo!("unsupported content type: {:?}", content_type)
    }
}

/// 将cookie store中对应的url中的cookies填充requst builder
pub fn fill_request_cookies(
    cookie_store: &cookie_store::CookieStore,
    req_builder: RequestBuilder,
    req_url: &str,
) -> RequestBuilder {
    debug!("filling reqeust cookies");
    let url = &reqwest::Url::parse(req_url).unwrap();
    let delimiter = "; ";
    let mut cookies = "".to_string();
    for c in cookie_store.get_request_cookies(url) {
        cookies = cookies + c.name() + "=" + c.value() + delimiter;
    }
    if cookies.is_empty() {
        debug!("No cookies found for url: {}", url);
        return req_builder;
    }
    let start = cookies.len() - delimiter.len();
    cookies.drain(start..cookies.len());
    debug!("found reqeust cookie str: {}", cookies);
    match HeaderValue::from_str(&cookies) {
        Ok(v) => req_builder.header(reqwest::header::COOKIE, v),
        Err(e) => {
            warn!(
                "skiped unable to request cookie: {}. error: {:?}",
                cookies, e
            );
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
/// 如果header中存在不合法的key,val被跳过
pub fn fill_headers(
    req_builder: RequestBuilder,
    headers: &HashMap<String, String>,
) -> RequestBuilder {
    let mut req_headers = HeaderMap::new();
    for (key, val) in headers {
        trace!("filling request header: {}={}", key, val);
        let name = if let Ok(name) = HeaderName::from_lowercase(key.to_lowercase().as_bytes()) {
            name
        } else {
            warn!("skip invalid headername: {}", key);
            continue;
        };
        let val = if let Ok(name) = HeaderValue::from_str(val) {
            name
        } else {
            warn!("skip invalid header value: {}", val);
            continue;
        };
        if let Some(old) = req_headers.insert(name, val) {
            debug!("replace old header: {}={}", key, old.to_str().unwrap());
        }
    }
    req_builder.headers(req_headers)
}

/// 通过req_name从Config中获取一个request config
pub fn get_request_config<'a>(config: &'a AppConfig, req_name: &str) -> Option<&'a RequestConfig> {
    config.get_requests().and_then(|reqs| {
        reqs.iter()
            .find(|(name, _)| *name == req_name)
            .map(|(_, v)| v)
    })
}

pub async fn send_request_nobody<U: FnOnce(&str) -> String>(
    config: &AppConfig,
    client: &Client,
    cookie_store: &CookieStore,
    req_name: &str,
    url_handler: U,
) -> Result<reqwest::Response, String> {
    send_request(
        config,
        client,
        cookie_store,
        req_name,
        url_handler,
        None::<&str>,
    )
    .await
}

/// 获取req_name对应的config发送一个request
/// 
/// `url_handler`可以处理url。
///
/// 从config中读取url,method,headers与self.cookie_store中的cookie构造request
///
/// 如果body不为空，则通过header content-type处理，当前支持：
///
/// - json
/// - form
///
/// 如果response.status!=200 || != 302则返回error
///
pub async fn send_request<T: Serialize + ?Sized, U: FnOnce(&str) -> String>(
    config: &AppConfig,
    client: &Client,
    cookie_store: &CookieStore,
    req_name: &str,
    url_handler: U,
    body: Option<&T>,
) -> Result<reqwest::Response, String> {
    let req_config = get_request_config(config, req_name)
        .ok_or(format!("not found req config with req_name: {}", req_name))?;
    debug!("sending request: {}", req_name);

    let url = req_config.get_url();
    debug!("found the configured url: {}", url);
    let url = url_handler(url);
    debug!("new url: [{}] processed by url_handler", url);

    let method =
        Method::from_bytes(req_config.get_method().as_bytes()).map_err(|e| format!("{:?}", e))?;
    debug!("found the configured method: {}", method);

    let mut req_builder = client.request(method, &url);

    let headers = req_config
        .get_headers()
        .ok_or(format!("not found any headers in req url: {}", url))?;
    debug!("Fill in the request from the configured headers");
    req_builder = fill_headers(req_builder, headers);

    req_builder = fill_request_cookies(cookie_store, req_builder, &url);

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
                format!(
                    "not found content-type in request headers: {:?}",
                    req_config.get_headers()
                )
            })?;
        req_builder = fill_body(req_builder, content_type, body)?;
        // req_builder = req_builder.form(body);
    }

    trace!("sending request: {:?}", req_builder);
    let resp = req_builder.send().await.map_err(|e| format!("{:?}", e))?;
    trace!("response received: {:?}", resp);
    let status = resp.status();
    if status.as_u16() == 200 || status.as_u16() == 302 {
        Ok(resp)
    } else {
        let msg = format!("Response code error: {}", status);
        debug!("{}", msg);
        Err(msg)
    }
}
