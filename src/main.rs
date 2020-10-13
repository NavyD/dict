use reqwest::header::*;
use reqwest::{self, Client, ClientBuilder, Method, Request, RequestBuilder, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::prelude::*;
use std::io::Cursor;

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
#[derive(Serialize, Deserialize, Debug)]
struct WordItem {
    #[serde(rename = "itemId")]
    item_id: String,
    #[serde(rename = "bookId")]
    book_id: String,
    #[serde(rename = "bookName")]
    book_name: String,
    word: String,
    trans: String,
    phonetic: String,
    #[serde(rename = "modifiedTime")]
    modified_time: usize,
}

#[tokio::main]
async fn main() -> Result<(), reqwest::Error> {
    let client = Client::builder()
        .cookie_store(true)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
        .build()?;
    // let a = get_words().await?;
    let username = "dhjnavyd@163.com";
    let password = "4ff32ab339c507639b234bf2a2919182";
    login(client, username, password).await?;
    Ok(())
}
async fn login(client: Client, username: &str, password: &str) -> Result<(), reqwest::Error> {
    let outfox_search_user_id = get_cookie_outfox_search_user_id(client.clone()).await?;
    let url = "https://logindict.youdao.com/login/acc/login";
    // Content-Length: 237
    // Cookie: OUTFOX_SEARCH_USER_ID=-92990524@113.89.40.63
    let headers = r#"Host: logindict.youdao.com
Connection: keep-alive
Cache-Control: max-age=0
Upgrade-Insecure-Requests: 1
Origin: http://account.youdao.com
Content-Length: 237
Content-Type: application/x-www-form-urlencoded
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Sec-Fetch-Site: cross-site
Sec-Fetch-Mode: navigate
Sec-Fetch-User: ?1
Sec-Fetch-Dest: document
Referer: http://account.youdao.com/
Accept-Encoding: gzip, deflate, br
Accept-Language: zh-CN,zh;q=0.9"#;
    let headers = parse_key_values(headers, ": ");
    let resp = set_builder_headers(&headers, client.post(url))
        .header(COOKIE, format!("OUTFOX_SEARCH_USER_ID={}", outfox_search_user_id))
        .form(&[
            ("app", "web"),
            ("tp", "urstoken"),
            ("cf", "7"),
            ("fr", "1"),
            (
                "ru",
                "http://dict.youdao.com/wordbook/wordlist?keyfrom=dict2.index#/",
            ),
            ("product", "DICT"),
            ("type", "1"),
            ("um", "true"),
            ("username", username),
            ("password", password),
            // 同意登录
            ("agreePrRule", "1"),
            // 保存cookie登录
            ("savelogin", "1"),
        ])
        
        .send()
        .await?
        ;
    println!("{:?}", resp);
    println!("{}", resp.text().await?);
    Ok(())
}

/// 获取youdao set-cookie: outfox_search_user_id，保证后续登录有效
async fn get_cookie_outfox_search_user_id<'a>(client: Client) -> Result<String, reqwest::Error> {
    let url = "http://account.youdao.com/login?service=dict&back_url=http%3A%2F%2Fdict.youdao.com%2Fwordbook%2Fwordlist%3Fkeyfrom%3Ddict2.index%23%2F";
    let headers = r#"Host: account.youdao.com
Connection: keep-alive
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: gzip, deflate
Accept-Language: zh-CN,zh;q=0.9"#;
    let headers = parse_key_values(headers, ": ");
    // set_builder_headers(&headers, client.get(url))
    //     .send()
    //     .await?
    //     .headers()
    //     .get(SET_COOKIE)
    //     .ok_or_else(|| panic!("not found set cookie in headers"))
    //     .map(|v| v.to_str().unwrap().to_string())
    let name = "OUTFOX_SEARCH_USER_ID";
    set_builder_headers(&headers, client.get(url))
        .send()
        .await?
        .cookies()
        .find(|c| c.name() == name)
        .map(|c| c.value().to_string())
        .ok_or_else(|| panic!("not found set cookie: {} in headers", name))
}

async fn get_words() -> Result<Vec<WordItem>, reqwest::Error> {
    let resp = Client::new()
        .get("http://dict.youdao.com/wordbook/webapi/words?limit=1500")
        .header("HOST", "dict.youdao.com")
        .header("Connection", "keep-alive")
        .header("Cache-Control", "max-age=0")
        // .header("Content-Length", "225")
        .header("Upgrade-Insecure-Requests", "1")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.75 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
        .header("Accept-Encoding", "gzip, deflate")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Cookie", "OUTFOX_SEARCH_USER_ID_NCOO=700868444.2922732; OUTFOX_SEARCH_USER_ID=\"431531447@10.169.0.84\"; JSESSIONID=abcAugXD8uv4G-msLMdtx; SL_GWPT_Show_Hide_tmp=1; SL_wptGlobTipTmp=1; DICT_UGC=bde5f2bab547edc50fe3805144291d44|dhjnavyd@163.com; ___rl__test__cookies=1601014088518; DICT_FORCE=true; DICT_SESS=v2|URSM|DICT||dhjnavyd@163.com||urstoken||ccpzZBq00g_RpAgmbU4RC9.SuDKlewHP.iZUb6FyJ3yfsHBnse9N0v21jmOTR1WdQZwA2KKDA_ZqbpR9uCCQ5k9kW6fKv.J4Acsy0o8x_I0zODFgQFjqgUul.LlmAF6oTQM8yO.X6IHcu1qBTWdvdN0mcF4s2zDJnqp8MlkritYPzVa70rYAezX.oi4ns3zXSZZus6vgCrKrv||604800000||P4O4P4OMqy0lfPLgBnfUWRgF0LUl64zm0QyOfeun4640JK64pzh4k50gBnMkA0fT40OWOMkWRMJS06uOfUAh4zfR; DICT_PERS=v2|urstoken||DICT||web||-1||1602567639902||113.89.43.90||dhjnavyd@163.com||eBRHqK0Lqy0wBnHk5h4Ju0PBk4U5RfUMRzM6LTz0LYY0zMP4OmhMgZ06unMp4kfJF0gK0fkGRMlERzmO4zGhLwuR; DICT_LOGIN=7||1602567639938")
        .send()
        .await?
        .json::<ResponseResult<Page<WordItem>>>()
        .await?;
    // println!("{:#?}", resp);
    // word_items.sort_unstable_by(|a, b| b.modified_time.cmp(&a.modified_time));
    Ok(resp.data.item_list)
}

/// 通过`pat`分离多行的k-v返回一个map。map只会返回成功分离的k-v。
///
/// # panic
///
/// 如果存在一行不能通过pat分离为k-v形式，则panic
fn parse_key_values(params: &str, pat: &str) -> HashMap<String, String> {
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
            let line = s.split(pat).collect::<Vec<&str>>();
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

fn set_builder_headers(
    headers: &HashMap<String, String>,
    mut req_builder: RequestBuilder,
) -> RequestBuilder {
    for (key, value) in headers {
        req_builder = req_builder.header(key, value);
    }
    req_builder
}


// fn to_header_map(headers: &HashMap<String, String>) -> HeaderMap {
//     let mut header_map = HeaderMap::new();
//     headers.iter().for_each(|(k, v)| {
//         if let Ok(key) = HeaderName::from_lowercase(k.as_bytes()) {
//             HeaderValue::from
//             header_map.insert(key, v);
//         }
//     });
//     header_map

// }
