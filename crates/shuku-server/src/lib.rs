//! Shuku 的扫描和 HTTP 服务。

mod admin;
mod api;
mod categories;
mod db;
mod decode;
mod download;
mod edit;
mod hash;
mod hosts;
mod images;
mod logs;
mod scan;
mod snapshot;
mod webtoon;

use arc_swap::ArcSwap;
use shuku_index::Index;
use shuku_store::{PooledReader, Writer};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
};
use tokio::sync::Notify;

pub use hash::HashReport;
pub use logs::{LogLayer, log_layer};
pub use scan::ScanReport;

#[derive(Debug, Clone)]
pub struct Config {
    pub content_dir: PathBuf,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
    pub listen: SocketAddr,
    /// 也扫描以 `.` 开头的目录和文件。默认跳过：它们通常是下载工具的临时目录，
    /// 往往只有那个程序自己能读。
    pub scan_hidden: bool,
    /// 按页面宽高比判断条漫，让网页端自动改用上下连续滚动。判断错得多时可以关掉，
    /// 关掉后只认阅读器里手动打开的条漫开关。
    pub detect_webtoons: bool,
    /// 除 IP、localhost 和不带点的主机名以外，还接受这些 Host（例如 Tailscale 的完整域名）。
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Avif(#[from] avif_decode::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Resize(#[from] fast_image_resize::ResizeError),
    #[error("{message}")]
    Http {
        status: axum::http::StatusCode,
        message: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] shuku_store::Error),
    #[error(transparent)]
    Archive(#[from] shuku_archive::Error),
    #[error(transparent)]
    Index(#[from] shuku_index::BuildError),
    #[error(transparent)]
    Walk(#[from] walkdir::Error),
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
    #[error("数据库锁已损坏")]
    Lock,
    #[error("写入失败：{0}")]
    Write(String),
    #[error("配置错误：{0}")]
    Config(String),
    #[error("文件不可用：{0}")]
    File(String),
}

impl Error {
    fn bad_request(message: impl ToString) -> Self {
        Self::Http {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    fn not_found() -> Self {
        Self::Http {
            status: axum::http::StatusCode::NOT_FOUND,
            message: "资源不存在".into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

struct Inner {
    config: Config,
    database: Arc<db::Database>,
    /// 进度上报和指纹补算经这里合并提交；扫描直接调用 [`db::Database::write`]。
    writes: SyncSender<db::Request>,
    snapshot: ArcSwap<snapshot::Snapshot>,
    /// 让快照的刷新一个接一个进行。
    refresh_lock: Mutex<()>,
    /// 写入了快照里的数据时通知，由后台任务延迟刷新快照。
    changed: Arc<Notify>,
    /// 置位表示列表元数据或标签变了，下次刷新要重建整个快照；否则只刷新进度。
    listing_changed: AtomicBool,
    scan_lock: Mutex<()>,
    /// 补算指纹一次只跑一个。启动时的补算和控制台触发的补算撞上时，后一个等前一个结束，
    /// 那时剩下要算的已经不多。
    hash_lock: Mutex<()>,
    /// 扫描和补算指纹的进度，控制台读它。
    tasks: Mutex<admin::Tasks>,
    started_at: std::time::SystemTime,
    /// 图片缓存的大小要遍历目录才知道，控制台每 5 秒问一次，结果缓存一会儿。
    cache_usage: Mutex<Option<(std::time::Instant, admin::CacheUsage)>>,
    /// 单元测试用：扫描每提交一批后调用，让扫描停在中途。
    #[cfg(test)]
    scan_batch_hook: Mutex<Option<Box<dyn FnMut() + Send>>>,
    /// 置位后后台的长任务在下一个检查点返回，进程退出时不用等它们跑完。
    stopping: AtomicBool,
}

#[derive(Clone)]
pub struct App(Arc<Inner>);

impl App {
    pub async fn open(mut config: Config) -> Result<Self> {
        let app = tokio::task::spawn_blocking(move || {
            config.content_dir = config.content_dir.canonicalize()?;
            if !config.content_dir.is_dir() {
                return Err(Error::Config("内容路径必须是目录".into()));
            }
            config.data_dir = destination(&std::path::absolute(&config.data_dir)?)?;
            if config.data_dir.starts_with(&config.content_dir)
                || config.content_dir.starts_with(&config.data_dir)
            {
                return Err(Error::Config(
                    "内容目录和数据目录必须分开，不能互相包含".into(),
                ));
            }
            std::fs::create_dir_all(&config.data_dir)?;
            let database = Arc::new(db::Database::open(&config.data_dir.join("shuku.sqlite"))?);
            let snapshot = snapshot::Snapshot::load(&*database.reader()?)?;
            let changed = Arc::new(Notify::new());
            let writes = db::spawn_writer(database.clone(), changed.clone())?;
            Ok(Self(Arc::new(Inner {
                config,
                database,
                writes,
                snapshot: ArcSwap::from_pointee(snapshot),
                refresh_lock: Mutex::new(()),
                changed,
                listing_changed: AtomicBool::new(false),
                scan_lock: Mutex::new(()),
                hash_lock: Mutex::new(()),
                tasks: Mutex::new(admin::Tasks::default()),
                started_at: std::time::SystemTime::now(),
                cache_usage: Mutex::new(None),
                #[cfg(test)]
                scan_batch_hook: Mutex::new(None),
                stopping: AtomicBool::new(false),
            })))
        })
        .await??;
        app.spawn_progress_refresher();
        Ok(app)
    }

    /// 让后台的长任务尽快停下。tokio 退出时会等 spawn_blocking 的任务结束，
    /// 不停下的话，按 Ctrl-C 要等补算指纹跑完整个书库。
    pub fn stop(&self) {
        self.0.stopping.store(true, Ordering::Relaxed);
    }

    pub fn index(&self) -> Arc<Index> {
        self.snapshot().index.clone()
    }

    fn reader(&self) -> Result<PooledReader<'_>> {
        self.0.database.reader()
    }

    fn write<T>(&self, work: impl FnOnce(&mut Writer<'_>) -> Result<T>) -> Result<T> {
        self.0.database.write(work)
    }

    /// 写队列满时阻塞，所以只在阻塞线程里调用。
    fn enqueue(&self, request: db::Request) -> Result<()> {
        self.0.writes.send(request).map_err(|_| db::queue_stopped())
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(Self) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let app = self.clone();
        tokio::task::spawn_blocking(move || work(app)).await?
    }
}

fn destination(path: &std::path::Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| Error::Config("数据目录缺少父路径".into()))?;
            let name = path
                .file_name()
                .ok_or_else(|| Error::Config("数据目录路径无效".into()))?;
            Ok(destination(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}
