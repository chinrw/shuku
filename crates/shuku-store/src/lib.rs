//! Shuku 的 SQLite 存储层。

mod models;
mod store;

pub use models::{
    Archive, ArchiveListItem, ArchiveMetadata, CompressionMethod, Page, Progress, ScanEntry, Tag,
};
pub use store::Store;

use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};

const MIGRATIONS: &[&str] = &[include_str!("migrations/001_initial.sql")];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("数据库版本 {found} 比当前程序支持的 {supported} 新")]
    UnsupportedVersion { found: i64, supported: i64 },
    #[error("归档 {0} 不存在")]
    ArchiveNotFound(i64),
}

pub type Result<T> = std::result::Result<T, Error>;

/// 打开并迁移数据库。需要直接操作分类等表的调用者也使用此入口。
/// `:memory:` 可用于测试；SQLite 内存库的日志模式仍为 `memory`。
pub fn open_connection(path: impl AsRef<Path>) -> Result<Connection> {
    let mut connection = Connection::open(path)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let supported = MIGRATIONS.len() as i64;
    if version > supported {
        return Err(Error::UnsupportedVersion {
            found: version,
            supported,
        });
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", (index + 1) as i64)?;
    }
    transaction.commit()?;
    Ok(connection)
}
