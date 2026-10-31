//! 搜索和列表页读的内存快照：索引、列表元数据和阅读进度。快照构建后不可变，
//! 数据变化后整个换掉，搜索结果页因此不查数据库。

use crate::{App, Result};
use shuku_index::Index;
use shuku_store::{Category, LibraryArchive, LibraryTag, Progress, Reader, Tag};
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
    /// 索引和列表的构建时间与耗时，只刷新进度时沿用。
    timing: Timing,
    /// 进度变得最勤，单独刷新，不用重建索引和列表。
    progress: Arc<HashMap<i64, Progress>>,
    /// 置顶在前，再按名字。分类很少，改动后立即单独刷新，加进分类后马上能按分类筛到。
    categories: Arc<Vec<Category>>,
}

#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct Timing {
    /// Unix 毫秒。
    built_at: i64,
    archives: usize,
    read_ms: u64,
    build_ms: u64,
}

/// 控制台标签页的一行。
pub(crate) struct NamespaceCount {
    pub(crate) name: String,
    /// 这个命名空间下有多少种标签（至少被一本用到的）。
    pub(crate) tags: usize,
    /// 这个命名空间的标签一共被用了多少次。
    pub(crate) uses: usize,
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
        let elapsed_ms = start.elapsed().as_millis();
        tracing::info!(
            count = library.archives.len(),
            read_ms,
            elapsed_ms,
            "快照构建完成"
        );
        let built_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        Ok(Self {
            timing: Timing {
                built_at,
                archives: library.archives.len(),
                read_ms: read_ms as u64,
                build_ms: (elapsed_ms - read_ms) as u64,
            },
            index: Arc::new(index),
            listing: Arc::new(Listing {
                tags: library.tags,
                archives: library.archives,
            }),
            progress: Arc::new(library.progress),
            categories: Arc::new(library.categories),
        })
    }

    /// 全部阅读进度，按归档 ID。
    pub(crate) fn progress(&self) -> &HashMap<i64, Progress> {
        &self.progress
    }

    pub(crate) fn categories(&self) -> &[Category] {
        &self.categories
    }

    pub(crate) fn timing(&self) -> Timing {
        self.timing
    }

    /// 翻页时间不早于 `since`（Unix 毫秒）的归档数。
    pub(crate) fn read_since(&self, since: i64) -> usize {
        self.progress
            .values()
            .filter(|progress| progress.client_time >= since)
            .count()
    }

    /// 至少被一本用到的标签数和命名空间数。
    pub(crate) fn tag_counts(&self) -> (usize, usize) {
        let namespaces = self.namespaces();
        (namespaces.iter().map(|n| n.tags).sum(), namespaces.len())
    }

    /// 每个命名空间的标签数和使用次数，按使用次数从多到少，同数按名字。
    /// 逐本累加，30 万本、每本 10 个标签时约几十毫秒。
    pub(crate) fn namespaces(&self) -> Vec<NamespaceCount> {
        let tags = &self.listing.tags;
        let mut uses = vec![0_usize; tags.len()];
        for archive in &self.listing.archives {
            for &position in &archive.tags {
                uses[position as usize] += 1;
            }
        }
        // 和搜索一样不分大小写，显示第一个遇到的写法。
        let mut namespaces: HashMap<String, NamespaceCount> = HashMap::new();
        for (tag, &count) in tags.iter().zip(&uses) {
            if count == 0 {
                continue;
            }
            let entry = namespaces
                .entry(tag.tag.namespace.to_lowercase())
                .or_insert_with(|| NamespaceCount {
                    name: tag.tag.namespace.clone(),
                    tags: 0,
                    uses: 0,
                });
            entry.tags += 1;
            entry.uses += count;
        }
        let mut namespaces: Vec<_> = namespaces.into_values().collect();
        namespaces.sort_by(|a, b| b.uses.cmp(&a.uses).then_with(|| a.name.cmp(&b.name)));
        namespaces
    }

    /// 有阅读进度的归档里翻页时间最新的 `limit` 本，从新到旧。进度表里的归档
    /// 不在列表里时（快照之间被删除）跳过。
    pub(crate) fn recently_read(&self, limit: usize) -> Vec<Item<'_>> {
        let mut read: Vec<(i64, i64)> = self
            .progress
            .iter()
            .map(|(&id, progress)| (progress.client_time, id))
            .collect();
        // 时间相同时按 ID 从大到小，结果不随 HashMap 的遍历顺序变化。
        let newest = |a: &(i64, i64), b: &(i64, i64)| b.cmp(a);
        if read.len() > limit {
            read.select_nth_unstable_by(limit, newest);
            read.truncate(limit);
        }
        read.sort_unstable_by(newest);
        read.into_iter()
            .filter_map(|(_, id)| self.item(id))
            .collect()
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
            timing: current.timing,
            progress: Arc::new(progress),
            categories: current.categories.clone(),
        }));
        Ok(())
    }

    /// 分类或成员变了：马上只换掉快照里的分类，不等刷新延迟。
    pub(crate) fn refresh_categories(&self) -> Result<()> {
        let _refresh = self
            .0
            .refresh_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let categories = self.reader()?.categories()?;
        let current = self.snapshot();
        self.0.snapshot.store(Arc::new(Snapshot {
            index: current.index.clone(),
            listing: current.listing.clone(),
            timing: current.timing,
            progress: current.progress.clone(),
            categories: Arc::new(categories),
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
