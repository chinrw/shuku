use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::{
    Archive, ArchiveMetadata, Error, Library, LibraryArchive, LibraryTag, MIGRATIONS, Page,
    Progress, Result, ScanEntry, Tag, configure,
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
        let archive = read_archive(&transaction, id)?;
        transaction.commit()?;
        Ok(archive)
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

    /// 一次读出搜索快照需要的全部数据：标签字典、每本的列表元数据和全部进度，
    /// 都来自同一个读取快照。
    pub fn library(&self) -> Result<Library> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut positions = HashMap::new();
        let tags = transaction
            .prepare("SELECT id, namespace, value, full FROM tag ORDER BY full")?
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    LibraryTag {
                        tag: Tag {
                            namespace: row.get(1)?,
                            value: row.get(2)?,
                        },
                        full: row.get(3)?,
                    },
                ))
            })?
            .enumerate()
            .map(|(position, row)| {
                let (id, tag) = row?;
                positions.insert(id, position as u32);
                Ok(tag)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut archives = transaction
            .prepare(
                "SELECT id, title, page_count, quick_hash, is_new, date_added, source_url
                 FROM archive ORDER BY id",
            )?
            .query_map([], |row| {
                Ok(LibraryArchive {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    page_count: row.get(2)?,
                    quick_hash: row.get(3)?,
                    is_new: row.get(4)?,
                    date_added: row.get(5)?,
                    source_url: row.get(6)?,
                    tags: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // 两边都按归档 ID 升序，归并时不必为每本查一次。
        let mut associations = transaction
            .prepare("SELECT archive_id, tag_id FROM archive_tag ORDER BY archive_id")?;
        let mut rows = associations.query([])?;
        let mut current = archives.iter_mut().peekable();
        while let Some(row) = rows.next()? {
            let archive_id: i64 = row.get(0)?;
            while current
                .peek()
                .is_some_and(|archive| archive.id < archive_id)
            {
                current.next();
            }
            if let Some(archive) = current
                .peek_mut()
                .filter(|archive| archive.id == archive_id)
                && let Some(position) = positions.get(&row.get::<_, i64>(1)?)
            {
                archive.tags.push(*position);
            }
        }
        drop(rows);
        drop(associations);
        for archive in &mut archives {
            archive.tags.sort_unstable();
        }
        let progress = read_all_progress(&transaction)?;
        transaction.commit()?;
        Ok(Library {
            tags,
            archives,
            progress,
        })
    }

    /// 全部归档的阅读进度。
    pub fn progress(&self) -> Result<HashMap<i64, Progress>> {
        read_all_progress(&self.connection)
    }
}

fn read_all_progress(connection: &Connection) -> Result<HashMap<i64, Progress>> {
    Ok(connection
        .prepare("SELECT archive_id, page, client_time, updated_at FROM progress")?
        .query_map([], |row| Ok((row.get(0)?, read_progress(row)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

fn read_progress(row: &rusqlite::Row<'_>) -> rusqlite::Result<Progress> {
    Ok(Progress {
        page: row.get("page")?,
        client_time: row.get("client_time")?,
        updated_at: row.get("updated_at")?,
    })
}

/// 不自己开事务，调用方要在一个事务里调用，元数据、标签和进度才来自同一个快照。
pub(crate) fn read_archive(connection: &Connection, id: i64) -> Result<Option<Archive>> {
    let archive = connection
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
    archive.tags = connection.prepare_cached(
        "SELECT tag.namespace, tag.value FROM tag JOIN archive_tag ON tag.id = archive_tag.tag_id
         WHERE archive_tag.archive_id = ?1 ORDER BY tag.full"
    )?.query_map([id], |row| Ok(Tag { namespace: row.get(0)?, value: row.get(1)? }))?
        .collect::<rusqlite::Result<_>>()?;
    archive.progress = connection
        .query_row(
            "SELECT page, client_time, updated_at FROM progress WHERE archive_id = ?1",
            [id],
            read_progress,
        )
        .optional()?;
    Ok(Some(archive))
}
