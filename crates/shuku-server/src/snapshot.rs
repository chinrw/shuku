//! 搜索和列表页读的内存快照：索引、列表元数据和阅读进度。快照构建后不可变，
//! 数据变化后整个换掉，搜索结果页因此不查数据库。

use crate::{App, Result};
use shuku_index::Index;
use shuku_store::{LibraryArchive, LibraryTag, Progress, Reader, Tag};
use std::{
    collections::HashMap,
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};

/// 进度上报后等这么久再刷新快照，期间的上报合并成一次刷新。读可以滞后 10 到 20 秒。
const REFRESH_DELAY: Duration = Duration::from_secs(10);

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

    /// 立即按数据库重建整个快照，不等进度刷新的延迟。扫描结束时调用；
    /// 测试用它在写入后马上检查搜索结果。
    pub async fn refresh(&self) -> Result<()> {
        self.blocking(|app| app.refresh_blocking()).await
    }

    pub(crate) fn refresh_blocking(&self) -> Result<()> {
        // 刷新串行进行，并且在拿到锁以后才读库，后换上的快照不会比先换上的旧。
        let _refresh = self
            .0
            .refresh_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let snapshot = Snapshot::load(&*self.reader()?)?;
        self.0.snapshot.store(Arc::new(snapshot));
        Ok(())
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

    /// 写队列每次提交了被接受的进度就通知一次；这里等 [`REFRESH_DELAY`] 再刷新，
    /// 等待期间的通知合并成一次。任务只持有弱引用，不会让 `App` 一直活着。
    pub(crate) fn spawn_progress_refresher(&self) {
        let app = Arc::downgrade(&self.0);
        let changed = self.0.progress_changed.clone();
        tokio::spawn(async move {
            loop {
                changed.notified().await;
                tokio::time::sleep(REFRESH_DELAY).await;
                let Some(inner) = app.upgrade() else { return };
                if let Err(error) = App(inner).blocking(|app| app.refresh_progress()).await {
                    tracing::error!(%error, "刷新阅读进度失败");
                }
            }
        });
    }
}
