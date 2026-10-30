use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};
use shuku_index::ArchiveRecord;

use crate::{
    Archive, ArchiveListItem, ArchiveMetadata, Error, Page, Progress, Result, Tag, open_connection,
};

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            connection: open_connection(path)?,
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
        transaction.execute("DELETE FROM page WHERE archive_id = ?1", [id])?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO page (archive_id, page_no, entry_name, data_offset, compressed_size,
                    size, method, width, height) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for (page_no, page) in pages.iter().enumerate() {
                insert.execute(params![
                    id,
                    page_no as u64,
                    page.entry_name,
                    page.data_offset,
                    page.compressed_size,
                    page.size,
                    page.method,
                    page.width,
                    page.height
                ])?;
            }
        }
        transaction.commit()?;
        Ok(id)
    }

    /// 标签按完整名称的 Unicode 小写形式去重，显示形式保留首次写入的大小写。
    pub fn replace_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM archive_tag WHERE archive_id = ?1", [id])?;
        {
            let mut insert_tag = transaction.prepare_cached(
                "INSERT INTO tag (namespace, value, full) VALUES (?1, ?2, ?3) ON CONFLICT(full) DO NOTHING"
            )?;
            let mut associate = transaction.prepare_cached(
                "INSERT INTO archive_tag (tag_id, archive_id) SELECT id, ?1 FROM tag WHERE full = ?2
                 ON CONFLICT(tag_id, archive_id) DO NOTHING"
            )?;
            for tag in tags {
                let full = if tag.namespace.is_empty() {
                    tag.value.to_lowercase()
                } else {
                    format!("{}:{}", tag.namespace, tag.value).to_lowercase()
                };
                insert_tag.execute(params![tag.namespace, tag.value, full])?;
                associate.execute(params![id, full])?;
            }
        }
        transaction.commit()?;
        Ok(())
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
        let mut statement = self
            .connection
            .prepare_cached("SELECT * FROM page WHERE archive_id = ?1 ORDER BY page_no")?;
        Ok(statement
            .query_map([id], |row| {
                Ok(Page {
                    entry_name: row.get("entry_name")?,
                    data_offset: row.get("data_offset")?,
                    compressed_size: row.get("compressed_size")?,
                    size: row.get("size")?,
                    method: row.get("method")?,
                    width: row.get("width")?,
                    height: row.get("height")?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
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
    pub fn list_metadata(&self, ids: &[i64]) -> Result<Vec<ArchiveListItem>> {
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut statement = self.connection.prepare(&format!(
            "SELECT a.id, a.title, a.page_count, a.quick_hash, a.content_hash, a.is_new,
                p.page, p.client_time, p.updated_at
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
                    progress: if row.get::<_, Option<u32>>("page")?.is_some() {
                        Some(read_progress(row)?)
                    } else {
                        None
                    },
                })
            })?
            .map(|row| row.map(|item| (item.id, item)))
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(ids.iter().filter_map(|id| items.remove(id)).collect())
    }

    /// 仅接受翻页时间严格更新的上报，返回是否写入。
    pub fn write_progress(&mut self, id: i64, page: u32, client_time: i64) -> Result<bool> {
        let transaction = self
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
