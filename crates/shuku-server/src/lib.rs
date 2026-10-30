//! Shuku 的扫描和 HTTP 服务。

mod api;
mod decode;
mod images;
mod scan;

use arc_swap::ArcSwap;
use shuku_index::Index;
use shuku_store::Store;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

pub use scan::ScanReport;

#[derive(Debug, Clone)]
pub struct Config {
    pub content_dir: PathBuf,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
    pub listen: SocketAddr,
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
    store: Mutex<Store>,
    index: ArcSwap<Index>,
    scan_lock: Mutex<()>,
}

#[derive(Clone)]
pub struct App(Arc<Inner>);

impl App {
    pub async fn open(mut config: Config) -> Result<Self> {
        tokio::task::spawn_blocking(move || {
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
            let store = Store::open(config.data_dir.join("shuku.sqlite"))?;
            let index = build_index(&store)?;
            Ok(Self(Arc::new(Inner {
                config,
                store: Mutex::new(store),
                index: ArcSwap::from_pointee(index),
                scan_lock: Mutex::new(()),
            })))
        })
        .await?
    }

    pub fn index(&self) -> Arc<Index> {
        self.0.index.load_full()
    }

    fn store(&self) -> Result<MutexGuard<'_, Store>> {
        self.0.store.lock().map_err(|_| Error::Lock)
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

fn build_index(store: &Store) -> Result<Index> {
    let start = Instant::now();
    let mut records = Vec::new();
    store.for_each_archive_record(|record| {
        records.push(record);
        Ok(())
    })?;
    let count = records.len();
    let index = Index::build(records)?;
    tracing::info!(
        count,
        elapsed_ms = start.elapsed().as_millis(),
        "索引构建完成"
    );
    Ok(index)
}
