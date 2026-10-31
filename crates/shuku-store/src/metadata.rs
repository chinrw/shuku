//! 在线元数据的后台队列和待确认的候选结果。来源用服务端定的名字（`ehentai`、`bangumi`、
//! `anilist`），存储层不解释它们。

use rusqlite::{Connection, OptionalExtension, params};

use crate::{MetadataCandidate, Reader, Result, Writer};

/// 列表接口一次最多给这么多条。
const PAGE_LIMIT: usize = 1000;

impl Reader {
    /// 某个来源队列里的下一本，按 ID 从小到大。
    pub fn next_queued_metadata(&self, provider: &str) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row(
                "SELECT archive_id FROM metadata_queue WHERE provider = ?1
                 ORDER BY archive_id LIMIT 1",
                [provider],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// 每个来源排着的本数，按来源名排序。
    pub fn metadata_queue_counts(&self) -> Result<Vec<(String, u64)>> {
        counts(&self.connection, "metadata_queue")
    }

    /// 队列里排在 `after`（归档 ID 和来源）之后的最多 `limit` 条，按归档 ID、来源排序。
    pub fn metadata_queue(
        &self,
        provider: Option<&str>,
        after: Option<(i64, &str)>,
        limit: usize,
    ) -> Result<Vec<(i64, String)>> {
        let (after_id, after_provider) = after.unwrap_or((i64::MIN, ""));
        let mut statement = self.connection.prepare_cached(
            "SELECT archive_id, provider FROM metadata_queue
             WHERE (?1 IS NULL OR provider = ?1) AND (archive_id, provider) > (?2, ?3)
             ORDER BY archive_id, provider LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![provider, after_id, after_provider, limit.min(PAGE_LIMIT)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn metadata_candidate(
        &self,
        archive_id: i64,
        provider: &str,
    ) -> Result<Option<MetadataCandidate>> {
        Ok(self
            .connection
            .query_row(
                "SELECT archive_id, provider, found, fetched_at FROM metadata_candidate
                 WHERE archive_id = ?1 AND provider = ?2",
                params![archive_id, provider],
                candidate,
            )
            .optional()?)
    }

    /// 排在 `after` 之后的最多 `limit` 条候选，按归档 ID、来源排序。
    pub fn metadata_candidates(
        &self,
        provider: Option<&str>,
        after: Option<(i64, &str)>,
        limit: usize,
    ) -> Result<Vec<MetadataCandidate>> {
        let (after_id, after_provider) = after.unwrap_or((i64::MIN, ""));
        let mut statement = self.connection.prepare_cached(
            "SELECT archive_id, provider, found, fetched_at FROM metadata_candidate
             WHERE (?1 IS NULL OR provider = ?1) AND (archive_id, provider) > (?2, ?3)
             ORDER BY archive_id, provider LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![provider, after_id, after_provider, limit.min(PAGE_LIMIT)],
            candidate,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 每个来源待确认的候选数，按来源名排序。
    pub fn metadata_candidate_counts(&self) -> Result<Vec<(String, u64)>> {
        counts(&self.connection, "metadata_candidate")
    }
}

impl Writer<'_> {
    /// 把归档放进某个来源的队列，已经在里面的不变。归档不存在时返回外键错误。
    pub fn queue_metadata(&mut self, archive_id: i64, provider: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO metadata_queue (archive_id, provider) VALUES (?1, ?2)
             ON CONFLICT DO NOTHING",
            params![archive_id, provider],
        )?;
        Ok(())
    }

    /// 从队列里去掉一条，返回它是否在队列里。
    pub fn dequeue_metadata(&mut self, archive_id: i64, provider: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM metadata_queue WHERE archive_id = ?1 AND provider = ?2",
            params![archive_id, provider],
        )? > 0)
    }

    /// 清空一个来源的队列，`None` 清空全部，返回去掉了几条。
    pub fn clear_metadata_queue(&mut self, provider: Option<&str>) -> Result<u64> {
        Ok(self.connection.execute(
            "DELETE FROM metadata_queue WHERE ?1 IS NULL OR provider = ?1",
            [provider],
        )? as u64)
    }

    /// 存下或替换一条候选。
    pub fn save_metadata_candidate(
        &mut self,
        archive_id: i64,
        provider: &str,
        found: &str,
        fetched_at: i64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO metadata_candidate (archive_id, provider, found, fetched_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(archive_id, provider) DO UPDATE SET found = excluded.found,
                fetched_at = excluded.fetched_at",
            params![archive_id, provider, found, fetched_at],
        )?;
        Ok(())
    }

    /// 改一条已有候选的内容（例如 agent 译好的），不改查到的时间，返回它是否存在。
    pub fn update_metadata_candidate(
        &mut self,
        archive_id: i64,
        provider: &str,
        found: &str,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE metadata_candidate SET found = ?3 WHERE archive_id = ?1 AND provider = ?2",
            params![archive_id, provider, found],
        )? > 0)
    }

    /// 删掉一条候选并返回它，批准和拒绝都用它。
    pub fn take_metadata_candidate(
        &mut self,
        archive_id: i64,
        provider: &str,
    ) -> Result<Option<MetadataCandidate>> {
        Ok(self
            .connection
            .query_row(
                "DELETE FROM metadata_candidate WHERE archive_id = ?1 AND provider = ?2
                 RETURNING archive_id, provider, found, fetched_at",
                params![archive_id, provider],
                candidate,
            )
            .optional()?)
    }
}

fn candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetadataCandidate> {
    Ok(MetadataCandidate {
        archive_id: row.get(0)?,
        provider: row.get(1)?,
        found: row.get(2)?,
        fetched_at: row.get(3)?,
    })
}

/// `table` 只会是本模块里的两个表名。
fn counts(connection: &Connection, table: &str) -> Result<Vec<(String, u64)>> {
    let mut statement = connection.prepare(&format!(
        "SELECT provider, count(*) FROM {table} GROUP BY provider ORDER BY provider"
    ))?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
