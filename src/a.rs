use std::collections::HashMap;
use reqwest::{
    self,
    Client,
    ClientBuilder,
    Method,
    RequestBuilder,
    Request,
    Url,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let resp = reqwest::get("https://httpbin.org/ip")
    //     .await?
    //     .json::<HashMap<String, String>>()
    //     .await?;
    let resp = Client::new()
        .post("https://logindict.youdao.com/login/acc/login")
        .header("HOST", "logindict.youdao.com")
        .header("Connection", "keep-alive")
        .header("Content-Length", "225")
        .header("Upgrade-Insecure-Requests", "1")
        .header("Origin", "http://account.youdao.com")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0.4183.121 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
        .header("Sec-Fetch-Site", "cross-site")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-User", "?1")
        .header("Sec-Fetch-Dest", "document")
        .header("Referer", "http://account.youdao.com/login?service=dict&back_url=http%3A%2F%2Fdict.youdao.com%2Fwordbook%2Fwordlist%3Fkeyfrom%3Ddict2.index%23%2F")
        .header("Accept-Encoding", "gzip, deflate, br")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Cookie", "DICT_UGC=be3af0da19b5c5e6aa4e17bd8d90b28a|; OUTFOX_SEARCH_USER_ID=-284983025@113.89.43.166; JSESSIONID=abcVQFpmrdJBc-TaNcetx; OUTFOX_SEARCH_USER_ID_NCOO=1538462202.6844223")
        // .form("app=web&tp=urstoken&cf=3&fr=1&ru=http%3A%2F%2Fdict.youdao.com%2Fwordbook%2Fwordlist%3Fkeyfrom%3Ddict2.index%23%2F&product=DICT&type=1&um=true&username=dhjnavyd%40163.com&password=4ff32ab339c507639b234bf2a2919182&agreePrRule=1")
        .send()
        .await?;
    println!("{:#?}", resp);
    Ok(())
}
// login 
// 
// request:
// POST /login/acc/login HTTP/1.1
// Host: logindict.youdao.com
// Connection: keep-alive
// Content-Length: 225
// Cache-Control: max-age=0
// Upgrade-Insecure-Requests: 1
// Origin: http://account.youdao.com
// Content-Type: application/x-www-form-urlencoded
// User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0.4183.121 Safari/537.36
// Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
// Sec-Fetch-Site: cross-site
// Sec-Fetch-Mode: navigate
// Sec-Fetch-User: ?1
// Sec-Fetch-Dest: document
// Referer: http://account.youdao.com/login?service=dict&back_url=http%3A%2F%2Fdict.youdao.com%2Fwordbook%2Fwordlist%3Fkeyfrom%3Ddict2.index%23%2F
// Accept-Encoding: gzip, deflate, br
// Accept-Language: zh-CN,zh;q=0.9
// Cookie: OUTFOX_SEARCH_USER_ID=-702231008@113.89.42.215
// 
// form:
// app=web&tp=urstoken&cf=3&fr=1&ru=http%3A%2F%2Fdict.youdao.com%2Fwordbook%2Fwordlist%3Fkeyfrom%3Ddict2.index%23%2F&product=DICT&type=1&um=true&username=dhjnavyd%40163.com&password=4ff32ab339c507639b234bf2a2919182&agreePrRule=1
// 
// response:
// HTTP/1.1 302
// Server: nginx
// Date: Fri, 25 Sep 2020 06:09:06 GMT
// Content-Length: 0
// Connection: keep-alive
// Vary: Origin
// Vary: Access-Control-Request-Method
// Vary: Access-Control-Request-Headers
// Access-Control-Allow-Origin: http://account.youdao.com
// Access-Control-Allow-Credentials: true
// Cache-Control: no-cache, no-store, must-revalidate
// Pragma: no-cache
// Expires: Thu, 01 Jan 1970 00:00:00 GMT
// P3P: CP="CAO CONi ONL OUR"
// Set-Cookie: DICT_SESS=v2|URSM|DICT||dhjnavyd@163.com||urstoken||SJXYKuGC.S7QY.nnT2c86PNTzRo0ZGfOLKvla_CxONxkd3GbdnHABw4MIot6zMRgDv9p4ZZrpFvja2zHL00DmJMz6.NRlvk.PSdxBQqXFUBetrCiDCIjilLTPhTopC_Q6DEqxtP7_U3wmkmJP4Qn59sQ6hbtrbqQRcpXSL5StljjILlpNeWzeTSPQKsbdNe7yvvLd_wi0YZYw||604800000||quPLJLPMP4Rz50fl5PMYERQKh4T40HQuRlW64YGhHUfRQ4RLJzhfw4RTuhHU5n4Pu0QBhHUfRMpBRTu6LQKOMPF0; Domain=.youdao.com; Path=/; HttpOnly
// Set-Cookie: DICT_LOGIN=1||1601014146905; Domain=.youdao.com; Path=/
// Location: http://dict.youdao.com/wordbook/wordlist?keyfrom=dict2.index#/

use quick_xml::events::Event;
use quick_xml::Reader;

fn main1() {
    let file = "/home/navyd/Workspaces/projects/youdao-dict-export/a.xml";
    let word_txt = get_word_text(file);
    for i in 0..100 {
        println!("{}", word_txt.get(i).unwrap());
    }
}

fn get_word_text(path: &str) -> Vec<String> {
    let mut word_txt = Vec::new();

    let mut reader = Reader::from_file(path).expect("not found file");
    reader.trim_text(true);
    let mut buf = Vec::new();

    // The `Reader` does not implement `Iterator` because it outputs borrowed data (`Cow`s)
    loop {
        match reader.read_event(&mut buf) {
            // for triggering namespaced events, use this instead:
            // match reader.read_namespaced_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                // for namespaced:
                // Ok((ref namespace_value, Event::Start(ref e)))
                match e.name() {
                    // get tag `word` content
                    b"word" => {
                        match reader.read_event(&mut buf) {
                            Ok(Event::Text(e)) => {
                                word_txt.push(e.unescape_and_decode(&reader).unwrap());
                            }
                            _ => (),
                        };
                    }
                    _ => (),
                }
            }
            Ok(Event::Eof) => break, // exits the loop when reaching end of file
            Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            _ => (), // There are several other `Event`s we do not consider here
        }

        // if we don't keep a borrow elsewhere, we can clear the buffer to keep memory usage low
        buf.clear();
    }
    word_txt
}