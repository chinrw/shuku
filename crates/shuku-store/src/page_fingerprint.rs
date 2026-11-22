//! 页面查重读取 SQLite 的已提交版本，不依赖可能落后的书库列表快照。
use rusqlite::params;

use crate::{Archive, Reader, Result, Writer, reader::read_archive};

impl Reader {
    pub fn pending_page_fingerprints(&self, after: i64, limit: usize) -> Result<Vec<i64>> {
        Ok(self
            .connection
            .prepare(
                "SELECT id FROM archive WHERE missing=0 AND page_fingerprint IS NULL
             AND content_hash IS NOT NULL AND id>?1 ORDER BY id LIMIT ?2",
            )?
            .query_map(params![after, limit as i64], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// 命中和覆盖情况必须来自同一个读取事务，否则扫描并发入库时可能给出错误的否定结论。
    pub fn lookup_page_fingerprint(&self, signature: &[u8; 32]) -> Result<(u64, Vec<Archive>)> {
        let tx = self.connection.unchecked_transaction()?;
        let pending = tx.query_row(
            "SELECT count(*) FROM archive WHERE missing=0 AND page_fingerprint IS NULL",
            [],
            |row| row.get(0),
        )?;
        let mut matches = Vec::new();
        let mut statement = tx.prepare(
            "SELECT id FROM archive WHERE missing=0 AND page_fingerprint=?1 ORDER BY id",
        )?;
        for id in statement.query_map([signature], |row| row.get::<_, i64>(0))? {
            let id = id?;
            matches.push(read_archive(&tx, id)?.ok_or(crate::Error::ArchiveNotFound(id))?);
        }
        drop(statement);
        tx.commit()?;
        Ok((pending, matches))
    }
}

impl Writer<'_> {
    pub fn record_page_fingerprint(
        &mut self,
        book: &Archive,
        signature: &[u8; 32],
        primary_stamp: Option<i64>,
    ) -> Result<()> {
        self.connection
            .prepare_cached(
                "UPDATE archive SET page_fingerprint=?1, copy_stamp=coalesce(?2,copy_stamp)
             WHERE id=?3 AND path=?4 AND mtime=?5 AND content_hash=?6 AND page_count=?7
               AND missing=0 AND page_fingerprint IS NULL",
            )?
            .execute(params![
                signature,
                primary_stamp,
                book.id,
                book.metadata.path,
                book.metadata.mtime,
                book.metadata.content_hash,
                book.page_count
            ])?;
        Ok(())
    }
}
