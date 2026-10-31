use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::{Reader, Result};

/// 最多留这么多空闲连接。并发超过时按需新建，用完多出来的直接关掉。
const MAX_IDLE: usize = 16;

/// 只读连接池。连接按需打开，用完放回，下次复用它的页缓存和预编译语句。
pub struct ReaderPool {
    path: PathBuf,
    idle: Mutex<Vec<Reader>>,
}

impl ReaderPool {
    /// 不在这里打开连接；数据库要先由 [`crate::Store::open`] 完成迁移。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            idle: Mutex::new(Vec::new()),
        }
    }

    pub fn get(&self) -> Result<PooledReader<'_>> {
        // 空闲列表只有压入和弹出，持锁的线程 panic 也不会让它处于不一致的状态。
        let idle = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let reader = match idle {
            Some(reader) => reader,
            None => Reader::open(&self.path)?,
        };
        Ok(PooledReader {
            pool: self,
            reader: Some(reader),
        })
    }
}

pub struct PooledReader<'a> {
    pool: &'a ReaderPool,
    reader: Option<Reader>,
}

impl Deref for PooledReader<'_> {
    type Target = Reader;

    fn deref(&self) -> &Reader {
        self.reader.as_ref().expect("连接只在 drop 时取走")
    }
}

impl Drop for PooledReader<'_> {
    fn drop(&mut self) {
        let mut idle = self
            .pool
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if idle.len() < MAX_IDLE {
            idle.extend(self.reader.take());
        }
    }
}
