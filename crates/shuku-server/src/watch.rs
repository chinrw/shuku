//! 监视内容目录：有文件新增、改动、删除或移动时，等目录安静一会儿再整体扫描一次。
//! 只通过 [`App::scan`] 和 [`App::hash_contents`] 进入扫描，不重复扫描本身的逻辑。
//! 整体扫描对没变的文件只比较大小和修改时间，所以不按单个文件增量处理。
//!
//! 目录由这里逐个加监视，不用 notify 的递归模式：递归模式遇到一个加不上的子目录
//! （例如下载工具建的、只有它自己能读的隐藏临时目录）就整体失败。逐个加时按扫描的
//! 规则跳过隐藏目录，加不上的目录记一条警告，其余照常监视。

use crate::{App, Result};
use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode, ModifyKind, RenameMode},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

/// 最后一个事件之后安静这么久才扫描。下载工具和复制大文件时事件连续不断，
/// 等它们写完再扫，免得读到写了一半的压缩包。
pub const QUIET: Duration = Duration::from_secs(10);
/// 事件一直不停时，从第一个事件起最多等这么久也扫一次。
pub const MAX_DELAY: Duration = Duration::from_secs(120);

/// 收到的变化。监视回调只更新它，不排队，复制几十万个文件也不会堆积。
struct Changes {
    signal: Notify,
    /// 收到过的事件数。扫描开始前记下它，之后再变才需要下一次扫描。
    count: AtomicU64,
    /// 最后一个事件距 `epoch` 的毫秒数。
    last_ms: AtomicU64,
    epoch: Instant,
    /// 句柄已经丢掉，后台任务该退出了。
    stopped: AtomicBool,
    /// 新出现的目录，后台任务在扫描前给它们加监视。回调运行在 notify 的事件线程上，
    /// 在那里加监视会等事件线程自己，所以只能交给别的线程。
    new_dirs: Mutex<Vec<PathBuf>>,
    /// 事件队列溢出过，新目录的事件可能丢了，扫描前给整个目录树重新加一遍监视。
    rewatch: AtomicBool,
}

impl Changes {
    fn record(&self) {
        let elapsed = self.epoch.elapsed().as_millis() as u64;
        self.last_ms.store(elapsed, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
        self.signal.notify_one();
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn since_last(&self) -> Duration {
        let last = Duration::from_millis(self.last_ms.load(Ordering::Relaxed));
        self.epoch.elapsed().saturating_sub(last)
    }
}

/// 监视内容目录的句柄。丢掉时停止监视，后台任务在当前这次扫描结束后退出，
/// 不会在扫描中途被取消，扫描状态不会卡在“进行中”。
pub struct ContentWatcher {
    changes: Arc<Changes>,
}

impl Drop for ContentWatcher {
    fn drop(&mut self) {
        self.changes.stopped.store(true, Ordering::Release);
        self.changes.signal.notify_one();
    }
}

impl App {
    /// 开始监视内容目录，返回的句柄丢掉时停止。监视器建不起来时（例如 inotify 的
    /// 监视数用完，或内容目录在不支持 inotify 的网络文件系统上）返回错误，调用方可以
    /// 不监视、照常运行。`quiet` 和 `max_delay` 见 [`QUIET`]、[`MAX_DELAY`]。
    pub fn watch_content(&self, quiet: Duration, max_delay: Duration) -> Result<ContentWatcher> {
        let changes = Arc::new(Changes {
            signal: Notify::new(),
            count: AtomicU64::new(0),
            last_ms: AtomicU64::new(0),
            epoch: Instant::now(),
            stopped: AtomicBool::new(false),
            new_dirs: Mutex::new(Vec::new()),
            rewatch: AtomicBool::new(false),
        });
        let content = self.0.config.content_dir.clone();
        // 是否扫描隐藏条目可以在控制台里随时改，每个事件现读。
        let (recorder, app) = (changes.clone(), self.clone());
        let mut watcher = RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| match event {
                Ok(event) => {
                    if event.need_rescan() {
                        recorder.rewatch.store(true, Ordering::Release);
                    }
                    if adds_directory(&event) {
                        // 不跟随符号链接：指向内容目录以外的链接不该加监视。
                        let dirs = event
                            .paths
                            .iter()
                            .filter(|path| {
                                std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
                            })
                            .cloned();
                        recorder
                            .new_dirs
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .extend(dirs);
                    }
                    let scan_hidden = app.settings().scan.scan_hidden;
                    if event.need_rescan() || is_change(&event, &content, scan_hidden) {
                        recorder.record();
                    }
                }
                // 出错时可能漏掉了变化，按有变化处理，扫一遍就补上了。
                Err(error) => {
                    tracing::warn!(%error, "监视内容目录出错");
                    recorder.rewatch.store(true, Ordering::Release);
                    recorder.record();
                }
            },
            notify::Config::default(),
        )
        .map_err(watch_error)?;
        let content = &self.0.config.content_dir;
        // 内容目录本身加不上就没法监视；子目录加不上只记警告。
        watcher
            .watch(content, RecursiveMode::NonRecursive)
            .map_err(watch_error)?;
        let scan_hidden = self.settings().scan.scan_hidden;
        watch_tree(&mut watcher, content, content, scan_hidden);
        let watcher = Arc::new(Mutex::new(watcher));
        tokio::spawn(self.clone().watch_loop(
            watcher,
            changes.clone(),
            scan_hidden,
            quiet,
            max_delay,
        ));
        tracing::info!(content = %content.display(), "开始监视内容目录");
        Ok(ContentWatcher { changes })
    }

    /// 监视器归这个任务所有，任务退出时停止监视。
    async fn watch_loop(
        self,
        watcher: Arc<Mutex<RecommendedWatcher>>,
        changes: Arc<Changes>,
        mut watched_hidden: bool,
        quiet: Duration,
        max_delay: Duration,
    ) {
        let hashing = Arc::new(HashQueue::default());
        let mut handled = 0;
        loop {
            changes.signal.notified().await;
            if changes.is_stopped() {
                return;
            }
            // 扫描期间到达的事件会留下一个通知；它们要是都在扫描开始前，就不用再扫。
            if changes.count.load(Ordering::Acquire) == handled {
                continue;
            }
            let first = Instant::now();
            // 等到安静了 `quiet`，或者从第一个事件起过了 `max_delay`。两次读时钟之间
            // 可能已经越过界限，所以剩余时间用 saturating_sub，到零就不再等。
            loop {
                let wait = quiet
                    .saturating_sub(changes.since_last())
                    .min(max_delay.saturating_sub(first.elapsed()));
                if wait.is_zero() || changes.is_stopped() {
                    break;
                }
                tokio::time::sleep(wait).await;
            }
            // 控制台里改了是否扫描隐藏条目，要监视的目录也跟着变。
            let scan_hidden = self.settings().scan.scan_hidden;
            if scan_hidden != watched_hidden {
                changes.rewatch.store(true, Ordering::Release);
                watched_hidden = scan_hidden;
            }
            self.add_watches(&watcher, &changes, scan_hidden).await;
            if !self.claim_scan(&changes, quiet).await {
                return;
            }
            handled = changes.count.load(Ordering::Acquire);
            tracing::info!("内容目录有变化，开始扫描");
            if let Err(error) = self.scan().await {
                tracing::error!(%error, "扫描失败");
                continue;
            }
            hashing.request(&self);
        }
    }

    /// 给新出现的目录加监视；事件队列溢出过时给整个目录树重新加一遍。目录被删掉、
    /// 改名后旧路径上的监视由 inotify 自己撤掉，这里不用管。
    async fn add_watches(
        &self,
        watcher: &Arc<Mutex<RecommendedWatcher>>,
        changes: &Changes,
        scan_hidden: bool,
    ) {
        let rewatch = changes.rewatch.swap(false, Ordering::AcqRel);
        let content = self.0.config.content_dir.clone();
        let roots = if rewatch {
            vec![content.clone()]
        } else {
            std::mem::take(
                &mut *changes
                    .new_dirs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            )
        };
        if roots.is_empty() {
            return;
        }
        let watcher = watcher.clone();
        let added = tokio::task::spawn_blocking(move || {
            let mut watcher = watcher.lock().unwrap_or_else(PoisonError::into_inner);
            for root in roots {
                watch_tree(&mut watcher, &root, &content, scan_hidden);
            }
        })
        .await;
        if let Err(error) = added {
            tracing::error!(%error, "给新目录加监视失败");
        }
    }

    /// 和控制台的“立即扫描”一样先占住扫描状态。已经有扫描在跑时等它结束：
    /// 它可能在这次变化之前就遍历完了目录。返回 `false` 表示等待期间停止了监视或
    /// 服务正在退出，这时没有占住扫描状态。
    async fn claim_scan(&self, changes: &Changes, retry: Duration) -> bool {
        loop {
            if changes.is_stopped() || self.0.stopping.load(Ordering::Relaxed) {
                return false;
            }
            if self.tasks(|tasks| tasks.scan.claim()) {
                return true;
            }
            tokio::time::sleep(retry).await;
        }
    }
}

/// 扫描后的指纹补算。补算可能要跑几个小时，单独跑，不挡下一次扫描。正在跑时又有
/// 请求，就在它结束后再跑一轮：一轮只往 ID 大的方向走，跑到一半时改动了已经走过的
/// 归档，要下一轮才补得上。
#[derive(Default)]
struct HashQueue {
    running: AtomicBool,
    again: AtomicBool,
}

impl HashQueue {
    fn request(self: &Arc<Self>, app: &App) {
        self.again.store(true, Ordering::Release);
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let (queue, app) = (self.clone(), app.clone());
        tokio::spawn(async move {
            loop {
                while queue.again.swap(false, Ordering::AcqRel) {
                    if let Err(error) = app.hash_contents().await {
                        tracing::error!(%error, "计算完整指纹失败");
                    }
                }
                queue.running.store(false, Ordering::Release);
                // 清掉 `running` 之前到达的请求看到它还在跑，没有另起任务，由这里接着跑。
                if !queue.again.load(Ordering::Acquire)
                    || queue.running.swap(true, Ordering::AcqRel)
                {
                    return;
                }
            }
        });
    }
}

/// 给 `root` 和它下面的目录各加一个非递归的监视，跳过的目录和扫描一致：不跟随符号链接，
/// 不扫隐藏条目时也不进隐藏目录。加不上的目录记一条警告。
fn watch_tree(watcher: &mut RecommendedWatcher, root: &Path, content: &Path, scan_hidden: bool) {
    let walk = walkdir::WalkDir::new(root)
        .follow_root_links(false)
        .into_iter()
        .filter_entry(|entry| scan_hidden || !is_hidden(entry.path(), content));
    for entry in walk {
        let path = match entry {
            Ok(entry) if entry.file_type().is_dir() => entry.into_path(),
            Ok(_) => continue,
            Err(error) => {
                tracing::warn!(%error, "无法列出内容目录里的子目录，不监视它");
                continue;
            }
        };
        if let Err(error) = watcher.watch(&path, RecursiveMode::NonRecursive) {
            tracing::warn!(path = %path.display(), %error, "无法监视目录");
        }
    }
}

/// 新建或移进来的条目，可能是要加监视的目录。
fn adds_directory(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Name(
                RenameMode::To | RenameMode::Both | RenameMode::Any
            ))
    )
}

fn watch_error(error: notify::Error) -> crate::Error {
    crate::Error::Config(format!("无法监视内容目录：{error}"))
}

/// 会影响书库的事件。扫描本身要打开每个文件，打开和读取的事件不算，否则每次扫描
/// 都会触发下一次扫描。属性变化算：inotify 分不出改的是权限还是修改时间，而扫描靠
/// 修改时间认出变化。跳过隐藏条目时，隐藏目录里的变化（下载工具的临时目录）不算。
fn is_change(event: &notify::Event, content: &Path, scan_hidden: bool) -> bool {
    let relevant = match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => true,
        EventKind::Any | EventKind::Other => true,
    };
    relevant
        && (scan_hidden
            || event.paths.is_empty()
            || event.paths.iter().any(|path| !is_hidden(path, content)))
}

fn is_hidden(path: &Path, content: &Path) -> bool {
    path.strip_prefix(content).is_ok_and(|relative| {
        relative
            .components()
            .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use crate::{App, Config};
    use std::time::Duration;
    use tower::ServiceExt;

    async fn scan_started_at(app: &App) -> serde_json::Value {
        let request = axum::http::Request::get("/api/admin/status")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.router().oneshot(request).await.unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["scan"]["started_at"].clone()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_the_watcher_while_it_waits_for_a_scan_starts_none() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            content_dir: root.path().join("content"),
            data_dir: root.path().join("data"),
            web_dir: root.path().join("web"),
            listen: "127.0.0.1:0".parse().unwrap(),
        };
        std::fs::create_dir(&config.content_dir).unwrap();
        let app = App::open(config.clone()).await.unwrap();
        // 假装有一个扫描正在跑，监视任务只能等着。
        assert!(app.tasks(|tasks| tasks.scan.claim()));
        let quiet = Duration::from_millis(50);
        let watcher = app.watch_content(quiet, Duration::from_secs(1)).unwrap();
        std::fs::write(config.content_dir.join("a.cbz"), b"x").unwrap();
        tokio::time::sleep(quiet * 6).await;
        drop(watcher);
        app.tasks(|tasks| tasks.scan.finish(&Ok(Default::default())));
        tokio::time::sleep(quiet * 6).await;
        assert_eq!(scan_started_at(&app).await, serde_json::Value::Null);
        // 等待已经结束，扫描状态没有被占着。
        assert!(app.tasks(|tasks| tasks.scan.claim()));
    }
}
