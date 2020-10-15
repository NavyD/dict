use crate::youdao_client::WordItem;

use std::io;
use std::fs;
use std::path::Path;

/// 一个word store。提供单词本的缓存与持久化
pub struct WordStore {
    words: Vec<WordItem>,
}

impl WordStore {
    pub fn new(words: Vec<WordItem>) -> Self {
        Self {words}
    }

    /// 用一个file构造WordStore。当前仅支持由WordStore持久化的格式文件json，
    /// 
    /// 用其它不可识别的file构造将导致Err
    pub fn from_file<P: AsRef<Path>>(from_file: P) -> io::Result<Self> {
        let words = serde_json::from_str::<Vec<WordItem>>(&fs::read_to_string(from_file)?).unwrap();
        Ok(Self {words})
    }

    /// 将内存中的words保存到一个文件中。如果文件存在则会被覆盖
    pub fn persist<P: AsRef<Path>>(&self, to_file: P) -> io::Result<()> {
        let contents = serde_json::to_string(&self.words)?;
        fs::write(to_file, contents)?;
        Ok(())
    }

    pub fn get_mut_words(&mut self) -> &mut Vec<WordItem> {
        &mut self.words
    }

    pub fn get_words(&self) -> &Vec<WordItem> {
        &self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_file() -> io::Result<()> {
        let words = get_words();
        let path = "target/test-words.json";
        fs::write(path, &serde_json::to_string(&words)?)?;

        let store = WordStore::from_file(path)?;
        assert_eq!(store.get_words(), &words);

        Ok(())
    }

    #[test]
    fn persist_to_file() -> io::Result<()> {
        let words = get_words();
        let store = WordStore::new(words.clone());
        let path = "target/test-words.json";
        store.persist(path)?;
        let file_content = fs::read_to_string(path)?;
        let persisted_words = serde_json::from_str::<Vec<WordItem>>(&file_content).unwrap();
        assert_eq!(persisted_words, words);
        Ok(())
    }

    
    fn get_words() -> Vec<WordItem> {
        // date1: Fri Jul 26 22:48:07 CST 2019
        // date2: Thu Sep 05 17:03:58 CST 2019
        // date3: Tue Sep 17 15:51:15 CST 2019
        let data = r#"[
        {
            "itemId": "9cef81095a2a7e35c169c990b37839eb",
            "bookId": "0",
            "bookName": "无标签",
            "word": "Accommodate",
            "trans": "vt. 容纳；使适应；供应；调解\nvi. 适应；调解",
            "phonetic": "[ə'kɒmədeɪt]",
            "modifiedTime": 1564152487000
        },
        {
            "itemId": "3223bb4547bd4bad4f17d31e207c6b3c",
            "bookId": "0",
            "bookName": "无标签",
            "word": "Acronym",
            "trans": "n. 首字母缩略词",
            "phonetic": "['ækrənɪm]",
            "modifiedTime": 1567674238000
        },
        {
            "itemId": "cf5b783279932d2d14d2241f0a166e76",
            "bookId": "0",
            "bookName": "无标签",
            "word": "Antenna",
            "trans": "n. [电讯] 天线；[动] 触角，[昆] 触须\nn. (Antenna)人名；(法)安泰纳",
            "phonetic": "[æn'tenə]",
            "modifiedTime": 1568706675000
        }]"#;

        serde_json::from_str(data).unwrap()
    }
}