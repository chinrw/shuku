use std::ops::Deref;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};

use crate::reader::read_archive;
use crate::{
    Archive, ArchiveMetadata, Error, LrrArchive, LrrCategory, LrrPending, LrrTankoubon, Page,
    Reader, Result, ScanEntry, Tag, open_connection,
};

/// 读写连接，打开时完成迁移。读方法通过 [`Deref`] 来自 [`Reader`]，
/// 和写入用同一个连接，所以能读到自己刚提交的数据。
pub struct Store {
    reader: Reader,
}

/// 一个写事务里的操作。方法出错时不回滚已经做过的修改，由调用方决定：
/// 用 `?` 把错误传出 [`Store::write`] 回滚整个事务，或者用 [`Writer::savepoint`] 只撤销一步。
pub struct Writer<'a> {
    pub(crate) connection: &'a Connection,
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
            reader: Reader::wrap(open_connection(path)?),
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
    /// 比仍在读的快照更新的页留到下次。别的连接正在 checkpoint 时 SQLite 返回
    /// busy 和两个 -1，这时也算没写完。
    pub fn checkpoint(&self) -> Result<bool> {
        let (busy, log, checkpointed): (i64, i64, i64) =
            self.reader
                .connection
                .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
        Ok(busy == 0 && checkpointed == log)
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

    pub fn set_finished(&mut self, id: i64, finished: bool) -> Result<()> {
        self.write(|writer| writer.set_finished(id, finished))
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

    /// 读到的是本事务里已经做过的修改。
    pub fn find_by_path(&self, path: &str) -> Result<Option<i64>> {
        crate::reader::find_by_path(self.connection, path)
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
        let broken = pages.iter().filter(|page| page.width == 0).count() as i64;
        transaction.execute(
            "UPDATE archive SET broken_pages = ?1 WHERE id = ?2",
            params![broken, id],
        )?;
        Ok(id)
    }

    /// 标签按完整名称的 Unicode 小写形式去重，显示形式保留首次写入的大小写。
    /// 换下来的标签不再被任何归档和 Tankoubon 用到时一并删除。
    pub fn replace_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        require_archive(self.connection, id)?;
        set_archive_tags(self.connection, id, tags)
    }

    /// 改标题或简介，`None` 的不变。
    pub fn edit_archive(
        &mut self,
        id: i64,
        title: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let updated = self.connection.execute(
            "UPDATE archive SET title = coalesce(?2, title), summary = coalesce(?3, summary)
             WHERE id = ?1",
            params![id, title, summary],
        )?;
        if updated == 0 {
            return Err(Error::ArchiveNotFound(id));
        }
        Ok(())
    }

    /// 设置或清除来源 URL。
    pub fn set_source_url(&mut self, id: i64, source_url: Option<&str>) -> Result<()> {
        let updated = self.connection.execute(
            "UPDATE archive SET source_url = ?2 WHERE id = ?1",
            params![id, source_url],
        )?;
        if updated == 0 {
            return Err(Error::ArchiveNotFound(id));
        }
        Ok(())
    }

    /// 给归档加上标签，已经有的不变。
    pub fn add_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        require_archive(self.connection, id)?;
        let mut associate = self.connection.prepare_cached(
            "INSERT INTO archive_tag (tag_id, archive_id) VALUES (?1, ?2)
             ON CONFLICT(tag_id, archive_id) DO NOTHING",
        )?;
        for tag in tags {
            associate.execute([ensure_tag(self.connection, tag)?, id])?;
        }
        Ok(())
    }

    /// 从归档上去掉标签，按小写完整名匹配，归档没有的标签跳过。
    pub fn remove_tags(&mut self, id: i64, tags: &[Tag]) -> Result<()> {
        require_archive(self.connection, id)?;
        let mut remove = self.connection.prepare_cached(
            "DELETE FROM archive_tag WHERE archive_id = ?1
                AND tag_id = (SELECT id FROM tag WHERE full = ?2)
             RETURNING tag_id",
        )?;
        let mut removed = Vec::new();
        for tag in tags {
            removed.extend(
                remove
                    .query_map(params![id, full_name(tag)], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<i64>>>()?,
            );
        }
        prune_tags(self.connection, &removed)
    }

    /// 把标签 `from` 改名为 `to`，返回用到它的归档数。`to` 已经存在时合并进去：
    /// 原来带 `from` 的归档和 Tankoubon 改带 `to`，`from` 删除。两个名字只差大小写时
    /// 只改显示形式。
    pub fn rename_tag(&mut self, from: &Tag, to: &Tag) -> Result<u64> {
        let connection = self.connection;
        let from_full = full_name(from);
        let from_id: i64 = connection
            .query_row("SELECT id FROM tag WHERE full = ?1", [&from_full], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(Error::TagNotFound(from_full))?;
        let uses: u64 = connection.query_row(
            "SELECT count(*) FROM archive_tag WHERE tag_id = ?1",
            [from_id],
            |row| row.get(0),
        )?;
        let to_full = full_name(to);
        let to_id: Option<i64> = connection
            .query_row(
                "SELECT id FROM tag WHERE full = ?1 AND id != ?2",
                params![to_full, from_id],
                |row| row.get(0),
            )
            .optional()?;
        match to_id {
            None => {
                connection.execute(
                    "UPDATE tag SET namespace = ?2, value = ?3, full = ?4 WHERE id = ?1",
                    params![from_id, to.namespace, to.value, to_full],
                )?;
            }
            Some(to_id) => {
                connection.execute(
                    "INSERT INTO archive_tag (tag_id, archive_id)
                     SELECT ?2, archive_id FROM archive_tag WHERE tag_id = ?1
                     ON CONFLICT(tag_id, archive_id) DO NOTHING",
                    [from_id, to_id],
                )?;
                connection.execute(
                    "INSERT INTO tankoubon_tag (tankoubon_id, tag_id)
                     SELECT tankoubon_id, ?2 FROM tankoubon_tag WHERE tag_id = ?1
                     ON CONFLICT DO NOTHING",
                    [from_id, to_id],
                )?;
                // 旧标签的关联由外键级联删除。
                connection.execute("DELETE FROM tag WHERE id = ?1", [from_id])?;
            }
        }
        Ok(uses)
    }

    /// 删除归档记录和它的标签关联、页面清单、阅读进度、分类和 Tankoubon 成员关系。
    /// 原始文件不动；只剩这本用到的标签一并删除。
    pub fn delete_archive(&mut self, id: i64) -> Result<()> {
        let tags = archive_tag_ids(self.connection, id)?;
        if self
            .connection
            .execute("DELETE FROM archive WHERE id = ?1", [id])?
            == 0
        {
            return Err(Error::ArchiveNotFound(id));
        }
        prune_tags(self.connection, &tags)
    }

    /// 用 LRR 的元数据和标签覆盖一本已入库的归档。
    pub fn import_lrr_archive(&mut self, id: i64, lrr: &LrrArchive) -> Result<()> {
        let transaction = self.connection;
        let updated = transaction.execute(
            "UPDATE archive SET title = ?1, summary = ?2, date_added = COALESCE(?3, mtime),
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

    /// 新建分类，返回它的 ID。`search` 为空是静态分类，成员用 [`Writer::set_category_member`] 增删。
    pub fn create_category(&mut self, name: &str, search: &str, pinned: bool) -> Result<i64> {
        Ok(self.connection.query_row(
            "INSERT INTO category (name, search, pinned) VALUES (?1, ?2, ?3) RETURNING id",
            params![name, search, pinned],
            |row| row.get(0),
        )?)
    }

    /// 改分类的名字、搜索语句或置顶，`None` 的不变。静态分类和动态分类不能互相转换：
    /// 转换会让静态分类的成员失效，或者让动态分类变成空的静态分类。
    pub fn update_category(
        &mut self,
        id: i64,
        name: Option<&str>,
        search: Option<&str>,
        pinned: Option<bool>,
    ) -> Result<()> {
        let dynamic = category_is_dynamic(self.connection, id)?;
        if search.is_some_and(|search| search.is_empty() == dynamic) {
            return Err(Error::CategoryKindChange(id));
        }
        self.connection.execute(
            "UPDATE category SET name = coalesce(?2, name), search = coalesce(?3, search),
                pinned = coalesce(?4, pinned)
             WHERE id = ?1",
            params![id, name, search, pinned],
        )?;
        Ok(())
    }

    /// 删除分类和它的成员关系，归档本身不动。
    pub fn delete_category(&mut self, id: i64) -> Result<()> {
        if self
            .connection
            .execute("DELETE FROM category WHERE id = ?1", [id])?
            == 0
        {
            return Err(Error::CategoryNotFound(id));
        }
        Ok(())
    }

    /// 确认分类存在而且是静态分类，批量操作在改任何一本之前先查一次。
    pub fn require_static_category(&self, id: i64) -> Result<()> {
        if category_is_dynamic(self.connection, id)? {
            return Err(Error::DynamicCategory(id));
        }
        Ok(())
    }

    /// 把归档加进静态分类（`member` 为真）或移出，返回成员关系是否有变化。
    pub fn set_category_member(
        &mut self,
        category: i64,
        archive: i64,
        member: bool,
    ) -> Result<bool> {
        if category_is_dynamic(self.connection, category)? {
            return Err(Error::DynamicCategory(category));
        }
        require_archive(self.connection, archive)?;
        let sql = if member {
            "INSERT INTO category_archive (category_id, archive_id) VALUES (?1, ?2)
             ON CONFLICT DO NOTHING"
        } else {
            "DELETE FROM category_archive WHERE category_id = ?1 AND archive_id = ?2"
        };
        Ok(self.connection.execute(sql, [category, archive])? > 0)
    }

    /// 按 LRR ID 新建或更新 Tankoubon，标签和归档成员整体替换。
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
            for (member, position) in &tankoubon.archives {
                archive.execute(params![id, member, position])?;
            }
        }
        Ok(())
    }

    /// 在写事务里读一组设置，读、改、写在同一个事务里，两个请求同时改不会丢掉一个。
    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        crate::reader::read_setting(self.connection, key)
    }

    /// 整组写入一组设置，存成 JSON。
    pub fn set_setting<T: serde::Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let value = serde_json::to_string(value).map_err(|source| Error::Setting {
            key: key.into(),
            source,
        })?;
        self.connection.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// 新建设备令牌，`hash` 是令牌的摘要，返回 ID。
    pub fn create_device_token(&mut self, name: &str, hash: &str, now: i64) -> Result<i64> {
        Ok(self.connection.query_row(
            "INSERT INTO device_token (name, hash, created_at) VALUES (?1, ?2, ?3) RETURNING id",
            params![name, hash, now],
            |row| row.get(0),
        )?)
    }

    /// 吊销令牌，返回它是否存在。
    pub fn delete_device_token(&mut self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM device_token WHERE id = ?1", [id])?
            > 0)
    }

    /// 记下令牌最近一次使用的时间。
    pub fn touch_device_token(&mut self, id: i64, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE device_token SET last_used_at = ?2 WHERE id = ?1",
            [id, now],
        )?;
        Ok(())
    }

    pub fn delete_setting(&mut self, key: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM setting WHERE key = ?1", [key])?;
        Ok(())
    }

    /// 删除全部暂存的 LRR 元数据。迁移工具每次运行先清空再按 LRR 的现状重新暂存。
    pub fn clear_lrr_pending(&mut self) -> Result<()> {
        self.connection.execute("DELETE FROM lrr_pending", [])?;
        Ok(())
    }

    /// 暂存一本还没扫描到的归档的 LRR 元数据，`path` 是相对内容目录的路径。
    pub fn stage_lrr_pending(&mut self, path: &str, pending: &LrrPending) -> Result<()> {
        self.connection.execute(
            "INSERT INTO lrr_pending (path, data) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET data = excluded.data",
            params![path, postcard::to_stdvec(pending)?],
        )?;
        Ok(())
    }

    /// 归档 `id` 的路径有暂存的 LRR 元数据时，套用元数据、阅读进度和分类、
    /// Tankoubon 成员，然后删除暂存，返回是否套用。分类或 Tankoubon 已经不在时跳过那一项。
    pub fn apply_lrr_pending(&mut self, id: i64, path: &str) -> Result<bool> {
        let data: Option<Vec<u8>> = self
            .connection
            .query_row(
                "DELETE FROM lrr_pending WHERE path = ?1 RETURNING data",
                [path],
                |row| row.get(0),
            )
            .optional()?;
        let Some(data) = data else {
            return Ok(false);
        };
        let pending: LrrPending = postcard::from_bytes(&data)?;
        self.import_lrr_archive(id, &pending.archive)?;
        if let Some((page, client_time)) = pending.progress {
            self.write_progress(id, page, client_time)?;
        }
        self.add_lrr_memberships(id, &pending.categories, &pending.tankoubons)?;
        Ok(true)
    }

    /// 把归档加进按 LRR ID 指定的分类和 Tankoubon，已经在里面的不变，
    /// 分类或 Tankoubon 不存在时跳过。
    pub fn add_lrr_memberships(
        &mut self,
        id: i64,
        categories: &[String],
        tankoubons: &[(String, u32)],
    ) -> Result<()> {
        let transaction = self.connection;
        let mut category = transaction.prepare_cached(
            "INSERT INTO category_archive (category_id, archive_id)
             SELECT id, ?2 FROM category WHERE lrr_id = ?1 ON CONFLICT DO NOTHING",
        )?;
        for lrr_id in categories {
            category.execute(params![lrr_id, id])?;
        }
        let mut tankoubon = transaction.prepare_cached(
            "INSERT INTO tankoubon_archive (tankoubon_id, archive_id, position)
             SELECT id, ?2, ?3 FROM tankoubon WHERE lrr_id = ?1 ON CONFLICT DO NOTHING",
        )?;
        for (lrr_id, position) in tankoubons {
            tankoubon.execute(params![lrr_id, id, position])?;
        }
        Ok(())
    }

    /// 把归档放进 `key` 对应的系列，`volume` 是它在系列里的排序位置，可以有小数：`volume` 列
    /// 声明为 INTEGER，整数照旧存成整数，带小数的 SQLite 存成 REAL，排序时按数值比较。
    /// 系列不存在时以 `name` 新建，已经存在时名字不变。
    pub fn set_series(&mut self, id: i64, key: &str, name: &str, volume: f64) -> Result<()> {
        require_archive(self.connection, id)?;
        let series: i64 = self.connection.query_row(
            "INSERT INTO series (name, key) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET key = excluded.key
             RETURNING id",
            params![name, key],
            |row| row.get(0),
        )?;
        self.connection.execute(
            "UPDATE archive SET series_id = ?1, volume = ?2 WHERE id = ?3",
            params![series, volume, id],
        )?;
        Ok(())
    }

    /// 有没有以 `key` 为归组依据的系列。
    pub fn series_key_exists(&self, key: &str) -> Result<bool> {
        Ok(self
            .connection
            .query_row("SELECT 1 FROM series WHERE key = ?1", [key], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// 以 `key` 为归组依据的系列里有没有排序位置小于 `below` 的归档，服务端用它判断有没有正篇。
    pub fn series_has_volume_below(&self, key: &str, below: f64) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM archive JOIN series ON series.id = archive.series_id
                 WHERE series.key = ?1 AND archive.volume < ?2 LIMIT 1",
                params![key, below],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// 归档所在系列的文件夹搬到了别处：把系列的归组依据换成 `key`，之后扫到的新卷归进原系列。
    /// 归档不属于系列、`key` 已经是另一个系列的依据、或者系列不是按路径归组（依据以 `/` 开头，
    /// 例如按 ComicInfo 的系列名，和位置无关）时不变。
    pub fn move_series(&mut self, id: i64, key: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE series SET key = ?1
             WHERE id = (SELECT series_id FROM archive WHERE id = ?2)
               AND substr(key, 1, 1) != '/'
               AND NOT EXISTS (SELECT 1 FROM series WHERE key = ?1)",
            params![key, id],
        )?;
        Ok(())
    }

    /// 仅接受翻页时间严格更新的上报，返回是否写入。
    pub fn write_progress(&mut self, id: i64, page: u32, client_time: i64) -> Result<bool> {
        let transaction = self.connection;
        require_archive(transaction, id)?;
        // 读到最后一页时记下 finished，之后翻回前面也不清掉。
        let accepted = transaction.execute(
            "INSERT INTO progress (archive_id, page, client_time, updated_at, finished)
             VALUES (?1, ?2, ?3, CAST(unixepoch('subsec') * 1000 AS INTEGER),
                ?2 + 1 >= (SELECT page_count FROM archive WHERE id = ?1))
             ON CONFLICT(archive_id) DO UPDATE SET page = excluded.page,
                client_time = excluded.client_time, updated_at = excluded.updated_at,
                finished = progress.finished OR excluded.finished
             WHERE excluded.client_time > progress.client_time",
            params![id, page, client_time],
        )? > 0;
        Ok(accepted)
    }

    /// 手动标记读完或取消。标记时没有进度的书记在最后一页，和读到最后一页一样（LRR 的
    /// “标记为已读”也是这样）；已有进度的只改标记，不动页码和翻页时间，各设备之后的
    /// 上报照常按时间先后比较。
    pub fn set_finished(&mut self, id: i64, finished: bool) -> Result<()> {
        let transaction = self.connection;
        require_archive(transaction, id)?;
        if finished {
            transaction.execute(
                "INSERT INTO progress (archive_id, page, client_time, updated_at, finished)
                 SELECT id, max(page_count - 1, 0), CAST(unixepoch('subsec') * 1000 AS INTEGER),
                    CAST(unixepoch('subsec') * 1000 AS INTEGER), 1
                 FROM archive WHERE id = ?1
                 ON CONFLICT(archive_id) DO UPDATE SET finished = 1",
                [id],
            )?;
        } else {
            transaction.execute(
                "UPDATE progress SET finished = 0 WHERE archive_id = ?1",
                [id],
            )?;
        }
        Ok(())
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
    let old = archive_tag_ids(connection, id)?;
    connection.execute("DELETE FROM archive_tag WHERE archive_id = ?1", [id])?;
    let mut associate = connection.prepare_cached(
        "INSERT INTO archive_tag (tag_id, archive_id) VALUES (?1, ?2)
         ON CONFLICT(tag_id, archive_id) DO NOTHING",
    )?;
    for tag in tags {
        associate.execute([ensure_tag(connection, tag)?, id])?;
    }
    prune_tags(connection, &old)
}

fn archive_tag_ids(connection: &Connection, id: i64) -> Result<Vec<i64>> {
    Ok(connection
        .prepare_cached("SELECT tag_id FROM archive_tag WHERE archive_id = ?1")?
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// 删除 `ids` 里已经没有归档和 Tankoubon 用到的标签。没人用的标签留着会出现在
/// 搜索框的补全里，改错的标签也就一直删不掉。
fn prune_tags(connection: &Connection, ids: &[i64]) -> Result<()> {
    let mut prune = connection.prepare_cached(
        "DELETE FROM tag WHERE id = ?1
            AND NOT EXISTS (SELECT 1 FROM archive_tag WHERE tag_id = ?1)
            AND NOT EXISTS (SELECT 1 FROM tankoubon_tag WHERE tag_id = ?1)",
    )?;
    for id in ids {
        prune.execute([id])?;
    }
    Ok(())
}

/// 小写的完整名，标签表按它去重。
fn full_name(tag: &Tag) -> String {
    if tag.namespace.is_empty() {
        tag.value.to_lowercase()
    } else {
        format!("{}:{}", tag.namespace, tag.value).to_lowercase()
    }
}

/// 返回标签 ID，没有时新建。冲突时的空更新只是为了让 RETURNING 在已有标签上也返回 ID。
fn ensure_tag(connection: &Connection, tag: &Tag) -> Result<i64> {
    let full = full_name(tag);
    Ok(connection
        .prepare_cached(
            "INSERT INTO tag (namespace, value, full) VALUES (?1, ?2, ?3)
             ON CONFLICT(full) DO UPDATE SET full = excluded.full
             RETURNING id",
        )?
        .query_row(params![tag.namespace, tag.value, full], |row| row.get(0))?)
}

/// 分类是否是动态分类；分类不存在时返回 [`Error::CategoryNotFound`]。
fn category_is_dynamic(connection: &Connection, id: i64) -> Result<bool> {
    connection
        .query_row(
            "SELECT search != '' FROM category WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::CategoryNotFound(id))
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
