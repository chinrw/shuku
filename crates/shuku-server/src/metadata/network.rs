//! 在线查找：E-Hentai（可选译成中文）、Bangumi 和 AniList。每个来源各自限速，
//! 手动运行和后台队列共用同一个限速。

use super::{Provider, proposal};
use crate::{App, Error, Result};
use shuku_metadata::{
    Metadata, anilist, bangumi, ehentai, series::series_query, translate::Translations,
};
use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime},
};
use tokio::{sync::Notify, time::Instant};

/// EhTagTranslation 的数据库，放在数据目录里。
const TRANSLATION_FILE: &str = "db.text.json";
/// 翻译数据库约 4 MiB，超过这个大小多半选错了文件。
const TRANSLATION_LIMIT: u64 = 64 * 1024 * 1024;

/// 一个来源两次查找之间至少隔多久，以及对方说请求太快时隔多久。
fn cooldown(provider: Provider, throttled: bool) -> Duration {
    let seconds = match (provider, throttled) {
        // 和 LRR 给 E-Hentai 插件设的冷却时间一样；LRR 被提示太快时随机等 15 到 50 秒。
        (Provider::Ehentai, false) => 4,
        (Provider::Ehentai, true) => 40,
        // Bangumi 没有公开的限额，按每秒一次。
        (Provider::Bangumi, false) => 1,
        (Provider::Bangumi, true) => 30,
        // AniList 限每分钟 30 次，超了要等一分钟。
        (Provider::Anilist, false) => 2,
        (Provider::Anilist, true) => 60,
        (Provider::Filename | Provider::Comicinfo, _) => 0,
    };
    Duration::from_secs(seconds)
}

/// 在线来源的运行状态，只在内存里。
pub(crate) struct MetadataNetwork {
    /// 按路径和修改时间缓存，文件换了下次用时重新载入。
    translations: Mutex<Option<CachedTranslations>>,
    /// 每个来源下一次查找最早的时间，持有锁的那个在查。
    next: HashMap<Provider, tokio::sync::Mutex<Option<Instant>>>,
    /// 队列里加了书、设置变了或者要求继续时叫醒各来源的后台任务。
    pub(super) wake: HashMap<Provider, Notify>,
    pub(super) status: Mutex<HashMap<Provider, QueueStatus>>,
}

struct CachedTranslations {
    path: PathBuf,
    modified: SystemTime,
    translations: Arc<Translations>,
}

/// 控制台显示的队列状态。
#[derive(Debug, Default, Clone, serde::Serialize)]
pub(crate) struct QueueStatus {
    /// 正在查的归档。
    pub(super) current: Option<i64>,
    pub(super) done: u64,
    pub(super) failed: u64,
    pub(super) last_error: Option<String>,
    /// 遇到 IP 被封、cookie 失效或者网络不通时停下，原因写在这里，那本书留在队列里。
    /// 改设置或在控制台点“继续”后重新开始。
    pub(super) paused: Option<String>,
}

impl Default for MetadataNetwork {
    fn default() -> Self {
        Self {
            translations: Mutex::new(None),
            next: Provider::NETWORK
                .into_iter()
                .map(|provider| (provider, tokio::sync::Mutex::new(None)))
                .collect(),
            wake: Provider::NETWORK
                .into_iter()
                .map(|provider| (provider, Notify::new()))
                .collect(),
            status: Mutex::new(HashMap::new()),
        }
    }
}

impl MetadataNetwork {
    /// 本地来源没有限速锁。
    fn limiter(&self, provider: Provider) -> Result<&tokio::sync::Mutex<Option<Instant>>> {
        self.next
            .get(&provider)
            .ok_or_else(|| Error::bad_request(format!("{} 不是在线来源", provider.as_str())))
    }
}

impl App {
    fn network(&self) -> &MetadataNetwork {
        &self.0.metadata.network
    }

    pub(crate) fn queue_status<T>(
        &self,
        provider: Provider,
        work: impl FnOnce(&mut QueueStatus) -> T,
    ) -> T {
        let mut status = self
            .network()
            .status
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        work(status.entry(provider).or_default())
    }

    /// 在一个在线来源上查一本书，返回要写的内容，不写入。`url` 是用户指定的画廊或条目。
    pub(super) async fn run_network(
        &self,
        provider: Provider,
        id: i64,
        url: Option<String>,
    ) -> Result<Metadata> {
        let limiter = self.network().limiter(provider)?;
        let settings = self.metadata_settings();
        let use_cover = provider == Provider::Ehentai
            && settings.ehentai.options.use_thumbnail
            && url.is_none();
        let (archive, cover_hash, series) = self
            .blocking(move |app| {
                let reader = app.reader()?;
                let (archive, pages) = reader
                    .archive_with_pages(id)?
                    .ok_or_else(Error::not_found)?;
                // 扫描按文件夹和卷号认出的系列，Bangumi 和 AniList 用它的名字搜。
                let series = reader.archive_series(id)?.map(|series| series.name);
                drop(reader);
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
                Ok((archive, cover_hash, series))
            })
            .await?;
        let translations = if provider == Provider::Ehentai && settings.translation.enabled {
            Some(self.blocking(|app| app.translations()).await?)
        } else {
            None
        };
        let source = archive.metadata.source_url.as_deref();
        let query = series_query(&archive.metadata.title, &archive.tags, series.as_deref());
        let mut next = limiter.lock().await;
        if let Some(at) = *next {
            tokio::time::sleep_until(at).await;
        }
        let result = match provider {
            Provider::Ehentai => {
                let client = ehentai::client(&settings.ehentai.cookies)?;
                ehentai::lookup(
                    &client,
                    &settings.ehentai.options,
                    &ehentai::Lookup {
                        title: &archive.metadata.title,
                        tags: &archive.tags,
                        source_url: source,
                        gallery_url: url.as_deref(),
                        cover_hash: cover_hash.as_deref(),
                    },
                    translations.as_deref(),
                )
                .await
                .map(|found| (found.metadata, found.slow_down))
            }
            Provider::Bangumi => {
                let client = bangumi::client(&settings.bangumi.token)?;
                bangumi::lookup(
                    &client,
                    &settings.bangumi.options,
                    &bangumi::Lookup {
                        query: query.as_deref(),
                        source_url: source,
                        subject_url: url.as_deref(),
                    },
                )
                .await
                .map(|metadata| (metadata, false))
            }
            Provider::Anilist => {
                let client = anilist::client()?;
                anilist::lookup(
                    &client,
                    &settings.anilist.options,
                    &anilist::Lookup {
                        query: query.as_deref(),
                        source_url: source,
                        media_url: url.as_deref(),
                    },
                )
                .await
                .map(|metadata| (metadata, false))
            }
            Provider::Filename | Provider::Comicinfo => {
                return Err(Error::bad_request("本地来源不联网"));
            }
        };
        let throttled = match &result {
            Ok((_, slow_down)) => *slow_down,
            Err(error) => matches!(error, shuku_metadata::Error::Throttled),
        };
        *next = Some(Instant::now() + cooldown(provider, throttled));
        drop(next);
        let (metadata, _) = result?;
        Ok(proposal(metadata, settings.replace_title))
    }

    /// 封面原文件的 SHA-1，E-Hentai 的以图搜图用它。文件在扫描后变了时不算，
    /// 按旧的偏移读出来的不是这本书的封面。
    pub(super) fn cover_hash(
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
    pub(super) fn translations(&self) -> Result<Arc<Translations>> {
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
            .network()
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

    pub(super) fn translation_path(&self) -> PathBuf {
        self.0.config.data_dir.join(TRANSLATION_FILE)
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
}
