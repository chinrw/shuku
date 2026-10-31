//! 控制台的接口：状态、任务、扫描问题、重复文件、命名空间统计和日志。
//! M1 没有认证，这些接口和其他接口一样只在局域网里用；会改数据的管理功能放到 M2。

use crate::{App, Error, HashReport, Result, ScanReport, logs};
use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::PoisonError,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// 每类扫描问题最多记这么多条，数量另计。内容目录配错时可能整库都失败，不能全部留在内存里。
const ISSUE_LIMIT: usize = 1000;
/// 图片缓存的大小隔这么久才重新统计一次。
const CACHE_USAGE_TTL: Duration = Duration::from_secs(60);
/// “最近 7 天读过”的窗口。
const RECENT_READ_WINDOW_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(Default)]
pub(crate) struct Tasks {
    pub(crate) scan: ScanTask,
    pub(crate) hash: HashTask,
}

#[derive(Default)]
pub(crate) struct ScanTask {
    running: bool,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    /// 内容目录里的文件数，遍历完目录后才知道。
    pub(crate) total: usize,
    processed: usize,
    report: ScanReport,
    error: Option<String>,
    /// 当前或最近一次扫描遇到的问题，扫描开始时清空。
    pub(crate) issues: ScanIssues,
}

#[derive(Default)]
pub(crate) struct HashTask {
    running: bool,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    /// 开始时还没有完整指纹的归档数。
    total: u64,
    pub(crate) report: HashReport,
    error: Option<String>,
}

#[derive(Default)]
pub(crate) struct ScanIssues {
    pub(crate) failed: Capped<FileIssue>,
    pub(crate) skipped: Capped<String>,
    pub(crate) unreadable: Capped<FileIssue>,
    pub(crate) missing: Capped<MissingArchive>,
}

/// 只留前 [`ISSUE_LIMIT`] 条，总数照常累加。
pub(crate) struct Capped<T> {
    items: Vec<T>,
    total: usize,
}

impl<T> Default for Capped<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: 0,
        }
    }
}

impl<T> Capped<T> {
    pub(crate) fn push(&mut self, item: T) {
        self.total += 1;
        if self.items.len() < ISSUE_LIMIT {
            self.items.push(item);
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct FileIssue {
    /// 相对内容目录。
    pub(crate) path: String,
    pub(crate) error: String,
}

/// 库里有记录、内容目录里已经找不到文件的归档。
#[derive(Clone)]
pub(crate) struct MissingArchive {
    pub(crate) id: i64,
    pub(crate) path: String,
}

#[derive(Clone, Copy)]
pub(crate) struct CacheUsage {
    bytes: u64,
    files: u64,
}

impl ScanTask {
    pub(crate) fn start(&mut self) {
        *self = Self {
            running: true,
            started_at: Some(now_ms()),
            ..Self::default()
        };
    }

    pub(crate) fn progress(&mut self, files: usize, report: &ScanReport) {
        self.processed += files;
        self.report = report.clone();
    }

    pub(crate) fn finish(&mut self, result: &Result<ScanReport>) {
        match result {
            Ok(report) => {
                self.running = false;
                self.finished_at = Some(now_ms());
                self.processed = self.total;
                self.report = report.clone();
            }
            Err(error) => self.fail(error),
        }
    }

    pub(crate) fn fail(&mut self, error: &Error) {
        self.running = false;
        self.finished_at = Some(now_ms());
        self.error = Some(error.to_string());
    }
}

impl HashTask {
    pub(crate) fn start(&mut self, total: u64) {
        *self = Self {
            running: true,
            started_at: Some(now_ms()),
            total,
            ..Self::default()
        };
    }

    pub(crate) fn finish(&mut self, result: &Result<HashReport>) {
        match result {
            Ok(report) => {
                self.running = false;
                self.finished_at = Some(now_ms());
                self.report = report.clone();
            }
            Err(error) => self.fail(error),
        }
    }

    pub(crate) fn fail(&mut self, error: &Error) {
        self.running = false;
        self.finished_at = Some(now_ms());
        self.error = Some(error.to_string());
    }
}

/// 任务线程 panic 时走不到正常的结束，这个守卫在展开时把任务标成中断，
/// 免得控制台一直显示运行中、也不能再触发。它比任务锁先释放，所以仍在锁内。
pub(crate) struct FailOnUnwind<'a> {
    app: &'a App,
    hash: bool,
}

impl<'a> FailOnUnwind<'a> {
    pub(crate) fn scan(app: &'a App) -> Self {
        Self { app, hash: false }
    }

    pub(crate) fn hash(app: &'a App) -> Self {
        Self { app, hash: true }
    }
}

impl Drop for FailOnUnwind<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let error = Error::Write("任务意外中断，详情见日志".into());
        self.app.tasks(|tasks| {
            if self.hash {
                tasks.hash.fail(&error);
            } else {
                tasks.scan.fail(&error);
            }
        });
    }
}

impl App {
    /// 在任务状态锁里执行 `work`。锁只保护几个计数，持有时间很短；中毒时照常使用，
    /// 最坏是控制台显示的计数不准。
    pub(crate) fn tasks<T>(&self, work: impl FnOnce(&mut Tasks) -> T) -> T {
        work(&mut self.0.tasks.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(crate) fn admin_router(&self) -> Router<App> {
        Router::new()
            .route("/api/admin/status", get(status))
            .route("/api/admin/issues", get(issues))
            .route("/api/admin/duplicates", get(duplicates))
            .route("/api/admin/namespaces", get(namespaces))
            .route("/api/admin/logs", get(logs))
            .route("/api/admin/scan", post(start_scan))
            .route("/api/admin/hash", post(start_hash))
            .route("/api/admin/refresh", post(refresh))
            .route("/api/admin/cache", axum::routing::delete(clear_cache))
            .layer(axum::middleware::from_fn(same_origin))
    }

    fn cache_dir(&self) -> std::path::PathBuf {
        self.0.config.data_dir.join("cache/images")
    }

    fn cache_usage(&self) -> Result<CacheUsage> {
        let mut cached = self
            .0
            .cache_usage
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((at, usage)) = *cached
            && at.elapsed() < CACHE_USAGE_TTL
        {
            return Ok(usage);
        }
        let usage = measure(&self.cache_dir())?;
        *cached = Some((Instant::now(), usage));
        Ok(usage)
    }
}

/// 拒绝别的网站发来的写请求。没有认证时，用户浏览的任意网页都能让浏览器向局域网里的
/// 服务端发一个不带请求体的 POST（简单请求不经过 CORS 预检）。浏览器会带上
/// `Sec-Fetch-Site`，旧浏览器至少带 `Origin`；两个都没有的是 curl 之类的工具，放行。
async fn same_origin(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let headers = request.headers();
    let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let cross_site = if request.method().is_safe() {
        false
    } else if let Some(site) = text("sec-fetch-site") {
        !matches!(site, "same-origin" | "none")
    } else if let Some(origin) = text("origin") {
        let origin_host = origin.split_once("://").map_or(origin, |(_, host)| host);
        Some(origin_host) != text("host")
    } else {
        false
    };
    if cross_site {
        return Error::Http {
            status: StatusCode::FORBIDDEN,
            message: "控制台的写操作只接受同源请求".into(),
        }
        .into_response();
    }
    next.run(request).await
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// 已经发布的缓存图片和它们的大小。生成中的图片先写成同目录的临时文件再 rename，
/// 临时文件没有 `.jpg` 扩展名，不算进来，清空时也不碰。目录还不存在时算作空；
/// 遍历期间被删掉的文件跳过。
fn cached_images(directory: &Path) -> Result<Vec<(std::path::PathBuf, u64)>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut images = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "jpg") {
            continue;
        }
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => images.push((path, metadata.len())),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(images)
}

fn measure(directory: &Path) -> Result<CacheUsage> {
    let images = cached_images(directory)?;
    Ok(CacheUsage {
        bytes: images.iter().map(|(_, size)| size).sum(),
        files: images.len() as u64,
    })
}

fn file_size(path: &Path) -> Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

async fn status(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let stats = app.reader()?.stats()?;
        let snapshot = app.snapshot();
        let read_since = now_ms() - RECENT_READ_WINDOW_MS;
        let database = app.0.config.data_dir.join("shuku.sqlite");
        let cache = app.cache_usage()?;
        let (tags, namespaces) = snapshot.tag_counts();
        let config = &app.0.config;
        let (scan, hash) = app.tasks(|tasks| (scan_json(&tasks.scan), hash_json(&tasks.hash)));
        Ok(Json(json!({
            "server": {
                "version": env!("CARGO_PKG_VERSION"),
                "started_at": unix_ms(app.0.started_at),
                "content_dir": config.content_dir, "data_dir": config.data_dir,
                "web_dir": config.web_dir, "listen": config.listen.to_string(),
                "scan_hidden": config.scan_hidden,
            },
            "library": {
                "archives": stats.archives, "pages": stats.pages, "bytes": stats.bytes,
                "hashed": stats.hashed, "lrr_imported": stats.lrr_imported,
                "lrr_pending": stats.lrr_pending, "with_progress": stats.with_progress,
                "read_last_week": snapshot.read_since(read_since),
                "categories": stats.categories, "tankoubons": stats.tankoubons,
                "tags": tags, "namespaces": namespaces,
            },
            "storage": {
                "database_bytes": file_size(&database)?,
                "wal_bytes": file_size(&database.with_extension("sqlite-wal"))?,
                "cache_bytes": cache.bytes, "cache_files": cache.files,
            },
            "snapshot": snapshot.timing(),
            "scan": scan,
            "hash": hash,
        })))
    })
    .await
}

fn scan_json(task: &ScanTask) -> Value {
    json!({
        "running": task.running, "started_at": task.started_at,
        "finished_at": task.finished_at, "total": task.total, "processed": task.processed,
        "report": task.report, "error": task.error,
        "issues": {
            "failed": task.issues.failed.total, "skipped": task.issues.skipped.total,
            "unreadable": task.issues.unreadable.total, "missing": task.issues.missing.total,
        },
    })
}

fn hash_json(task: &HashTask) -> Value {
    json!({
        "running": task.running, "started_at": task.started_at,
        "finished_at": task.finished_at, "total": task.total,
        "report": task.report, "error": task.error,
    })
}

async fn issues(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let snapshot = app.snapshot();
        Ok(Json(app.tasks(|tasks| {
            let issues = &tasks.scan.issues;
            let missing: Vec<_> = issues
                .missing
                .items
                .iter()
                .map(|archive| {
                    let title = snapshot
                        .item(archive.id)
                        .map(|item| item.archive.title.clone());
                    json!({"id": archive.id, "path": archive.path, "title": title})
                })
                .collect();
            let skipped: Vec<_> = issues
                .skipped
                .items
                .iter()
                .map(|path| json!({"path": path}))
                .collect();
            json!({
                "scanning": tasks.scan.running, "finished_at": tasks.scan.finished_at,
                "failed": {"total": issues.failed.total, "items": issues.failed.items},
                "skipped": {"total": issues.skipped.total, "items": skipped},
                "unreadable": {"total": issues.unreadable.total, "items": issues.unreadable.items},
                "missing": {"total": issues.missing.total, "items": missing},
            })
        })))
    })
    .await
}

#[derive(Default, Deserialize)]
struct DuplicateParams {
    limit: Option<usize>,
}

async fn duplicates(
    State(app): State<App>,
    params: std::result::Result<Query<DuplicateParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let limit = params.limit.unwrap_or(100);
    if !(1..=500).contains(&limit) {
        return Err(Error::bad_request("limit 必须在 1 到 500 之间"));
    }
    app.blocking(move |app| {
        let (total, groups) = app.reader()?.duplicates(limit)?;
        let groups: Vec<_> = groups
            .into_iter()
            .map(|files| {
                let files: Vec<_> = files
                    .into_iter()
                    .map(|file| {
                        json!({"id": file.id, "path": file.path, "title": file.title,
                            "file_size": file.file_size})
                    })
                    .collect();
                json!({ "files": files })
            })
            .collect();
        Ok(Json(json!({"total": total, "groups": groups})))
    })
    .await
}

async fn namespaces(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let namespaces: Vec<_> = app
            .snapshot()
            .namespaces()
            .into_iter()
            .map(|namespace| {
                json!({"namespace": namespace.name, "tags": namespace.tags, "uses": namespace.uses})
            })
            .collect();
        Ok(Json(json!({ "namespaces": namespaces })))
    })
    .await
}

#[derive(Default, Deserialize)]
struct LogParams {
    level: Option<String>,
    #[serde(default)]
    after: u64,
    limit: Option<usize>,
}

async fn logs(
    State(app): State<App>,
    params: std::result::Result<Query<LogParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let level = match params.level.as_deref() {
        None | Some("info") => tracing::Level::INFO,
        Some("warn") => tracing::Level::WARN,
        Some("error") => tracing::Level::ERROR,
        _ => return Err(Error::bad_request("level 必须是 info、warn 或 error")),
    };
    let limit = params.limit.unwrap_or(200);
    if !(1..=1000).contains(&limit) {
        return Err(Error::bad_request("limit 必须在 1 到 1000 之间"));
    }
    let entries: Vec<_> = logs::recent(level, params.after, limit)
        .into_iter()
        .map(|entry| {
            json!({"seq": entry.seq, "time": entry.time,
                "level": entry.level.as_str().to_ascii_lowercase(), "message": entry.message})
        })
        .collect();
    // 序号每次启动从 1 开始，客户端看到 started_at 变了就要丢掉旧的游标。
    Ok(Json(
        json!({ "entries": entries, "started_at": unix_ms(app.0.started_at) }),
    ))
}

fn already_running(task: &str) -> Error {
    Error::Http {
        status: StatusCode::CONFLICT,
        message: format!("{task}已在运行"),
    }
}

/// 在后台扫描，结束后接着补算指纹，和服务启动时一样。立即返回。
async fn start_scan(State(app): State<App>) -> Result<(StatusCode, Json<Value>)> {
    let claimed = app.tasks(|tasks| {
        if tasks.scan.running {
            return false;
        }
        // 先占住，连续点两次时第二次就会被拒绝，不必等后台任务真正开始。
        tasks.scan.running = true;
        true
    });
    if !claimed {
        return Err(already_running("扫描"));
    }
    tokio::spawn(async move {
        match app.scan().await {
            Ok(_) => {
                if let Err(error) = app.hash_contents().await {
                    tracing::error!(%error, "计算完整指纹失败");
                }
            }
            Err(error) => tracing::error!(%error, "扫描失败"),
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"started": true}))))
}

async fn start_hash(State(app): State<App>) -> Result<(StatusCode, Json<Value>)> {
    let claimed = app.tasks(|tasks| {
        if tasks.hash.running {
            return false;
        }
        tasks.hash.running = true;
        true
    });
    if !claimed {
        return Err(already_running("补算指纹"));
    }
    tokio::spawn(async move {
        if let Err(error) = app.hash_contents().await {
            tracing::error!(%error, "计算完整指纹失败");
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"started": true}))))
}

async fn refresh(State(app): State<App>) -> Result<Json<Value>> {
    app.refresh().await?;
    Ok(Json(json!({ "snapshot": app.snapshot().timing() })))
}

/// 逐个删除已发布的缓存图片。目录和生成中的临时文件保留：正在生成的图片要把临时文件
/// rename 进这个目录，删掉任何一个都会让那次请求失败。
async fn clear_cache(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let mut removed = CacheUsage { bytes: 0, files: 0 };
        for (path, size) in cached_images(&app.cache_dir())? {
            match fs::remove_file(path) {
                Ok(()) => {
                    removed.bytes += size;
                    removed.files += 1;
                }
                // 另一个请求刚好也在清。
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        *app.0
            .cache_usage
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        Ok(Json(
            json!({"removed_files": removed.files, "removed_bytes": removed.bytes}),
        ))
    })
    .await
}

#[cfg(test)]
mod tests {
    use crate::{App, Config};
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn a_second_trigger_while_a_task_runs_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            content_dir: root.path().join("content"),
            data_dir: root.path().join("data"),
            web_dir: root.path().join("web"),
            listen: "127.0.0.1:0".parse().unwrap(),
            scan_hidden: false,
        };
        std::fs::create_dir(&config.content_dir).unwrap();
        let app = App::open(config).await.unwrap();
        // 模拟一个正在运行的扫描和补算，不真的启动它们。
        app.tasks(|tasks| {
            tasks.scan.running = true;
            tasks.hash.running = true;
        });
        for uri in ["/api/admin/scan", "/api/admin/hash"] {
            let response = app
                .router()
                .oneshot(Request::post(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::CONFLICT, "{uri}");
        }
    }
}
