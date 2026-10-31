use std::ops::Deref;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params, params_from_iter};

use crate::{
    ArchiveMetadata, Error, LrrArchive, LrrCategory, LrrTankoubon, Page, Reader, Result, ScanEntry,
    Tag, open_connection,
};

/// 读写连接，打开时完成迁移。读方法通过 [`Deref`] 来自 [`Reader`]，
/// 和写入用同一个连接，所以能读到自己刚提交的数据。
pub struct Store {
    reader: Reader,
}

impl Deref for Store {
    type Target = Reader;

    fn deref(&self) -> &Reader {
        &self.reader
    }
}

impl Store {
    /// 只在归档的文件身份仍与 `entry` 一致时写入完整指纹，返回是否写入。
    /// 指纹在锁外计算，期间扫描可能已经换了文件，那时算出的指纹已经过时。
    pub fn set_content_hash(&mut self, entry: &ScanEntry, hash: &str) -> Result<bool> {
        Ok(self.reader.connection.execute(
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

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            reader: Reader {
                connection: open_connection(path)?,
            },
        })
    }

    /// `None` 插入新归档，`Some(id)` 更新已有归档并保留其身份。
    /// 页面清单整体替换，page_no 和 page_count 由清单顺序和长度生成。
    pub fn save_archive(
        &mut self,
        id: Option<i64>,
        metadata: &ArchiveMetadata,
        pages: &[Page],
    ) -> Result<i64> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        transaction.commit()?;
        Ok(id)
    }

    /// 标签按完整名称的 Unicode 小写形式去重，显示形式保留首次写入的大小写。
    pub fn replace_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        set_archive_tags(&transaction, id, tags)?;
        transaction.commit()?;
        Ok(())
    }

    /// 用 LRR 的元数据和标签覆盖一本已入库的归档。
    pub fn import_lrr_archive(&mut self, id: i64, lrr: &LrrArchive) -> Result<()> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        set_archive_tags(&transaction, id, &lrr.tags)?;
        transaction.commit()?;
        Ok(())
    }

    /// 按 LRR ID 新建或更新分类，归档成员整体替换。
    pub fn import_lrr_category(&mut self, category: &LrrCategory) -> Result<()> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        transaction.commit()?;
        Ok(())
    }

    /// 按 LRR ID 新建或更新 Tankoubon，标签和归档成员整体替换，position 从 1 开始。
    pub fn import_lrr_tankoubon(&mut self, tankoubon: &LrrTankoubon) -> Result<()> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
                tag.execute([id, ensure_tag(&transaction, value)?])?;
            }
            let mut archive = transaction.prepare_cached(
                "INSERT INTO tankoubon_archive (tankoubon_id, archive_id, position)
                 VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            )?;
            for (index, member) in tankoubon.archives.iter().enumerate() {
                archive.execute(params![id, member, index as i64 + 1])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// 仅接受翻页时间严格更新的上报，返回是否写入。
    pub fn write_progress(&mut self, id: i64, page: u32, client_time: i64) -> Result<bool> {
        let transaction = self
            .reader
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_archive(&transaction, id)?;
        let accepted = transaction.execute(
            "INSERT INTO progress (archive_id, page, client_time, updated_at)
             VALUES (?1, ?2, ?3, CAST(unixepoch('subsec') * 1000 AS INTEGER))
             ON CONFLICT(archive_id) DO UPDATE SET page = excluded.page,
                client_time = excluded.client_time, updated_at = excluded.updated_at
             WHERE excluded.client_time > progress.client_time",
            params![id, page, client_time],
        )? > 0;
        transaction.commit()?;
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
