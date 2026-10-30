use std::collections::HashMap;

use shuku_migrate_lrr::{Result, Source};
use shuku_store::{ArchiveMetadata, CompressionMethod, Page, Store};

pub const PREFIX: &str = "/home/koyomi/lanraragi/content";

/// 内存里的 LRR 数据。SCAN 的模式只用到 `?`，这里只实现它。
#[derive(Default)]
pub struct MemorySource {
    pub hashes: HashMap<String, HashMap<String, Vec<u8>>>,
    pub sorted_sets: HashMap<String, Vec<(Vec<u8>, f64)>>,
}

impl MemorySource {
    pub fn hash(&mut self, key: &str, fields: &[(&str, &[u8])]) {
        self.hashes.insert(
            key.into(),
            fields
                .iter()
                .map(|(field, value)| (field.to_string(), value.to_vec()))
                .collect(),
        );
    }
}

fn matches(pattern: &str, key: &str) -> bool {
    pattern.chars().count() == key.chars().count()
        && pattern
            .chars()
            .zip(key.chars())
            .all(|(p, k)| p == '?' || p == k)
}

impl Source for MemorySource {
    fn scan(&mut self, pattern: &str) -> Result<Vec<Vec<u8>>> {
        let keys = self.hashes.keys().chain(self.sorted_sets.keys());
        Ok(keys
            .filter(|key| matches(pattern, key))
            .map(|key| key.as_bytes().to_vec())
            .collect())
    }

    fn hash(&mut self, key: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
        Ok(self.hashes[std::str::from_utf8(key).unwrap()].clone())
    }

    fn sorted_set(&mut self, key: &[u8]) -> Result<Vec<(Vec<u8>, f64)>> {
        Ok(self.sorted_sets[std::str::from_utf8(key).unwrap()].clone())
    }
}

pub fn add_archive(store: &mut Store, path: &str, pages: usize) -> i64 {
    let metadata = ArchiveMetadata {
        path: path.into(),
        title: "扫描时的标题".into(),
        summary: String::new(),
        file_size: 1000,
        mtime: 1_700_000_001,
        quick_hash: "quick".into(),
        content_hash: None,
        date_added: 1_700_000_000,
        source_url: None,
        is_new: true,
        lrr_id: None,
    };
    let page = Page {
        entry_name: "1.jpg".into(),
        data_offset: 0,
        compressed_size: 1,
        size: 1,
        method: CompressionMethod::Stored,
        width: 1,
        height: 1,
    };
    store
        .save_archive(None, &metadata, &vec![page; pages])
        .unwrap()
}

/// 模拟被重复编码过一次的 UTF-8：把每个字节当成 Latin-1 字符再编码。
pub fn double_encoded(text: &str) -> Vec<u8> {
    text.bytes()
        .map(char::from)
        .collect::<String>()
        .into_bytes()
}
