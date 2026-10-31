//! Shuku 的 SQLite 存储层。

mod models;
mod pool;
mod reader;
mod store;

pub use models::{
    Archive, ArchiveMetadata, Category, CompressionMethod, DuplicateFile, Library, LibraryArchive,
    LibraryStats, LibraryTag, LrrArchive, LrrCategory, LrrPending, LrrTankoubon, Page, Progress,
    ScanEntry, Tag,
};
pub use pool::{PooledReader, ReaderPool};
pub use reader::Reader;
pub use store::{Store, Writer};

use std::{path::Path, time::Duration};

use rusqlite::{Connection, TransactionBehavior};

/// 映射进内存的上限。bundled 的 SQLite 编译时把它限制在 0x7fff0000 字节，设得更大也只按这个值生效；
/// 30 万本的库约 550 MiB，远在上限之内。
const MMAP_SIZE: i64 = 0x7fff_0000;
/// 每个连接的页缓存，负数的单位是 KiB。
const CACHE_SIZE_KIB: i64 = 64 * 1024;
/// 另一个连接正在写时，等这么久再报 SQLITE_BUSY。服务端运行时迁移工具和测试会从别的连接写入。
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// 每项迁移先执行 SQL，再执行可选的 Rust 步骤。SQL 做不到的数据转换放在 Rust 步骤里。
type Migration = (&'static str, Option<fn(&Connection) -> Result<()>>);

const MIGRATIONS: &[Migration] = &[
    (include_str!("migrations/001_initial.sql"), None),
    (include_str!("migrations/002_lrr_ids.sql"), None),
    (
        include_str!("migrations/003_page_list.sql"),
        Some(store::pack_page_lists),
    ),
    (include_str!("migrations/004_lrr_pending.sql"), None),
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("数据库版本 {found} 比当前程序支持的 {supported} 新")]
    UnsupportedVersion { found: i64, supported: i64 },
    #[error("数据库版本 {found} 与当前程序的 {supported} 不一致，需要先由读写连接完成迁移")]
    VersionMismatch { found: i64, supported: i64 },
    #[error("归档 {0} 不存在")]
    ArchiveNotFound(i64),
    #[error("SQLite 已经回滚了整个事务")]
    TransactionAborted,
    #[error("页面清单编码错误：{0}")]
    PageList(#[from] postcard::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// 打开并迁移数据库。Store 还没有读取分类和 Tankoubon 的接口，测试用它直接查表。
/// `:memory:` 可用于测试；SQLite 内存库的日志模式仍为 `memory`。
pub fn open_connection(path: impl AsRef<Path>) -> Result<Connection> {
    let mut connection = Connection::open(path)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    configure(&connection)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let supported = MIGRATIONS.len() as i64;
    if version > supported {
        return Err(Error::UnsupportedVersion {
            found: version,
            supported,
        });
    }
    for (index, (sql, step)) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        transaction.execute_batch(sql)?;
        if let Some(step) = step {
            step(&transaction)?;
        }
        transaction.pragma_update(None, "user_version", (index + 1) as i64)?;
    }
    transaction.commit()?;
    Ok(connection)
}

/// 读写连接和只读连接共用的性能设置。在 30 万本的合成库上，mmap 让 8 线程并发取列表
/// 从每秒约 1900 次升到 2 万次，冷启动读出全部记录从 3.4 s 降到 1.4 s。
fn configure(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "mmap_size", MMAP_SIZE)?;
    connection.pragma_update(None, "cache_size", -CACHE_SIZE_KIB)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}
