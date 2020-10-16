pub mod maimemo_client;
pub mod word_store;
pub mod youdao_client;

pub extern crate pretty_env_logger;
#[macro_use]
pub extern crate log;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    username: String,
    password: String,
    cookies: Option<std::collections::HashSet<Cookie>>,
}

impl Config {}

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
