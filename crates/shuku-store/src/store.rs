use std::ops::Deref;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params, params_from_iter};

use crate::reader::read_archive;
use crate::{
    Archive, ArchiveMetadata, Error, LrrArchive, LrrCategory, LrrTankoubon, Page, Reader, Result,
    ScanEntry, Tag, open_connection,
};

/// 读写连接，打开时完成迁移。读方法通过 [`Deref`] 来自 [`Reader`]，
/// 和写入用同一个连接，所以能读到自己刚提交的数据。
pub struct Store {
    reader: Reader,
}

/// 一个写事务里的操作。方法出错时不回滚已经做过的修改，由调用方决定：
/// 用 `?` 把错误传出 [`Store::write`] 回滚整个事务，或者用 [`Writer::savepoint`] 只撤销一步。
pub struct Writer<'a> {
    connection: &'a Connection,
}

impl Deref for Store {
    type Target = Reader;

    fn deref(&self) -> &Reader {
        &self.reader
    }
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            reader: Reader {
                connection: open_connection(path)?,
            },
        })
    }

    /// 在一个 IMMEDIATE 事务里执行 `work`，返回 `Ok` 时提交，出错或 panic 时整个回滚。
    pub fn write<T, E: From<Error>>(
        &mut self,
        work: impl FnOnce(&mut Writer<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(Error::from)?;
        let value = work(&mut Writer {
            connection: &transaction,
        })?;
        transaction.commit().map_err(Error::from)?;
        Ok(value)
    }

    /// 关掉提交时的自动 checkpoint，改由调用方在空闲时调用 [`Store::checkpoint`]。
    /// 自动 checkpoint 在提交它的那次写入里同步执行，btrfs 上的 fsync 会让那次写入停顿几十毫秒。
    pub fn disable_auto_checkpoint(&self) -> Result<()> {
        self.reader
            .connection
            .pragma_update(None, "wal_autocheckpoint", 0)?;
        Ok(())
    }

    /// 把 WAL 里已提交的页写回数据库文件，返回是否全部写回。PASSIVE 模式不等读者，
    /// 比仍在读的快照更新的页留到下次。
    pub fn checkpoint(&self) -> Result<bool> {
        let (log, checkpointed): (i64, i64) =
            self.reader
                .connection
                .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                    Ok((row.get(1)?, row.get(2)?))
                })?;
        Ok(checkpointed == log)
    }

    pub fn save_archive(
        &mut self,
        id: Option<i64>,
        metadata: &ArchiveMetadata,
        pages: &[Page],
    ) -> Result<i64> {
        self.write(|writer| writer.save_archive(id, metadata, pages))
    }

    pub fn replace_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        self.write(|writer| writer.replace_tags(id, tags))
    }

    pub fn import_lrr_archive(&mut self, id: i64, lrr: &LrrArchive) -> Result<()> {
        self.write(|writer| writer.import_lrr_archive(id, lrr))
    }

    pub fn import_lrr_category(&mut self, category: &LrrCategory) -> Result<()> {
        self.write(|writer| writer.import_lrr_category(category))
    }

    pub fn import_lrr_tankoubon(&mut self, tankoubon: &LrrTankoubon) -> Result<()> {
        self.write(|writer| writer.import_lrr_tankoubon(tankoubon))
    }

    pub fn write_progress(&mut self, id: i64, page: u32, client_time: i64) -> Result<bool> {
        self.write(|writer| writer.write_progress(id, page, client_time))
    }

    pub fn set_content_hash(&mut self, entry: &ScanEntry, hash: &str) -> Result<bool> {
        self.write(|writer| writer.set_content_hash(entry, hash))
    }
}

impl Writer<'_> {
    /// 在保存点里执行 `work`，`work` 出错时只撤销这一步，事务里其余的修改保留，
    /// 错误放在内层返回。外层的错误表示整个事务已经不能用了，调用方必须把它传出
    /// [`Store::write`]：SQLite 遇到 I/O 错误或磁盘写满时会自己回滚整个事务并丢掉全部保存点，
    /// 这时再开保存点会在事务外自动提交，失败的批次就会留下一部分写入。
    pub fn savepoint<T, E>(
        &mut self,
        work: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> Result<std::result::Result<T, E>> {
        self.require_transaction()?;
        self.connection.execute_batch("SAVEPOINT step")?;
        let outcome = work(self);
        self.require_transaction()?;
        match outcome {
            Ok(_) => self.connection.execute_batch("RELEASE step")?,
            Err(_) => self
                .connection
                .execute_batch("ROLLBACK TO step; RELEASE step")?,
        }
        Ok(outcome)
    }

    fn require_transaction(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::TransactionAborted);
        }
        Ok(())
    }

    /// 读到的是本事务里已经做过的修改。
    pub fn archive(&self, id: i64) -> Result<Option<Archive>> {
        read_archive(self.connection, id)
    }

    /// 只在归档的文件身份仍与 `entry` 一致时写入完整指纹，返回是否写入。
    /// 指纹在锁外计算，期间扫描可能已经换了文件，那时算出的指纹已经过时。
    pub fn set_content_hash(&mut self, entry: &ScanEntry, hash: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE archive SET content_hash = ?1
             WHERE id = ?2 AND path = ?3 AND file_size = ?4 AND mtime = ?5 AND quick_hash = ?6
                AND content_hash IS NULL",
            params![
                hash,
                entry.id,
                entry.path,
                entry.file_size,
                entry.mtime,
                entry.quick_hash
            ],
        )? > 0)
    }

    /// `None` 插入新归档，`Some(id)` 更新已有归档并保留其身份。
    /// 页面清单整体替换，page_no 和 page_count 由清单顺序和长度生成。
    pub fn save_archive(
        &mut self,
        id: Option<i64>,
        metadata: &ArchiveMetadata,
        pages: &[Page],
    ) -> Result<i64> {
        let transaction = self.connection;
        let values = params![
            metadata.path,
            metadata.title,
            metadata.summary,
            metadata.file_size,
            metadata.mtime,
            metadata.quick_hash,
            metadata.content_hash,
            pages.len() as u64,
            metadata.date_added,
            metadata.source_url,
            metadata.is_new,
            metadata.lrr_id,
        ];
        let id = match id {
            None => {
                transaction.execute(
                    "INSERT INTO archive (path, title, summary, file_size, mtime, quick_hash,
                        content_hash, page_count, date_added, source_url, is_new, lrr_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    values,
                )?;
                transaction.last_insert_rowid()
            }
            Some(id) => {
                // UPDATE 不会插入新行，已删除的 ID 因此不会被重新使用。
                let updated = transaction.execute(
                    "UPDATE archive SET path = ?1, title = ?2, summary = ?3, file_size = ?4,
                        mtime = ?5, quick_hash = ?6, content_hash = ?7, page_count = ?8,
                        date_added = ?9, source_url = ?10, is_new = ?11, lrr_id = ?12
                     WHERE id = ?13",
                    params_from_iter(values.iter().copied().chain([&id as &dyn rusqlite::ToSql])),
                )?;
                if updated == 0 {
                    return Err(Error::ArchiveNotFound(id));
                }
                id
            }
        };
        transaction.execute(
            "INSERT INTO page_list (archive_id, pages) VALUES (?1, ?2)
             ON CONFLICT(archive_id) DO UPDATE SET pages = excluded.pages",
            params![id, postcard::to_stdvec(pages)?],
        )?;
        Ok(id)
    }

    /// 标签按完整名称的 Unicode 小写形式去重，显示形式保留首次写入的大小写。
    pub fn replace_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        let transaction = self.connection;
        set_archive_tags(transaction, id, tags)?;
        Ok(())
    }

    /// 用 LRR 的元数据和标签覆盖一本已入库的归档。
    pub fn import_lrr_archive(&mut self, id: i64, lrr: &LrrArchive) -> Result<()> {
        let transaction = self.connection;
        let updated = transaction.execute(
            "UPDATE archive SET title = ?1, summary = ?2, date_added = COALESCE(?3, date_added),
                source_url = ?4, is_new = ?5, lrr_id = ?6
             WHERE id = ?7",
            params![
                lrr.title,
                lrr.summary,
                lrr.date_added,
                lrr.source_url,
                lrr.is_new,
                lrr.lrr_id,
                id
            ],
        )?;
        if updated == 0 {
            return Err(Error::ArchiveNotFound(id));
        }
        set_archive_tags(transaction, id, &lrr.tags)?;
        Ok(())
    }

    /// 按 LRR ID 新建或更新分类，归档成员整体替换。
    pub fn import_lrr_category(&mut self, category: &LrrCategory) -> Result<()> {
        let transaction = self.connection;
        let id: i64 = transaction.query_row(
            "INSERT INTO category (lrr_id, name, search, pinned) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(lrr_id) DO UPDATE SET name = excluded.name, search = excluded.search,
                pinned = excluded.pinned
             RETURNING id",
            params![
                category.lrr_id,
                category.name,
                category.search,
                category.pinned
            ],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM category_archive WHERE category_id = ?1", [id])?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO category_archive (category_id, archive_id) VALUES (?1, ?2)
                 ON CONFLICT DO NOTHING",
            )?;
            for archive in &category.archives {
                insert.execute([id, *archive])?;
            }
        }
        Ok(())
    }

    /// 按 LRR ID 新建或更新 Tankoubon，标签和归档成员整体替换，position 从 1 开始。
    pub fn import_lrr_tankoubon(&mut self, tankoubon: &LrrTankoubon) -> Result<()> {
        let transaction = self.connection;
        let id: i64 = transaction.query_row(
            "INSERT INTO tankoubon (lrr_id, name, summary) VALUES (?1, ?2, ?3)
             ON CONFLICT(lrr_id) DO UPDATE SET name = excluded.name, summary = excluded.summary
             RETURNING id",
            params![tankoubon.lrr_id, tankoubon.name, tankoubon.summary],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM tankoubon_tag WHERE tankoubon_id = ?1", [id])?;
        transaction.execute(
            "DELETE FROM tankoubon_archive WHERE tankoubon_id = ?1",
            [id],
        )?;
        {
            let mut tag = transaction.prepare_cached(
                "INSERT INTO tankoubon_tag (tankoubon_id, tag_id) VALUES (?1, ?2)
                 ON CONFLICT DO NOTHING",
            )?;
            for value in &tankoubon.tags {
                tag.execute([id, ensure_tag(transaction, value)?])?;
            }
            let mut archive = transaction.prepare_cached(
                "INSERT INTO tankoubon_archive (tankoubon_id, archive_id, position)
                 VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            )?;
            for (index, member) in tankoubon.archives.iter().enumerate() {
                archive.execute(params![id, member, index as i64 + 1])?;
            }
        }
        Ok(())
    }

    /// 仅接受翻页时间严格更新的上报，返回是否写入。
    pub fn write_progress(&mut self, id: i64, page: u32, client_time: i64) -> Result<bool> {
        let transaction = self.connection;
        require_archive(transaction, id)?;
        let accepted = transaction.execute(
            "INSERT INTO progress (archive_id, page, client_time, updated_at)
             VALUES (?1, ?2, ?3, CAST(unixepoch('subsec') * 1000 AS INTEGER))
             ON CONFLICT(archive_id) DO UPDATE SET page = excluded.page,
                client_time = excluded.client_time, updated_at = excluded.updated_at
             WHERE excluded.client_time > progress.client_time",
            params![id, page, client_time],
        )? > 0;
        Ok(accepted)
    }
}

/// 迁移 003 的 Rust 步骤：把每页一行的 page 表按归档打包成 BLOB，再删掉旧表。
pub(crate) fn pack_page_lists(connection: &Connection) -> Result<()> {
    let mut rows = connection.prepare(
        "SELECT archive_id, entry_name, data_offset, compressed_size, size, method, width, height
         FROM page ORDER BY archive_id, page_no",
    )?;
    let mut rows = rows.query([])?;
    let mut insert =
        connection.prepare("INSERT INTO page_list (archive_id, pages) VALUES (?1, ?2)")?;
    let mut current: Option<(i64, Vec<Page>)> = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if current.as_ref().map(|(current, _)| *current) != Some(id) {
            if let Some((id, pages)) = current.take() {
                insert.execute(params![id, postcard::to_stdvec(&pages)?])?;
            }
            current = Some((id, Vec::new()));
        }
        if let Some((_, pages)) = &mut current {
            pages.push(Page {
                entry_name: row.get(1)?,
                data_offset: row.get(2)?,
                compressed_size: row.get(3)?,
                size: row.get(4)?,
                method: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
            });
        }
    }
    if let Some((id, pages)) = current {
        insert.execute(params![id, postcard::to_stdvec(&pages)?])?;
    }
    // 没有页面的归档在旧表里没有行，也补上一个空清单，让每本归档都有一行。
    connection.execute(
        "INSERT INTO page_list (archive_id, pages)
         SELECT id, ?1 FROM archive WHERE id NOT IN (SELECT archive_id FROM page_list)",
        [postcard::to_stdvec::<[Page]>(&[])?],
    )?;
    connection.execute_batch("DROP TABLE page")?;
    Ok(())
}

fn set_archive_tags(connection: &Connection, id: i64, tags: &[Tag]) -> Result<()> {
    connection.execute("DELETE FROM archive_tag WHERE archive_id = ?1", [id])?;
    let mut associate = connection.prepare_cached(
        "INSERT INTO archive_tag (tag_id, archive_id) VALUES (?1, ?2)
         ON CONFLICT(tag_id, archive_id) DO NOTHING",
    )?;
    for tag in tags {
        associate.execute([ensure_tag(connection, tag)?, id])?;
    }
    Ok(())
}

/// 返回标签 ID，没有时新建。冲突时的空更新只是为了让 RETURNING 在已有标签上也返回 ID。
fn ensure_tag(connection: &Connection, tag: &Tag) -> Result<i64> {
    let full = if tag.namespace.is_empty() {
        tag.value.to_lowercase()
    } else {
        format!("{}:{}", tag.namespace, tag.value).to_lowercase()
    };
    Ok(connection
        .prepare_cached(
            "INSERT INTO tag (namespace, value, full) VALUES (?1, ?2, ?3)
             ON CONFLICT(full) DO UPDATE SET full = excluded.full
             RETURNING id",
        )?
        .query_row(params![tag.namespace, tag.value, full], |row| row.get(0))?)
}

fn require_archive(connection: &Connection, id: i64) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM archive WHERE id = ?1)",
        [id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(Error::ArchiveNotFound(id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArchiveMetadata, Error};

    fn metadata(path: &str) -> ArchiveMetadata {
        ArchiveMetadata {
            path: path.into(),
            title: path.into(),
            summary: String::new(),
            file_size: 1,
            mtime: 1,
            quick_hash: "quick".into(),
            content_hash: None,
            date_added: 1,
            source_url: None,
            is_new: true,
            lrr_id: None,
        }
    }

    #[test]
    fn a_transaction_rolled_back_by_sqlite_fails_the_whole_batch() {
        let mut store = Store::open(":memory:").unwrap();
        let result = store.write(|writer| {
            writer.save_archive(None, &metadata("before.cbz"), &[])?;
            // 模拟 SQLite 因 I/O 错误自行回滚：事务和全部保存点都没了。
            let aborted = writer.savepoint(|writer| {
                writer.connection.execute_batch("ROLLBACK")?;
                Err::<(), Error>(Error::ArchiveNotFound(0))
            });
            assert!(matches!(aborted, Err(Error::TransactionAborted)));
            assert!(matches!(
                writer.savepoint(|writer| writer.save_archive(None, &metadata("after.cbz"), &[])),
                Err(Error::TransactionAborted)
            ));
            aborted?.map(|_| ())
        });
        assert!(result.is_err());
        assert_eq!(store.find_by_path("before.cbz").unwrap(), None);
        assert_eq!(store.find_by_path("after.cbz").unwrap(), None);
    }
}
