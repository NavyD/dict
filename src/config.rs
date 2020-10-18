use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;

/// 一个对应.yml文件的配置struct
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    maimemo: Maimemo,
}

impl Config {
    /// 从path yaml中加载配置。
    /// 
    /// # Errors
    /// 
    /// 如果path不存在或其它问题，yaml解析失败返回error
    pub fn from_yaml_file(path: &str) -> io::Result<Config> {
        std::fs::read_to_string(path).and_then(|contents| {
            serde_yaml::from_str::<Config>(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{:?}", e)))
        })
    }

    pub fn get_maimemo(&self) -> &Maimemo {
        &self.maimemo
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Maimemo {
    username: String,
    password: String,
    captcha_path: String,
    cookie_path: Option<String>,
    requests: Option<HashMap<String, RequestConfig>>,
}

impl Maimemo {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestConfig {
    url: String,
    method: String,
    headers: Option<HashMap<String, String>>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_maimemo_from_file() -> io::Result<()> {
        let path = "config.yml";
        let config = Config::from_yaml_file(path)?;
        let maimemo = config.get_maimemo();
        assert_eq!(maimemo.get_username(), "dhjnavyd@gmail.com");
        assert!(maimemo.get_password().len() > 0);
        assert_eq!("maimemo-captcha.png", maimemo.get_captcha_path());
        assert_eq!(Some("maimemo-cookies.json"), maimemo.get_cookie_path());
        if let Some(requests ) = maimemo.get_requests() {
            assert_eq!(requests.len(), 5);
        }
        Ok(())
    }
}
