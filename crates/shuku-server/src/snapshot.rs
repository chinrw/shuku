//! 搜索和列表页读的内存快照：索引、列表元数据和阅读进度。快照构建后不可变，
//! 数据变化后整个换掉，搜索结果页因此不查数据库。

use crate::{App, Result};
use shuku_index::Index;
use shuku_store::{LibraryArchive, LibraryTag, Progress, Reader, Tag};
use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, atomic::Ordering},
    time::{Duration, Instant},
};

/// 数据变化后等这么久再刷新快照，期间的变化合并成一次刷新。读可以滞后 10 到 20 秒。
#[cfg(not(test))]
const REFRESH_DELAY: Duration = Duration::from_secs(10);
/// 单元测试里扫描在阻塞线程上停住时 tokio 的暂停时钟不会快进，只能用真实时间，所以缩短。
#[cfg(test)]
const REFRESH_DELAY: Duration = Duration::from_millis(100);

pub(crate) struct Snapshot {
    pub(crate) index: Arc<Index>,
    listing: Arc<Listing>,
    /// 进度变得最勤，单独刷新，不用重建索引和列表。
    progress: Arc<HashMap<i64, Progress>>,
}

struct Listing {
    tags: Vec<LibraryTag>,
    /// 按 ID 升序，按 ID 二分查找。
    archives: Vec<LibraryArchive>,
}

/// 列表页的一项。
pub(crate) struct Item<'a> {
    pub(crate) archive: &'a LibraryArchive,
    pub(crate) progress: Option<&'a Progress>,
    tags: &'a [LibraryTag],
}

impl Item<'_> {
    /// 按小写完整名排序。
    pub(crate) fn tags(&self) -> impl Iterator<Item = &Tag> {
        self.archive
            .tags
            .iter()
            .map(|&position| &self.tags[position as usize].tag)
    }
}

impl Snapshot {
    pub(crate) fn load(reader: &Reader) -> Result<Self> {
        let start = Instant::now();
        let library = reader.library()?;
        let read_ms = start.elapsed().as_millis();
        let index = Index::build(library.index_records())?;
        tracing::info!(
            count = library.archives.len(),
            read_ms,
            elapsed_ms = start.elapsed().as_millis(),
            "快照构建完成"
        );
        Ok(Self {
            index: Arc::new(index),
            listing: Arc::new(Listing {
                tags: library.tags,
                archives: library.archives,
            }),
            progress: Arc::new(library.progress),
        })
    }

    pub(crate) fn item(&self, id: i64) -> Option<Item<'_>> {
        let archives = &self.listing.archives;
        let position = archives
            .binary_search_by_key(&id, |archive| archive.id)
            .ok()?;
        Some(Item {
            archive: &archives[position],
            progress: self.progress.get(&id),
            tags: &self.listing.tags,
        })
    }
}

impl App {
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.0.snapshot.load_full()
    }

    /// 立即按数据库重建整个快照，不等刷新延迟。扫描结束时调用；
    /// 测试用它在写入后马上检查搜索结果。
    pub async fn refresh(&self) -> Result<()> {
        self.blocking(|app| app.refresh_blocking()).await
    }

    /// 列表元数据或标签已经提交了变化：安排一次延迟的整体重建。
    /// 扫描每提交一批就调用，扫得再久，搜索结果里的封面 URL 也最多落后一个刷新延迟。
    pub(crate) fn listing_changed(&self) {
        self.0.listing_changed.store(true, Ordering::Release);
        self.0.changed.notify_one();
    }

    pub(crate) fn refresh_blocking(&self) -> Result<()> {
        // 刷新串行进行，并且在拿到锁以后才读库，后换上的快照不会比先换上的旧。
        let _refresh = self
            .0
            .refresh_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // 先清标志再读库：读库之后才提交的变化会重新置位，不会漏掉。失败时恢复标志，
        // 重试时仍然整体重建。
        self.0.listing_changed.store(false, Ordering::Release);
        let snapshot = self
            .reader()
            .and_then(|reader| Snapshot::load(&reader))
            .inspect_err(|_| self.0.listing_changed.store(true, Ordering::Release))?;
        self.0.snapshot.store(Arc::new(snapshot));
        Ok(())
    }

    fn refresh_pending(&self) -> Result<()> {
        if self.0.listing_changed.load(Ordering::Acquire) {
            self.refresh_blocking()
        } else {
            self.refresh_progress()
        }
    }

    fn refresh_progress(&self) -> Result<()> {
        let _refresh = self
            .0
            .refresh_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let progress = self.reader()?.progress()?;
        let current = self.snapshot();
        self.0.snapshot.store(Arc::new(Snapshot {
            index: current.index.clone(),
            listing: current.listing.clone(),
            progress: Arc::new(progress),
        }));
        Ok(())
    }

    /// 每次提交了快照里的数据就通知一次；这里等 [`REFRESH_DELAY`] 再刷新，
    /// 等待期间的通知合并成一次。任务只持有弱引用，不会让 `App` 一直活着。
    pub(crate) fn spawn_progress_refresher(&self) {
        let app = Arc::downgrade(&self.0);
        let changed = self.0.changed.clone();
        tokio::spawn(async move {
            loop {
                changed.notified().await;
                tokio::time::sleep(REFRESH_DELAY).await;
                let Some(inner) = app.upgrade() else { return };
                if let Err(error) = App(inner).blocking(|app| app.refresh_pending()).await {
                    // 之后可能不再有写入来触发刷新，自己再排一次，隔一个刷新延迟重试。
                    tracing::error!(%error, "刷新快照失败，稍后重试");
                    changed.notify_one();
                }
            }
        });
    }
}
