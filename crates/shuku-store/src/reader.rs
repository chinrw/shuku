use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::{
    Archive, ArchiveMetadata, BrokenArchive, Category, DeviceToken, DuplicateFile, Error, Library,
    LibraryArchive, LibraryStats, LibraryTag, MIGRATIONS, Page, Progress, Result, ScanEntry,
    Series, Tag, configure,
};

/// 只读连接。WAL 模式下读不挡写，写也不挡读；每个读取方法看到的是调用开始时已提交的数据。
/// [`crate::Store`] 的写连接也包在这里面。
pub struct Reader {
    /// 关闭要和打开串行，见 [`crate::lifecycle`]，所以由 `Drop` 自己关。
    pub(crate) connection: ManuallyDrop<Connection>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _lifecycle = crate::lifecycle();
        // SAFETY: 只在这里取走一次，之后 `self` 不再被使用。
        unsafe { ManuallyDrop::drop(&mut self.connection) };
    }
}

impl Reader {
    pub(crate) fn wrap(connection: Connection) -> Self {
        Self {
            connection: ManuallyDrop::new(connection),
        }
    }

    /// 打开只读连接。数据库必须已经由 [`crate::Store::open`] 迁移到当前版本。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let _lifecycle = crate::lifecycle();
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
        Ok(Self::wrap(connection))
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
        read_pages(&self.connection, id)
    }

    /// 归档和它的页面清单来自同一个读取快照。页面 URL 里的快速指纹取自归档，
    /// 分两次读的话，扫描在中间提交时会把旧指纹配上新清单。
    pub fn archive_with_pages(&self, id: i64) -> Result<Option<(Archive, Vec<Page>)>> {
        let transaction = self.connection.unchecked_transaction()?;
        let archive = match read_archive(&transaction, id)? {
            Some(archive) => Some((archive, read_pages(&transaction, id)?)),
            None => None,
        };
        transaction.commit()?;
        Ok(archive)
    }

    pub fn find_by_path(&self, path: &str) -> Result<Option<i64>> {
        find_by_path(&self.connection, path)
    }

    /// 全部设备令牌，按 ID 升序。
    pub fn device_tokens(&self) -> Result<Vec<DeviceToken>> {
        Ok(self
            .connection
            .prepare("SELECT id, name, created_at, last_used_at FROM device_token ORDER BY id")?
            .query_map([], device_token)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 摘要对应的令牌，已经吊销或者从来没有时返回 `None`。
    pub fn device_token_by_hash(&self, hash: &str) -> Result<Option<DeviceToken>> {
        Ok(self
            .connection
            .prepare_cached(
                "SELECT id, name, created_at, last_used_at FROM device_token WHERE hash = ?1",
            )?
            .query_row([hash], device_token)
            .optional()?)
    }

    /// 一组设置。每个功能一个键（例如 `scan`），值是那组设置的 JSON；没存过时返回 `None`。
    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        read_setting(&self.connection, key)
    }

    /// 暂存着、等扫描到时套用的 LRR 记录数。
    pub fn lrr_pending_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT count(*) FROM lrr_pending", [], |row| row.get(0))?)
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
        let names = read_series_names(&transaction)?;
        let mut archives = transaction
            .prepare(
                "SELECT id, title, page_count, quick_hash, is_new, date_added, source_url,
                    series_id, volume
                 FROM archive ORDER BY id",
            )?
            .query_map([], |row| {
                let series = row.get::<_, Option<i64>>(7)?;
                let volume = row.get::<_, Option<f64>>(8)?.unwrap_or(0.0);
                Ok(LibraryArchive {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    page_count: row.get(2)?,
                    quick_hash: row.get(3)?,
                    is_new: row.get(4)?,
                    date_added: row.get(5)?,
                    source_url: row.get(6)?,
                    tags: Vec::new(),
                    series: series
                        .filter(|id| names.contains_key(id))
                        .map(|id| (id, volume)),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let series = group_series(names, &archives);
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
            // 逐个压入时容量会翻倍到 16，30 万本要多占几 MiB。
            archive.tags.shrink_to_fit();
        }
        let progress = read_all_progress(&transaction)?;
        let categories = read_categories(&transaction)?;
        transaction.commit()?;
        Ok(Library {
            tags,
            archives,
            progress,
            categories,
            series,
        })
    }

    /// 还不属于任何系列的归档的 ID 和路径，按 ID 升序。
    pub fn archives_without_series(&self) -> Result<Vec<(i64, String)>> {
        Ok(self
            .connection
            .prepare("SELECT id, path FROM archive WHERE series_id IS NULL ORDER BY id")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// 一本归档所属的系列，只有一卷的系列不算。
    pub fn archive_series(&self, id: i64) -> Result<Option<Series>> {
        let series = self
            .connection
            .prepare_cached("SELECT series_id FROM archive WHERE id = ?1")?
            .query_row([id], |row| row.get::<_, Option<i64>>(0))
            .optional()?
            .flatten();
        let Some(series) = series else {
            return Ok(None);
        };
        let name: String = self.connection.query_row(
            "SELECT name FROM series WHERE id = ?1",
            [series],
            |row| row.get(0),
        )?;
        let volumes = self
            .connection
            .prepare_cached("SELECT id FROM archive WHERE series_id = ?1 ORDER BY volume, id")?
            .query_map([series], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok((volumes.len() >= 2).then_some(Series {
            id: series,
            name,
            volumes,
        }))
    }

    /// 全部分类和静态分类的成员，置顶在前，再按名字。
    pub fn categories(&self) -> Result<Vec<Category>> {
        read_categories(&self.connection)
    }

    /// 一本归档所在的静态分类，按分类 ID 升序。
    pub fn archive_categories(&self, id: i64) -> Result<Vec<i64>> {
        Ok(self
            .connection
            .prepare_cached(
                "SELECT category_id FROM category_archive WHERE archive_id = ?1
                 ORDER BY category_id",
            )?
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 全部归档的阅读进度。
    pub fn progress(&self) -> Result<HashMap<i64, Progress>> {
        read_all_progress(&self.connection)
    }

    /// 控制台概览用的计数，都来自同一个读取快照。每项是一次全表聚合，
    /// 30 万本时合计在几十毫秒内，只在控制台打开时请求。
    pub fn stats(&self) -> Result<LibraryStats> {
        Ok(self.connection.query_row(
            "SELECT (SELECT count(*) FROM archive),
                    (SELECT coalesce(sum(page_count), 0) FROM archive),
                    (SELECT coalesce(sum(file_size), 0) FROM archive),
                    (SELECT count(*) FROM archive WHERE content_hash IS NOT NULL),
                    (SELECT count(*) FROM archive WHERE lrr_id IS NOT NULL),
                    (SELECT count(*) FROM progress),
                    (SELECT count(*) FROM lrr_pending),
                    (SELECT count(*) FROM category),
                    (SELECT count(*) FROM tankoubon),
                    (SELECT count(*) FROM archive WHERE broken_pages > 0)",
            [],
            |row| {
                Ok(LibraryStats {
                    archives: row.get(0)?,
                    pages: row.get(1)?,
                    bytes: row.get(2)?,
                    hashed: row.get(3)?,
                    lrr_imported: row.get(4)?,
                    with_progress: row.get(5)?,
                    lrr_pending: row.get(6)?,
                    categories: row.get(7)?,
                    tankoubons: row.get(8)?,
                    with_broken_pages: row.get(9)?,
                })
            },
        )?)
    }

    /// 有坏页的归档，按路径排序。返回总数和前 `limit` 本，每本带坏页的条目名。
    pub fn broken_archives(&self, limit: usize) -> Result<(u64, Vec<BrokenArchive>)> {
        let total: u64 = self.connection.query_row(
            "SELECT count(*) FROM archive WHERE broken_pages > 0",
            [],
            |row| row.get(0),
        )?;
        let rows = self
            .connection
            .prepare(
                "SELECT id, path, title FROM archive WHERE broken_pages > 0 ORDER BY path LIMIT ?1",
            )?
            .query_map([limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<(i64, String, String)>>>()?;
        let mut archives = Vec::with_capacity(rows.len());
        for (id, path, title) in rows {
            let pages = read_pages(&self.connection, id)?
                .into_iter()
                .filter(|page| page.width == 0)
                .map(|page| page.entry_name)
                .collect();
            archives.push(BrokenArchive {
                id,
                path,
                title,
                pages,
            });
        }
        Ok((total, archives))
    }

    /// 完整指纹相同的归档，每组按路径排序，组按指纹排序。返回组的总数和前 `limit` 组。
    /// 没有指纹索引，每次全表分组；只在控制台的问题页按需请求。
    pub fn duplicates(&self, limit: usize) -> Result<(u64, Vec<Vec<DuplicateFile>>)> {
        let total: u64 = self.connection.query_row(
            "SELECT count(*) FROM (SELECT 1 FROM archive WHERE content_hash IS NOT NULL
                GROUP BY content_hash HAVING count(*) > 1)",
            [],
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(
            "WITH groups AS (
                SELECT content_hash FROM archive WHERE content_hash IS NOT NULL
                GROUP BY content_hash HAVING count(*) > 1 ORDER BY content_hash LIMIT ?1
             )
             SELECT archive.content_hash, archive.id, archive.path, archive.title, archive.file_size
             FROM archive JOIN groups ON archive.content_hash = groups.content_hash
             ORDER BY archive.content_hash, archive.path",
        )?;
        let mut rows = statement.query([limit as i64])?;
        let mut groups: Vec<Vec<DuplicateFile>> = Vec::new();
        let mut current: Option<String> = None;
        while let Some(row) = rows.next()? {
            let hash: String = row.get(0)?;
            let file = DuplicateFile {
                id: row.get(1)?,
                path: row.get(2)?,
                title: row.get(3)?,
                file_size: row.get(4)?,
            };
            match groups.last_mut() {
                Some(group) if current.as_deref() == Some(hash.as_str()) => group.push(file),
                _ => {
                    groups.push(vec![file]);
                    current = Some(hash);
                }
            }
        }
        Ok((total, groups))
    }
}

/// 至少有两卷的系列的名字。
fn read_series_names(connection: &Connection) -> Result<HashMap<i64, String>> {
    Ok(connection
        .prepare(
            "SELECT series.id, series.name FROM series
             JOIN archive ON archive.series_id = series.id
             GROUP BY series.id HAVING count(*) >= 2",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

/// 按 [`LibraryArchive::series`] 列出各系列的卷，按卷号、再按 ID 排列，系列按 ID 升序。
fn group_series(names: HashMap<i64, String>, archives: &[LibraryArchive]) -> Vec<Series> {
    let mut members: Vec<(i64, f64, i64)> = archives
        .iter()
        .filter_map(|archive| {
            let (series, volume) = archive.series?;
            Some((series, volume, archive.id))
        })
        .collect();
    members.sort_unstable_by(|a, b| {
        (a.0.cmp(&b.0))
            .then(a.1.total_cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    let mut series: Vec<Series> = names
        .into_iter()
        .map(|(id, name)| Series {
            id,
            name,
            volumes: Vec::new(),
        })
        .collect();
    series.sort_unstable_by_key(|series| series.id);
    for (id, _, archive) in members {
        if let Ok(position) = series.binary_search_by_key(&id, |series| series.id) {
            series[position].volumes.push(archive);
        }
    }
    series
}

fn read_pages(connection: &Connection, id: i64) -> Result<Vec<Page>> {
    let pages = connection
        .prepare_cached("SELECT pages FROM page_list WHERE archive_id = ?1")?
        .query_row([id], |row| row.get::<_, Vec<u8>>(0))
        .optional()?;
    Ok(match pages {
        Some(bytes) => postcard::from_bytes(&bytes)?,
        None => Vec::new(),
    })
}

fn read_all_progress(connection: &Connection) -> Result<HashMap<i64, Progress>> {
    Ok(connection
        .prepare("SELECT archive_id, page, client_time, updated_at, finished FROM progress")?
        .query_map([], |row| Ok((row.get(0)?, read_progress(row)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

fn read_progress(row: &rusqlite::Row<'_>) -> rusqlite::Result<Progress> {
    Ok(Progress {
        page: row.get("page")?,
        client_time: row.get("client_time")?,
        updated_at: row.get("updated_at")?,
        finished: row.get("finished")?,
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
            "SELECT page, client_time, updated_at, finished FROM progress WHERE archive_id = ?1",
            [id],
            read_progress,
        )
        .optional()?;
    Ok(Some(archive))
}

pub(crate) fn find_by_path(connection: &Connection, path: &str) -> Result<Option<i64>> {
    Ok(connection
        .query_row("SELECT id FROM archive WHERE path = ?1", [path], |row| {
            row.get(0)
        })
        .optional()?)
}

fn read_categories(connection: &Connection) -> Result<Vec<Category>> {
    let mut categories = connection
        .prepare("SELECT id, name, search, pinned FROM category ORDER BY pinned DESC, name, id")?
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                search: row.get(2)?,
                pinned: row.get(3)?,
                archives: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let positions: HashMap<i64, usize> = categories
        .iter()
        .enumerate()
        .map(|(position, category)| (category.id, position))
        .collect();
    let mut members = connection
        .prepare("SELECT category_id, archive_id FROM category_archive ORDER BY archive_id")?;
    let mut rows = members.query([])?;
    while let Some(row) = rows.next()? {
        if let Some(&position) = positions.get(&row.get::<_, i64>(0)?) {
            categories[position].archives.push(row.get(1)?);
        }
    }
    Ok(categories)
}

fn device_token(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceToken> {
    Ok(DeviceToken {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        last_used_at: row.get(3)?,
    })
}

/// 读一组设置，只读连接和写事务共用。
pub(crate) fn read_setting<T: serde::de::DeserializeOwned>(
    connection: &rusqlite::Connection,
    key: &str,
) -> Result<Option<T>> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    value
        .map(|value| {
            serde_json::from_str(&value).map_err(|source| Error::Setting {
                key: key.into(),
                source,
            })
        })
        .transpose()
}
