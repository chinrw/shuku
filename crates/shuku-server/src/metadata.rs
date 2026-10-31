//! 内置的元数据功能：文件名解析、ComicInfo.xml、E-Hentai/ExHentai 查找和标签中文翻译，
//! 取代 LRR 的同名插件。解析和联网在 `shuku-metadata` 里，这里负责设置、扫描时自动运行、
//! 后台的 E-Hentai 队列和手动运行的接口。结果都经 [`crate::edit::ArchiveEdit`] 写入，
//! 和 LRR 一样只加标签、不删已有的标签。
//!
//! 扫描新增的书（不是从 LRR 迁移来的）先在扫描的事务里跑文件名和 ComicInfo，
//! 打开 E-Hentai 自动运行时再放进 `metadata_queue`，由后台逐本查。

use crate::{
    App, Error, Result,
    edit::{ArchiveEdit, TagEdit},
};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shuku_metadata::{
    Metadata, comicinfo,
    ehentai::{self, Cookies, EhOptions},
    filename::{FilenameOptions, FilenameParser},
    translate::Translations,
};
use std::{
    io::Read,
    path::{Path as FilePath, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime},
};
use tokio::{sync::Notify, time::Instant};

/// 设置表里的键。
const SETTING_KEY: &str = "metadata";
/// ComicInfo.xml 一般只有几 KiB，超过这个大小当作损坏。
const COMICINFO_LIMIT: u64 = 1024 * 1024;
/// EhTagTranslation 的数据库，放在数据目录里。
const TRANSLATION_FILE: &str = "db.text.json";
/// 翻译数据库约 4 MiB，超过这个大小多半选错了文件。
const TRANSLATION_LIMIT: u64 = 64 * 1024 * 1024;
/// 两次 E-Hentai 查找之间至少隔这么久，和 LRR 给这个插件设的冷却时间一样。
const EH_COOLDOWN: Duration = Duration::from_secs(4);
/// E-Hentai 提示请求太快时多等一会儿。LRR 随机等 15 到 50 秒。
const EH_SLOW_DOWN: Duration = Duration::from_secs(40);
/// 后台队列读库出错时，隔这么久再试。
const RETRY_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    /// 用找到的标题替换原标题，对应 LRR 的“允许插件替换标题”。
    pub(crate) replace_title: bool,
    pub(crate) filename: FilenameSettings,
    pub(crate) comicinfo: ComicInfoSettings,
    pub(crate) ehentai: EhSettings,
    pub(crate) translation: TranslationSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            replace_title: true,
            filename: FilenameSettings::default(),
            comicinfo: ComicInfoSettings::default(),
            ehentai: EhSettings::default(),
            translation: TranslationSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FilenameSettings {
    /// 扫描到新书时自动运行。
    pub(crate) auto: bool,
    #[serde(flatten)]
    pub(crate) options: FilenameOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ComicInfoSettings {
    pub(crate) auto: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct EhSettings {
    /// 扫描到新书时放进后台队列。
    pub(crate) auto: bool,
    #[serde(flatten)]
    pub(crate) options: EhOptions,
    /// 只写不读：设置接口只返回每个 cookie 有没有设置。
    pub(crate) cookies: Cookies,
}

/// E-Hentai 的结果译成中文，代替 LRR 的 ETagCN。数据库固定是数据目录里的
/// [`TRANSLATION_FILE`]：路径不能从没有认证的设置接口改，否则任何人都能让服务端去读别的文件。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TranslationSettings {
    pub(crate) enabled: bool,
}

/// 设置、翻译表和后台队列的状态。
pub(crate) struct MetadataState {
    settings: Mutex<Arc<Settings>>,
    /// 按路径和修改时间缓存，文件换了下次用时重新载入。
    translations: Mutex<Option<CachedTranslations>>,
    /// 下一次 E-Hentai 查找最早的时间。手动运行和后台队列共用，持有锁的那个在查。
    next_lookup: tokio::sync::Mutex<Option<Instant>>,
    /// 队列里加了书、设置变了或者要求继续时叫醒后台队列。
    wake: Notify,
    queue: Mutex<QueueStatus>,
}

struct CachedTranslations {
    path: PathBuf,
    modified: SystemTime,
    translations: Arc<Translations>,
}

/// 控制台显示的队列状态，只在内存里。
#[derive(Debug, Default, Clone, Serialize)]
struct QueueStatus {
    /// 正在查的归档。
    current: Option<i64>,
    done: u64,
    failed: u64,
    last_error: Option<String>,
    /// 遇到 IP 被封、cookie 失效或者网络不通时停下，原因写在这里，那本书留在队列里。
    /// 改设置或在控制台点“继续”后重新开始。
    paused: Option<String>,
}

impl MetadataState {
    pub(crate) fn load(reader: &shuku_store::Reader) -> Result<Self> {
        let settings = reader.setting::<Settings>(SETTING_KEY)?.unwrap_or_default();
        Ok(Self {
            settings: Mutex::new(Arc::new(settings)),
            translations: Mutex::new(None),
            next_lookup: tokio::sync::Mutex::new(None),
            wake: Notify::new(),
            queue: Mutex::new(QueueStatus::default()),
        })
    }
}

/// 一次扫描里用的本地功能，开始扫描时按当时的设置准备好。
pub(crate) struct ScanPlugins {
    filename: Option<FilenameParser>,
    comicinfo: bool,
    replace_title: bool,
    queue_ehentai: bool,
}

impl ScanPlugins {
    /// 读新文件里的 ComicInfo.xml。读不了或解析不了只记日志，不影响入库。
    pub(crate) fn comicinfo(&self, path: &FilePath) -> Option<Metadata> {
        if !self.comicinfo {
            return None;
        }
        match read_comicinfo(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "ComicInfo.xml 读取失败");
                None
            }
        }
    }

    /// 新入库的书要写的元数据。结果通不过校验时只记日志，这本书照常入库。
    /// `name` 是去掉扩展名的文件名，`relative` 只用来记日志。
    pub(crate) fn edit_for_new(
        &self,
        relative: &str,
        name: &str,
        comicinfo: Option<Metadata>,
    ) -> Option<ArchiveEdit> {
        let mut metadata = Metadata::default();
        if let Some(parser) = &self.filename
            && let Some(parsed) = parser.parse(name)
        {
            metadata.merge(parsed);
        }
        if let Some(comicinfo) = comicinfo {
            metadata.merge(comicinfo);
        }
        if metadata.is_empty() {
            return None;
        }
        let edit = edit_from(metadata, self.replace_title, false);
        match edit.validate() {
            Ok(()) => Some(edit),
            Err(error) => {
                tracing::warn!(path = relative, %error, "文件名或 ComicInfo 的元数据无效，没有写入");
                None
            }
        }
    }

    pub(crate) fn queues_ehentai(&self) -> bool {
        self.queue_ehentai
    }
}

impl App {
    pub(crate) fn metadata_router(&self) -> Router<App> {
        Router::new()
            .route("/api/archives/{id}/metadata", post(run))
            .route(
                "/api/admin/settings/metadata",
                get(get_settings).put(put_settings),
            )
            .route("/api/admin/metadata", get(status))
            .route("/api/admin/metadata/resume", post(resume))
            .route(
                "/api/admin/metadata/queue",
                axum::routing::delete(clear_queue),
            )
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }

    fn metadata_settings(&self) -> Arc<Settings> {
        self.0
            .metadata
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn scan_plugins(&self) -> Result<ScanPlugins> {
        let settings = self.metadata_settings();
        Ok(ScanPlugins {
            filename: settings
                .filename
                .auto
                .then(|| FilenameParser::new(&settings.filename.options))
                .transpose()?,
            comicinfo: settings.comicinfo.auto,
            replace_title: settings.replace_title,
            queue_ehentai: settings.ehentai.auto,
        })
    }

    /// 扫描把新书放进队列后调用。
    pub(crate) fn metadata_queued(&self) {
        self.0.metadata.wake.notify_one();
    }

    /// 在后台逐本处理 E-Hentai 队列，服务运行期间一直在。只在 HTTP 服务里、启动扫描之后
    /// 启动：命令行的 `shuku scan` 扫完就退出，不该开始联网；停机期间移动或改过的文件
    /// 要等扫描更新了记录再查，否则按旧路径读不到封面。
    pub fn spawn_metadata_worker(&self) {
        let app = self.clone();
        tokio::spawn(async move { app.metadata_worker().await });
    }

    async fn metadata_worker(&self) {
        loop {
            let next = match self.next_queued().await {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(%error, "读元数据队列失败");
                    tokio::time::sleep(RETRY_DELAY).await;
                    continue;
                }
            };
            let Some(id) = next else {
                self.0.metadata.wake.notified().await;
                continue;
            };
            self.queue_status(|queue| queue.current = Some(id));
            let result = self.run_ehentai(id, None).await;
            let throttled = matches!(
                &result,
                Err(Error::Metadata(shuku_metadata::Error::Throttled))
            );
            let paused = result.as_ref().is_err_and(stops_queue);
            let message = result.as_ref().err().map(ToString::to_string);
            match &message {
                Some(error) if paused => tracing::error!(id, %error, "E-Hentai 队列暂停"),
                Some(error) => tracing::warn!(id, %error, "E-Hentai 查找失败"),
                None => {}
            }
            self.queue_status(|queue| {
                queue.current = None;
                match &result {
                    Ok(_) => queue.done += 1,
                    // 下一次查找之前已经要等一阵，这本留在队列里重查，不算失败。
                    Err(_) if throttled => {}
                    Err(_) if paused => queue.paused = message.clone(),
                    Err(_) => {
                        queue.failed += 1;
                        queue.last_error = message.clone();
                    }
                }
            });
            // 停下或被限速时那本书留在队列里，之后重新查它。
            if !paused
                && !throttled
                && let Err(error) = self
                    .blocking(move |app| app.write(|writer| Ok(writer.dequeue_metadata(id)?)))
                    .await
            {
                tracing::error!(id, %error, "从元数据队列去掉归档失败");
                tokio::time::sleep(RETRY_DELAY).await;
            }
        }
    }

    /// 队列里的下一本。自动运行关着或者队列暂停时返回 `None`，等设置变了再叫醒。
    async fn next_queued(&self) -> Result<Option<i64>> {
        let enabled = self.metadata_settings().ehentai.auto;
        if !enabled || self.queue_status(|queue| queue.paused.is_some()) {
            return Ok(None);
        }
        self.blocking(|app| Ok(app.reader()?.next_queued_metadata()?))
            .await
    }

    fn queue_status<T>(&self, work: impl FnOnce(&mut QueueStatus) -> T) -> T {
        work(
            &mut self
                .0
                .metadata
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// 在 E-Hentai 上查一本书并写入，返回写入的内容。`gallery_url` 是用户指定的画廊。
    async fn run_ehentai(&self, id: i64, gallery_url: Option<String>) -> Result<Metadata> {
        let settings = self.metadata_settings();
        let use_cover = settings.ehentai.options.use_thumbnail && gallery_url.is_none();
        let (archive, cover_hash) = self
            .blocking(move |app| {
                let (archive, pages) = app
                    .reader()?
                    .archive_with_pages(id)?
                    .ok_or_else(Error::not_found)?;
                let known = archive
                    .metadata
                    .source_url
                    .as_deref()
                    .and_then(ehentai::gallery_id)
                    .is_some();
                // 封面读不了（文件在扫描后变了或不在了）时只是少一种搜索方式，接着按标题查。
                let cover_hash = match pages.first() {
                    Some(page) if use_cover && !known => app
                        .cover_hash(&archive, page)
                        .inspect_err(|error| {
                            tracing::warn!(id, %error, "读不了封面，不用封面搜索");
                        })
                        .ok(),
                    _ => None,
                };
                Ok((archive, cover_hash))
            })
            .await?;
        let translations = if settings.translation.enabled {
            Some(self.blocking(|app| app.translations()).await?)
        } else {
            None
        };
        let client = ehentai::Client::new(&settings.ehentai.cookies)?;
        let found = {
            let mut next = self.0.metadata.next_lookup.lock().await;
            if let Some(at) = *next {
                tokio::time::sleep_until(at).await;
            }
            let result = ehentai::lookup(
                &client,
                &settings.ehentai.options,
                &ehentai::Lookup {
                    title: &archive.metadata.title,
                    tags: &archive.tags,
                    source_url: archive.metadata.source_url.as_deref(),
                    gallery_url: gallery_url.as_deref(),
                    cover_hash: cover_hash.as_deref(),
                },
                translations.as_deref(),
            )
            .await;
            let slow_down = match &result {
                Ok(found) => found.slow_down,
                Err(error) => matches!(error, shuku_metadata::Error::Throttled),
            };
            *next = Some(Instant::now() + if slow_down { EH_SLOW_DOWN } else { EH_COOLDOWN });
            result?
        };
        let replace_title = settings.replace_title;
        self.blocking(move |app| app.apply_found(id, found.metadata, replace_title))
            .await
    }

    /// 在一个事务里写入找到的元数据，返回写入的内容。来源地址按事务里读到的现状决定：
    /// 联网期间别的请求可能已经记下了来源，那时不再覆盖它。阻塞调用。
    fn apply_found(&self, id: i64, metadata: Metadata, replace_title: bool) -> Result<Metadata> {
        let applied = self.write(|writer| {
            let archive = writer.archive(id)?.ok_or_else(Error::not_found)?;
            let edit = edit_from(
                metadata,
                replace_title,
                archive.metadata.source_url.is_some(),
            );
            edit.validate()?;
            edit.apply(writer, id)?;
            Ok(applied(&edit))
        })?;
        self.listing_changed();
        Ok(applied)
    }

    /// 封面原文件的 SHA-1，E-Hentai 的以图搜图用它。文件在扫描后变了时不算，
    /// 按旧的偏移读出来的不是这本书的封面。
    fn cover_hash(
        &self,
        archive: &shuku_store::Archive,
        page: &shuku_store::Page,
    ) -> Result<String> {
        let path = self.0.config.content_dir.join(&archive.metadata.path);
        // 和扫描、读页用同一个身份：散图文件夹的大小和修改时间是按里面的图算的。
        let identity = shuku_archive::identity(&path)?;
        if identity.size != archive.metadata.file_size
            || identity.modified != archive.metadata.mtime
        {
            return Err(Error::File("文件在扫描后变了，重新扫描后再查".into()));
        }
        let bytes = shuku_archive::read_page(&path, &archive_page(page))?;
        Ok(ehentai::image_hash(&bytes))
    }

    /// 翻译表，文件变了时重新载入。阻塞调用。
    fn translations(&self) -> Result<Arc<Translations>> {
        let path = self.translation_path();
        // 数据库读不了时后面的书也一样，算作配置错误，后台队列会停下。
        let unavailable = |error: &dyn std::fmt::Display| {
            Error::Config(format!("翻译数据库 {}：{error}", path.display()))
        };
        let metadata = std::fs::metadata(&path).map_err(|error| unavailable(&error))?;
        if !metadata.is_file() {
            return Err(unavailable(&"不是普通文件"));
        }
        let modified = metadata.modified()?;
        let mut cached = self
            .0
            .metadata
            .translations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = &*cached
            && cached.path == path
            && cached.modified == modified
        {
            return Ok(cached.translations.clone());
        }
        // 按上限读，不信任读之前看到的大小：文件可能正在被替换。
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| file.take(TRANSLATION_LIMIT + 1).read_to_end(&mut bytes))
            .map_err(|error| unavailable(&error))?;
        if bytes.len() as u64 > TRANSLATION_LIMIT {
            return Err(unavailable(&"文件太大，不像是 db.text.json"));
        }
        let translations =
            Arc::new(Translations::from_json(&bytes).map_err(|error| unavailable(&error))?);
        tracing::info!(path = %path.display(), entries = translations.len(), "载入标签翻译数据库");
        *cached = Some(CachedTranslations {
            path,
            modified,
            translations: translations.clone(),
        });
        Ok(translations)
    }

    fn translation_path(&self) -> PathBuf {
        self.0.config.data_dir.join(TRANSLATION_FILE)
    }

    /// 在本地跑文件名或 ComicInfo，阻塞调用。
    fn run_local(&self, id: i64, provider: Provider) -> Result<Metadata> {
        let settings = self.metadata_settings();
        let archive = crate::api::require_archive(&*self.reader()?, id)?;
        let path = self.0.config.content_dir.join(&archive.metadata.path);
        let metadata = match provider {
            Provider::Filename => FilenameParser::new(&settings.filename.options)?
                .parse(&book_name(&path))
                .ok_or_else(|| nothing_found("文件名和正则匹配不上"))?,
            Provider::Comicinfo => {
                read_comicinfo(&path)?.ok_or_else(|| nothing_found("这本书里没有 ComicInfo.xml"))?
            }
            Provider::Ehentai => return Err(Error::bad_request("E-Hentai 要联网，见 run_ehentai")),
        };
        self.apply_found(id, metadata, settings.replace_title)
    }

    /// 合并设置的补丁、校验后保存，阻塞调用。
    fn save_metadata_settings(&self, patch: &Value) -> Result<Arc<Settings>> {
        // 读出、合并、提交和换上新设置都在这把锁里，两个同时到的补丁不会互相覆盖，
        // 内存里的设置也不会和库里的不一样。
        let mut cached = self
            .0
            .metadata
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current =
            serde_json::to_value(&**cached).map_err(|error| Error::Write(error.to_string()))?;
        check_keys(patch, &current, "")?;
        let mut merged = current;
        merge(&mut merged, patch);
        let settings: Settings = serde_json::from_value(merged).map_err(Error::bad_request)?;
        FilenameParser::new(&settings.filename.options).map_err(Error::bad_request)?;
        let settings = Arc::new(settings);
        self.write(|writer| Ok(writer.set_setting(SETTING_KEY, &*settings)?))?;
        *cached = settings.clone();
        drop(cached);
        // 改了设置（例如换了 cookie）就从暂停的地方接着查。
        self.queue_status(|queue| queue.paused = None);
        self.0.metadata.wake.notify_one();
        Ok(settings)
    }
}

/// 这些错误说明后面的书也会一样失败，队列停下等用户处理，不把整个队列都标成失败。
fn stops_queue(error: &Error) -> bool {
    use shuku_metadata::Error as Metadata;
    matches!(
        error,
        Error::Metadata(Metadata::Banned | Metadata::EmptyPage | Metadata::Http(_))
            | Error::Config(_)
            | Error::Lock
    ) || matches!(error, Error::Store(error) if !matches!(error, shuku_store::Error::ArchiveNotFound(_)))
}

fn nothing_found(message: &str) -> Error {
    Error::Http {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        message: message.into(),
    }
}

/// 归档里的 ComicInfo.xml；散图文件夹读文件夹里直接放着的那个。
fn read_comicinfo(path: &FilePath) -> Result<Option<Metadata>> {
    let bytes = if path.is_dir() {
        read_folder_comicinfo(path)?
    } else {
        shuku_archive::read_named_entry(path, comicinfo::FILE_NAME, COMICINFO_LIMIT)?
    };
    match bytes {
        Some(bytes) => Ok(Some(comicinfo::parse(&bytes)?)),
        None => Ok(None),
    }
}

fn read_folder_comicinfo(folder: &FilePath) -> Result<Option<Vec<u8>>> {
    let wanted = comicinfo::FILE_NAME.to_lowercase();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().to_lowercase() != wanted
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(entry.path())?
            .take(COMICINFO_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > COMICINFO_LIMIT {
            return Err(Error::File(format!("{} 太大", entry.path().display())));
        }
        return Ok(Some(bytes));
    }
    Ok(None)
}

/// 文件名解析用的名字：去掉扩展名的文件名；文件夹名里的点不是扩展名，用整个文件夹名。
fn book_name(path: &FilePath) -> String {
    let name = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };
    name.map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// 找到的元数据变成写入：标签只加不删，标题按设置决定，已有来源地址时不改。
fn edit_from(metadata: Metadata, replace_title: bool, has_source: bool) -> ArchiveEdit {
    ArchiveEdit {
        title: metadata.title.filter(|_| replace_title),
        summary: None,
        source_url: metadata.source_url.filter(|_| !has_source).map(Some),
        tags: TagEdit::Merge {
            add: metadata.tags,
            remove: Vec::new(),
        },
    }
}

/// 写入的内容，回给网页端显示。
fn applied(edit: &ArchiveEdit) -> Metadata {
    Metadata {
        title: edit.title.clone(),
        tags: match &edit.tags {
            TagEdit::Merge { add, .. } => add.clone(),
            TagEdit::Keep | TagEdit::Replace(_) => Vec::new(),
        },
        source_url: edit.source_url.clone().flatten(),
    }
}

fn archive_page(page: &shuku_store::Page) -> shuku_archive::Page {
    shuku_archive::Page {
        entry_name: page.entry_name.clone(),
        data_offset: page.data_offset,
        compressed_size: page.compressed_size,
        size: page.size,
        method: match page.method {
            shuku_store::CompressionMethod::Stored => shuku_archive::CompressionMethod::Stored,
            shuku_store::CompressionMethod::Deflate => shuku_archive::CompressionMethod::Deflate,
        },
        width: page.width,
        height: page.height,
    }
}

/// 补丁里的每个键都要是设置里已有的，拼错的键不会被悄悄忽略。
fn check_keys(patch: &Value, current: &Value, prefix: &str) -> Result<()> {
    let Some(patch) = patch.as_object() else {
        return if prefix.is_empty() {
            Err(Error::bad_request("设置必须是 JSON 对象"))
        } else {
            Ok(())
        };
    };
    for (key, value) in patch {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let Some(existing) = current.get(key) else {
            return Err(Error::bad_request(format!("没有这项设置：{path}")));
        };
        if existing.is_object() {
            if !value.is_object() {
                return Err(Error::bad_request(format!("{path} 必须是 JSON 对象")));
            }
            check_keys(value, existing, &path)?;
        }
    }
    Ok(())
}

/// 把补丁里的对象逐层合并进去，其他值直接替换。
fn merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                merge(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

/// 返回给控制台的设置：cookie 只说有没有设置。
fn settings_json(settings: &Settings) -> Result<Value> {
    let mut value =
        serde_json::to_value(settings).map_err(|error| Error::Write(error.to_string()))?;
    let cookies = &settings.ehentai.cookies;
    if let Some(ehentai) = value.get_mut("ehentai").and_then(Value::as_object_mut) {
        ehentai.remove("cookies");
        ehentai.insert(
            "cookies_set".into(),
            json!({
                "ipb_member_id": !cookies.ipb_member_id.is_empty(),
                "ipb_pass_hash": !cookies.ipb_pass_hash.is_empty(),
                "star": !cookies.star.is_empty(),
                "igneous": !cookies.igneous.is_empty(),
            }),
        );
    }
    Ok(value)
}

fn metadata_json(metadata: &Metadata) -> Value {
    let tags: Vec<_> = metadata
        .tags
        .iter()
        .map(|tag| json!({"namespace": tag.namespace, "value": tag.value}))
        .collect();
    json!({"title": metadata.title, "tags": tags, "source_url": metadata.source_url})
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Filename,
    Comicinfo,
    Ehentai,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunBody {
    provider: Provider,
    /// 只对 E-Hentai 有效：直接用这个画廊，不搜索。
    url: Option<String>,
}

/// 对一本书运行一个元数据功能并写入，返回写入的标题、标签和来源。
async fn run(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
    body: std::result::Result<Json<RunBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    let Json(body) = body.map_err(Error::bad_request)?;
    let url = body
        .url
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty());
    let metadata = match (body.provider, url) {
        (Provider::Ehentai, url) => app.run_ehentai(id, url).await?,
        (_, Some(_)) => return Err(Error::bad_request("只有 E-Hentai 能指定画廊地址")),
        (provider, None) => app.blocking(move |app| app.run_local(id, provider)).await?,
    };
    Ok(Json(metadata_json(&metadata)))
}

async fn get_settings(State(app): State<App>) -> Result<Json<Value>> {
    Ok(Json(settings_json(&app.metadata_settings())?))
}

async fn put_settings(
    State(app): State<App>,
    body: std::result::Result<Json<Value>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(patch) = body.map_err(Error::bad_request)?;
    let settings = app
        .blocking(move |app| app.save_metadata_settings(&patch))
        .await?;
    Ok(Json(settings_json(&settings)?))
}

/// 后台队列和翻译数据库的状态。
async fn status(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let length = app.reader()?.metadata_queue_len()?;
        let queue = app.queue_status(|queue| queue.clone());
        let settings = app.metadata_settings();
        let path = app.translation_path();
        let translation = if settings.translation.enabled {
            match app.translations() {
                Ok(translations) => json!({"entries": translations.len(), "error": null}),
                Err(error) => json!({"entries": null, "error": error.to_string()}),
            }
        } else {
            json!({"entries": null, "error": null})
        };
        Ok(Json(json!({
            "queue": {
                "length": length, "enabled": settings.ehentai.auto, "current": queue.current,
                "done": queue.done, "failed": queue.failed, "last_error": queue.last_error,
                "paused": queue.paused,
            },
            "translation": {
                "enabled": settings.translation.enabled, "path": path,
                "entries": translation["entries"], "error": translation["error"],
            },
            "cookies": settings.ehentai.cookies.is_set(),
        })))
    })
    .await
}

async fn resume(State(app): State<App>) -> StatusCode {
    app.queue_status(|queue| queue.paused = None);
    app.0.metadata.wake.notify_one();
    StatusCode::NO_CONTENT
}

async fn clear_queue(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let removed = app.write(|writer| Ok(writer.clear_metadata_queue()?))?;
        Ok(Json(json!({ "removed": removed })))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_cover_hash_works_for_folder_books_and_notices_changes() {
        let root = tempfile::tempdir().unwrap();
        let config = crate::Config {
            content_dir: root.path().join("content"),
            data_dir: root.path().join("data"),
            web_dir: root.path().join("web"),
            listen: "127.0.0.1:0".parse().unwrap(),
        };
        let folder = config.content_dir.join("Vol.1");
        std::fs::create_dir_all(&folder).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 6))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        for name in ["1.png", "2.png"] {
            std::fs::write(folder.join(name), png.get_ref()).unwrap();
        }
        let app = App::open(config).await.unwrap();
        app.scan().await.unwrap();
        let id = app
            .reader()
            .unwrap()
            .find_by_path("Vol.1")
            .unwrap()
            .unwrap();
        let cover = || {
            let (archive, pages) = app
                .reader()
                .unwrap()
                .archive_with_pages(id)
                .unwrap()
                .unwrap();
            app.cover_hash(&archive, &pages[0])
        };
        assert_eq!(cover().unwrap(), ehentai::image_hash(png.get_ref()));
        // 扫描后加了一页，按旧记录读出来的不一定是这本书现在的封面。
        std::fs::write(folder.join("3.png"), png.get_ref()).unwrap();
        assert!(cover().is_err());
    }

    #[test]
    fn unknown_setting_keys_are_rejected_and_objects_merge() {
        let current = serde_json::to_value(Settings::default()).unwrap();
        assert!(check_keys(&json!({"filename": {"auto": true}}), &current, "").is_ok());
        assert!(check_keys(&json!({"filename": {"atuo": true}}), &current, "").is_err());
        assert!(check_keys(&json!({"filename": true}), &current, "").is_err());
        assert!(check_keys(&json!([]), &current, "").is_err());

        let mut merged = current;
        merge(
            &mut merged,
            &json!({"ehentai": {"exhentai": true, "cookies": {"igneous": "x"}}}),
        );
        let settings: Settings = serde_json::from_value(merged).unwrap();
        assert!(settings.ehentai.options.exhentai);
        assert_eq!(settings.ehentai.cookies.igneous, "x");
        // 没提到的字段不变。
        assert!(settings.ehentai.options.use_thumbnail);
        assert!(settings.replace_title);
    }

    #[test]
    fn cookies_are_reported_but_not_returned() {
        let mut settings = Settings::default();
        settings.ehentai.cookies.ipb_pass_hash = "secret".into();
        let value = settings_json(&settings).unwrap();
        assert!(!value.to_string().contains("secret"));
        assert_eq!(value["ehentai"]["cookies_set"]["ipb_pass_hash"], true);
        assert_eq!(value["ehentai"]["cookies_set"]["igneous"], false);
    }

    #[test]
    fn found_metadata_only_adds_tags_and_keeps_an_existing_source() {
        let metadata = Metadata {
            title: Some("New".into()),
            tags: vec![shuku_store::Tag {
                namespace: "artist".into(),
                value: "a".into(),
            }],
            source_url: Some("https://e-hentai.org/g/1/a/".into()),
        };
        let edit = edit_from(metadata.clone(), false, true);
        assert_eq!(edit.title, None);
        assert_eq!(edit.source_url, None);
        assert!(
            matches!(&edit.tags, TagEdit::Merge { add, remove } if add.len() == 1 && remove.is_empty())
        );
        let edit = edit_from(metadata, true, false);
        assert_eq!(edit.title.as_deref(), Some("New"));
        assert_eq!(
            edit.source_url,
            Some(Some("https://e-hentai.org/g/1/a/".into()))
        );
    }
}
