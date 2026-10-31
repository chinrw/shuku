use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};
use shuku_index::ArchiveRecord;

use crate::{
    Archive, ArchiveListItem, ArchiveMetadata, Error, MIGRATIONS, Page, Progress, Result,
    ScanEntry, Tag, configure,
};

/// 只读连接。WAL 模式下读不挡写，写也不挡读；每个读取方法看到的是调用开始时已提交的数据。
pub struct Reader {
    pub(crate) connection: Connection,
}

impl Reader {
    /// 打开只读连接。数据库必须已经由 [`crate::Store::open`] 迁移到当前版本。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        configure(&connection)?;
        let found: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let supported = MIGRATIONS.len() as i64;
        if found != supported {
            return Err(Error::VersionMismatch { found, supported });
        }
        Ok(Self { connection })
    }

    pub fn scan_entries(&self) -> Result<Vec<ScanEntry>> {
        Ok(self
            .connection
            .prepare("SELECT id, path, file_size, mtime, quick_hash FROM archive ORDER BY id")?
            .query_map([], |row| {
                Ok(ScanEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    file_size: row.get(2)?,
                    mtime: row.get(3)?,
                    quick_hash: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// 还没有完整指纹、ID 大于 `after` 的归档，按 ID 从小到大最多 `limit` 本。
    /// 调用方用上一批最后一本的 ID 翻页，算不出指纹的归档不会被反复取到。
    pub fn missing_content_hashes(&self, after: i64, limit: usize) -> Result<Vec<ScanEntry>> {
        Ok(self
            .connection
            .prepare(
                "SELECT id, path, file_size, mtime, quick_hash FROM archive
                 WHERE content_hash IS NULL AND id > ?1 ORDER BY id LIMIT ?2",
            )?
            .query_map(params![after, limit as i64], |row| {
                Ok(ScanEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    file_size: row.get(2)?,
                    mtime: row.get(3)?,
                    quick_hash: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// 元数据、标签和进度从同一个 SQLite 读取快照中取得。
    pub fn archive(&self, id: i64) -> Result<Option<Archive>> {
        let transaction = self.connection.unchecked_transaction()?;
        let archive = transaction
            .query_row("SELECT * FROM archive WHERE id = ?1", [id], |row| {
                Ok(Archive {
                    id: row.get("id")?,
                    metadata: ArchiveMetadata {
                        path: row.get("path")?,
                        title: row.get("title")?,
                        summary: row.get("summary")?,
                        file_size: row.get("file_size")?,
                        mtime: row.get("mtime")?,
                        quick_hash: row.get("quick_hash")?,
                        content_hash: row.get("content_hash")?,
                        date_added: row.get("date_added")?,
                        source_url: row.get("source_url")?,
                        is_new: row.get("is_new")?,
                        lrr_id: row.get("lrr_id")?,
                    },
                    page_count: row.get("page_count")?,
                    tags: Vec::new(),
                    progress: None,
                })
            })
            .optional()?;
        let Some(mut archive) = archive else {
            return Ok(None);
        };
        archive.tags = transaction.prepare_cached(
            "SELECT tag.namespace, tag.value FROM tag JOIN archive_tag ON tag.id = archive_tag.tag_id
             WHERE archive_tag.archive_id = ?1 ORDER BY tag.full"
        )?.query_map([id], |row| Ok(Tag { namespace: row.get(0)?, value: row.get(1)? }))?
            .collect::<rusqlite::Result<_>>()?;
        archive.progress = transaction
            .query_row(
                "SELECT page, client_time, updated_at FROM progress WHERE archive_id = ?1",
                [id],
                read_progress,
            )
            .optional()?;
        transaction.commit()?;
        Ok(Some(archive))
    }

    /// 返回按 page_no 升序排列的清单；不存在的归档返回空清单。
    pub fn pages(&self, id: i64) -> Result<Vec<Page>> {
        let pages = self
            .connection
            .prepare_cached("SELECT pages FROM page_list WHERE archive_id = ?1")?
            .query_row([id], |row| row.get::<_, Vec<u8>>(0))
            .optional()?;
        Ok(match pages {
            Some(bytes) => postcard::from_bytes(&bytes)?,
            None => Vec::new(),
        })
    }

    pub fn find_by_path(&self, path: &str) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row("SELECT id FROM archive WHERE path = ?1", [path], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// 按输入 ID 的顺序返回列表项，跳过已不存在的归档。
    /// 调用方传入一页搜索结果，ID 数量受 API 的分页上限约束。
    /// 元数据和标签从同一个读取快照中取得。
    pub fn list_metadata(&self, ids: &[i64]) -> Result<Vec<ArchiveListItem>> {
        let placeholders = vec!["?"; ids.len()].join(",");
        let transaction = self.connection.unchecked_transaction()?;
        let mut statement = transaction.prepare(&format!(
            "SELECT a.id, a.title, a.page_count, a.quick_hash, a.content_hash, a.is_new,
                a.date_added, a.source_url, p.page, p.client_time, p.updated_at
             FROM archive a LEFT JOIN progress p ON p.archive_id = a.id
             WHERE a.id IN ({placeholders})"
        ))?;
        let mut items = statement
            .query_map(params_from_iter(ids), |row| {
                Ok(ArchiveListItem {
                    id: row.get("id")?,
                    title: row.get("title")?,
                    page_count: row.get("page_count")?,
                    quick_hash: row.get("quick_hash")?,
                    content_hash: row.get("content_hash")?,
                    is_new: row.get("is_new")?,
                    date_added: row.get("date_added")?,
                    source_url: row.get("source_url")?,
                    tags: Vec::new(),
                    progress: if row.get::<_, Option<u32>>("page")?.is_some() {
                        Some(read_progress(row)?)
                    } else {
                        None
                    },
                })
            })?
            .map(|row| row.map(|item| (item.id, item)))
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        let mut tags = transaction.prepare(&format!(
            "SELECT archive_tag.archive_id, tag.namespace, tag.value
             FROM archive_tag JOIN tag ON tag.id = archive_tag.tag_id
             WHERE archive_tag.archive_id IN ({placeholders})
             ORDER BY tag.full"
        ))?;
        let mut rows = tags.query(params_from_iter(ids))?;
        while let Some(row) = rows.next()? {
            if let Some(item) = items.get_mut(&row.get::<_, i64>(0)?) {
                item.tags.push(Tag {
                    namespace: row.get(1)?,
                    value: row.get(2)?,
                });
            }
        }
        Ok(ids.iter().filter_map(|id| items.remove(id)).collect())
    }

    /// 在同一读取快照中按 ID 流式导出，每次仅组装一本归档及其标签。
    /// 读取或回调出错时立即停止；调用方应丢弃已收到的部分结果。
    pub fn for_each_archive_record(
        &self,
        mut visit: impl FnMut(ArchiveRecord) -> Result<()>,
    ) -> Result<()> {
        let mut statement = self.connection.prepare(
            "SELECT a.id, a.title, a.date_added, t.full
             FROM archive a
             LEFT JOIN archive_tag at ON at.archive_id = a.id
             LEFT JOIN tag t ON t.id = at.tag_id
             ORDER BY a.id, at.tag_id",
        )?;
        let mut rows = statement.query([])?;
        let mut current: Option<ArchiveRecord> = None;
        while let Some(row) = rows.next()? {
            let id = row.get(0)?;
            if current.as_ref().map(|record| record.id) != Some(id) {
                if let Some(record) = current.take() {
                    visit(record)?;
                }
                current = Some(ArchiveRecord {
                    id,
                    title: row.get(1)?,
                    date_added: row.get(2)?,
                    tags: Vec::new(),
                });
            }
            if let (Some(record), Some(tag)) = (&mut current, row.get::<_, Option<String>>(3)?) {
                record.tags.push(tag);
            }
        }
        if let Some(record) = current {
            visit(record)?;
        }
        Ok(())
    }
}

fn read_progress(row: &rusqlite::Row<'_>) -> rusqlite::Result<Progress> {
    Ok(Progress {
        page: row.get("page")?,
        client_time: row.get("client_time")?,
        updated_at: row.get("updated_at")?,
    })
}
