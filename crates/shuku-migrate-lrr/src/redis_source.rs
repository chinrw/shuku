use std::collections::HashMap;

use crate::{Result, Source};

pub struct RedisSource {
    connection: redis::Connection,
}

impl RedisSource {
    /// 库编号写在 URL 的路径里，例如 `redis://127.0.0.1:6379/0`。
    pub fn connect(url: &str) -> Result<Self> {
        Ok(Self {
            connection: redis::Client::open(url)?.get_connection()?,
        })
    }
}

impl Source for RedisSource {
    fn scan(&mut self, pattern: &str) -> Result<Vec<Vec<u8>>> {
        let mut keys = Vec::new();
        let mut cursor = 0_u64;
        loop {
            let (next, batch): (u64, Vec<Vec<u8>>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(1000)
                .query(&mut self.connection)?;
            keys.extend(batch);
            if next == 0 {
                break;
            }
            cursor = next;
        }
        // SCAN 在遍历期间发生 rehash 时可能重复返回同一个键。
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    fn hash(&mut self, key: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
        Ok(redis::cmd("HGETALL").arg(key).query(&mut self.connection)?)
    }

    fn sorted_set(&mut self, key: &[u8]) -> Result<Vec<(Vec<u8>, f64)>> {
        Ok(redis::cmd("ZRANGE")
            .arg(key)
            .arg(0)
            .arg(-1)
            .arg("WITHSCORES")
            .query(&mut self.connection)?)
    }
}
