//! 完整指纹相同的文件共用归档 ID。主路径仍在 archive，额外路径不另存页面清单。
use crate::{Archive, Reader, Result, Writer};
use rusqlite::{OptionalExtension, params};

impl Reader {
    pub fn books_with_copies(&self) -> Result<Vec<i64>> {
        Ok(self
            .connection
            .prepare("SELECT DISTINCT archive_id FROM archive_copy")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn redirected_archive(&self, old: i64) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row(
                "SELECT archive_id FROM archive_redirect WHERE old_id=?1",
                [old],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// 只把新记录归到更早的 ID。旧库里即使有多个相同指纹，也不自动合并那些旧 ID。
    pub fn automatic_copy_pairs(&self) -> Result<Vec<(i64, i64)>> {
        Ok(self
            .connection
            .prepare(
                "SELECT min(a.id), b.id FROM archive b JOIN archive a
            ON a.content_hash=b.content_hash AND a.id<b.id WHERE b.auto_copy=1
            AND b.content_hash IS NOT NULL GROUP BY b.id ORDER BY b.id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

impl Writer<'_> {
    /// 归并前已经打开的阅读器仍可能上报旧 ID。和进度写入放在同一个事务。
    pub fn progress_archive_id(&self, id: i64) -> Result<i64> {
        Ok(self
            .connection
            .prepare_cached("SELECT archive_id FROM archive_redirect WHERE old_id=?1")?
            .query_row([id], |r| r.get(0))
            .optional()?
            .unwrap_or(id))
    }

    pub fn set_copy_stamp(&mut self, id: i64, stamp: Option<i64>) -> Result<()> {
        self.connection.execute(
            "UPDATE archive SET copy_stamp=?2 WHERE id=?1",
            params![id, stamp],
        )?;
        Ok(())
    }

    pub fn allow_auto_copy(&mut self, id: i64) -> Result<()> {
        self.connection
            .execute("UPDATE archive SET auto_copy=1 WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn remove_copy(&mut self, id: i64, path: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM archive_copy WHERE archive_id=?1 AND path=?2",
            params![id, path],
        )?;
        Ok(())
    }

    pub fn record_copy(
        &mut self,
        id: i64,
        path: &str,
        mtime: i64,
        stamp: Option<i64>,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO archive_copy (archive_id, path, mtime, stamp) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(archive_id, path) DO UPDATE SET mtime=excluded.mtime, stamp=excluded.stamp",
            params![id, path, mtime, stamp],
        )?;
        Ok(())
    }

    pub fn set_primary_file(
        &mut self,
        id: i64,
        path: &str,
        mtime: i64,
        stamp: Option<i64>,
    ) -> Result<()> {
        self.remove_copy(id, path)?;
        self.connection.execute(
            "UPDATE archive SET path=?2,mtime=?3,copy_stamp=?4,missing=0 WHERE id=?1",
            params![id, path, mtime, stamp],
        )?;
        Ok(())
    }

    /// 调用方已经核对磁盘上的指纹。事务内再读元数据，保留核对期间提交的进度和编辑。
    pub fn merge_copy(
        &mut self,
        keep: i64,
        other: i64,
        hash: &str,
        target_stamp: Option<i64>,
        source_stamp: Option<i64>,
    ) -> Result<bool> {
        let Some(target) = self.archive(keep)? else {
            return Ok(false);
        };
        let Some(source) = self.archive(other)? else {
            return Ok(false);
        };
        if keep == other
            || [&target, &source]
                .iter()
                .any(|a| a.metadata.content_hash.as_deref() != Some(hash))
        {
            return Ok(false);
        }
        let db = self.connection;
        db.execute(
            "INSERT INTO archive_tag (tag_id, archive_id)
             SELECT tag_id, ?1 FROM archive_tag WHERE archive_id=?2 ON CONFLICT DO NOTHING",
            params![keep, other],
        )?;
        db.execute(
            "INSERT INTO category_archive (category_id, archive_id)
             SELECT category_id, ?1 FROM category_archive WHERE archive_id=?2 ON CONFLICT DO NOTHING",
            params![keep, other],
        )?;
        db.execute(
            "INSERT INTO tankoubon_archive (tankoubon_id, archive_id, position)
             SELECT tankoubon_id, ?1, position FROM tankoubon_archive WHERE archive_id=?2
             ON CONFLICT DO NOTHING",
            params![keep, other],
        )?;
        db.execute(
            "INSERT INTO metadata_queue (archive_id, provider)
             SELECT ?1, provider FROM metadata_queue WHERE archive_id=?2 ON CONFLICT DO NOTHING",
            params![keep, other],
        )?;
        db.execute(
            "INSERT INTO metadata_candidate (archive_id, provider, found, fetched_at)
             SELECT ?1, provider, found, fetched_at FROM metadata_candidate WHERE archive_id=?2
             ON CONFLICT(archive_id, provider) DO UPDATE SET
                 found=excluded.found, fetched_at=excluded.fetched_at
             WHERE excluded.fetched_at > metadata_candidate.fetched_at",
            params![keep, other],
        )?;
        db.execute(
            "INSERT INTO progress (archive_id, page, client_time, updated_at, finished)
             SELECT ?1, page, client_time, updated_at, finished FROM progress WHERE archive_id=?2
             ON CONFLICT(archive_id) DO UPDATE SET
                 page=CASE WHEN excluded.client_time > progress.client_time THEN excluded.page ELSE progress.page END,
                 client_time=max(progress.client_time, excluded.client_time),
                 updated_at=max(progress.updated_at, excluded.updated_at),
                 finished=max(progress.finished, excluded.finished)",
            params![keep, other],
        )?;
        db.execute(
            "UPDATE archive AS a SET copy_stamp=?3, summary=coalesce(nullif(a.summary,''), b.summary),
                 source_url=coalesce(a.source_url, b.source_url), lrr_id=coalesce(a.lrr_id, b.lrr_id),
                 series_id=coalesce(a.series_id, b.series_id),
                 volume=CASE WHEN a.series_id IS NULL THEN b.volume ELSE a.volume END
             FROM archive AS b WHERE a.id=?1 AND b.id=?2",
            params![keep, other, target_stamp],
        )?;
        db.execute(
            "UPDATE archive_copy SET archive_id=?1 WHERE archive_id=?2",
            params![keep, other],
        )?;
        db.execute(
            "UPDATE archive_redirect SET archive_id=?1 WHERE archive_id=?2",
            params![keep, other],
        )?;
        self.delete_archive(other)?;
        self.record_copy(
            keep,
            &source.metadata.path,
            source.metadata.mtime,
            source_stamp,
        )?;
        db.execute(
            "INSERT INTO archive_redirect(old_id,archive_id) VALUES (?2,?1)",
            params![keep, other],
        )?;
        Ok(true)
    }
}

impl Archive {
    pub fn file_paths(&self) -> impl Iterator<Item = (&str, i64, Option<i64>)> {
        std::iter::once((
            self.metadata.path.as_str(),
            self.metadata.mtime,
            self.copy_stamp,
        ))
        .chain(
            self.copies
                .iter()
                .map(|(path, mtime, stamp)| (path.as_str(), *mtime, *stamp)),
        )
    }
}
