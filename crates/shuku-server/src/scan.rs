use crate::{App, Error, Result};
use rayon::prelude::*;
use shuku_store::{ArchiveMetadata, CompressionMethod, Page, ScanEntry};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    time::UNIX_EPOCH,
};

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub unchanged: usize,
    /// 其他文件：不是 zip、cbz，也不是图片文件夹里的图片。只计数，不算问题。
    pub skipped: usize,
    pub failed: usize,
}

struct Prepared {
    path: String,
    /// 散放图片的文件夹，标题取文件夹名而不是去掉扩展名的文件名。
    folder: bool,
    file_size: u64,
    /// 见 [`shuku_archive::Identity::modified`]，文件夹的不是一个时刻。
    mtime: i64,
    /// 最新的修改时间，Unix 秒，新书的加入时间用它。
    newest: i64,
    quick_hash: String,
    pages: Vec<Page>,
    /// 新文件里的 ComicInfo.xml，打开了它的自动运行时才读。
    comicinfo: Option<shuku_metadata::Metadata>,
}

impl App {
    pub async fn scan(&self) -> Result<ScanReport> {
        self.blocking(|app| app.scan_blocking()).await
    }

    fn scan_blocking(&self) -> Result<ScanReport> {
        let Ok(_scan) = self.0.scan_lock.lock() else {
            self.tasks(|tasks| tasks.scan.fail(&Error::Lock));
            return Err(Error::Lock);
        };
        // 结束状态在持有扫描锁时写入，下一次扫描拿到锁之前，这次的结果不会盖掉它的状态。
        let _interrupted = crate::admin::FailOnUnwind::scan(self);
        self.tasks(|tasks| tasks.scan.start());
        let result = self.scan_locked();
        self.tasks(|tasks| tasks.scan.finish(&result));
        result
    }

    fn scan_locked(&self) -> Result<ScanReport> {
        let content = &self.0.config.content_dir;
        let entries = self.reader()?.scan_entries()?;
        let plugins = self.scan_plugins()?;
        // 不跟随符号链接，避免循环和越出内容目录。
        let mut files = Vec::new();
        // 直接包含图片文件的子目录，每个当成一本书。
        let mut folders = std::collections::BTreeSet::new();
        let mut unreadable = Vec::new();
        let mut report = ScanReport::default();
        let settings = self.settings().scan;
        let walk = walkdir::WalkDir::new(content)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !skipped_entry(&settings, entry));
        for entry in walk {
            match entry {
                // 只扫认得的格式，其他文件只计数，不列进问题页。
                Ok(entry) if entry.file_type().is_file() => match classify(&entry) {
                    Kind::Archive => files.push(entry.into_path()),
                    // 内容目录本身直接放着的图片不成书，算其他文件。
                    Kind::Image if entry.depth() > 1 => {
                        if let Some(parent) = entry.path().parent() {
                            folders.insert(parent.to_path_buf());
                        }
                    }
                    Kind::Image | Kind::Other => report.skipped += 1,
                },
                Ok(_) => {}
                // 内容目录里可能有别的程序建的、只有它自己能读的子目录，跳过它们继续扫描。
                // 内容目录本身读不了说明配置有误，整次扫描失败。目录读到一半出错时
                // walkdir 不给路径，无法确定受影响的范围，同样整次失败。
                Err(error) => match error.path() {
                    Some(path) if error.depth() > 0 => {
                        tracing::warn!(%error, "无法读取内容目录中的条目");
                        self.tasks(|tasks| {
                            tasks
                                .scan
                                .issues
                                .unreadable
                                .push(issue(content, path, &error))
                        });
                        unreadable.push(path.to_path_buf());
                        report.failed += 1;
                    }
                    _ => return Err(error.into()),
                },
            }
        }
        let mut missing: HashMap<String, VecDeque<(i64, String)>> = HashMap::new();
        let reader = self.reader()?;
        for entry in &entries {
            let path = content.join(&entry.path);
            // 读不了的目录里的记录无法判断还在不在，不当作失踪，免得被同内容的新文件顶替。
            if unreadable.iter().any(|dir| path.starts_with(dir)) {
                continue;
            }
            // 按设置跳过的路径（隐藏条目、额外跳过的名字）里的记录也无法判断，同样不动。
            if record_excluded(&settings, &entry.path, || {
                // 路径还在就看它现在是不是文件；已经不在了就看页面清单：ZIP 的第一页前面总有
                // 本地文件头，偏移不会是 0，文件夹书的页面偏移都是 0。
                if path.exists() {
                    Ok(path.is_file())
                } else {
                    let pages = reader.pages(entry.id)?;
                    Ok(pages.first().is_some_and(|page| page.data_offset > 0))
                }
            })? {
                continue;
            }
            // 文件夹里的图片都删了、只剩空目录时也算失踪。
            match path.try_exists() {
                Ok(true) if !path.is_dir() || folders.contains(&path) => {}
                Ok(_) => missing
                    .entry(entry.quick_hash.clone())
                    .or_default()
                    .push_back((entry.id, entry.path.clone())),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "无法确认已入库的文件是否存在");
                    self.tasks(|tasks| {
                        tasks.scan.issues.failed.push(issue(content, &path, &error))
                    });
                    report.failed += 1;
                }
            }
        }
        drop(reader);
        let existing: HashMap<_, _> = entries
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect();
        // 这次扫描走到的条目和是不是文件夹，补认卷只看这些：隐藏、额外跳过和读不了的目录里的
        // 记录保持原样。
        let walked: HashMap<String, bool> = files
            .iter()
            .map(|path| (relative(content, path), false))
            .chain(folders.iter().map(|path| (relative(content, path), true)))
            .collect();
        files.extend(folders);
        files.sort();
        self.tasks(|tasks| tasks.scan.total = files.len());
        // 每批限制待提交页面清单的数量；图片解析和指纹计算并行，SQLite 提交串行。
        for batch in files.chunks(64) {
            let prepared: Vec<_> = batch
                .par_iter()
                .map(|path| {
                    let relative = path
                        .strip_prefix(content)
                        .map_err(|error| Error::File(error.to_string()))?;
                    let relative = relative.to_str().ok_or_else(|| {
                        Error::File(format!("路径不是 UTF-8：{}", path.display()))
                    })?;
                    prepare(path, relative, existing.get(relative), &plugins)
                })
                .collect();
            let mut saves = Vec::new();
            for (path, prepared) in batch.iter().zip(prepared) {
                match prepared {
                    Ok(Some(prepared)) => saves.push(prepared),
                    Ok(None) => report.unchanged += 1,
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "扫描文件失败");
                        self.tasks(|tasks| {
                            tasks.scan.issues.failed.push(crate::admin::FileIssue {
                                path: relative(content, path),
                                error: error.to_string(),
                            })
                        });
                        report.failed += 1;
                    }
                }
            }
            if saves.is_empty() {
                self.tasks(|tasks| tasks.scan.progress(batch.len(), &report));
                continue;
            }
            // 一批一个事务。已有记录的元数据在同一个事务里读出，迁移工具同时改了标题也不会被覆盖。
            let queued = self.write(|writer| {
                let mut queued = false;
                for prepared in saves {
                    let existing_id = existing.get(&prepared.path).map(|entry| entry.id);
                    let moved_id = if existing_id.is_none() {
                        missing
                            .get_mut(&prepared.quick_hash)
                            .and_then(VecDeque::pop_front)
                            .map(|(id, _)| id)
                    } else {
                        None
                    };
                    let id = existing_id.or(moved_id);
                    let mut metadata = match id {
                        Some(id) => {
                            writer
                                .archive(id)?
                                .ok_or(shuku_store::Error::ArchiveNotFound(id))?
                                .metadata
                        }
                        None => new_metadata(&prepared)?,
                    };
                    // 新书按路径认卷，名字只有卷号的换成带系列名的标题。文件名解析仍用原来的名字。
                    let file_title = file_title(&prepared)?;
                    let detected = crate::series::detect(
                        &prepared.path,
                        &file_title,
                        settings.series_from_top_dir,
                    );
                    let detected = match detected {
                        Some(volume) if !volume.joinable(writer)? => None,
                        detected => detected,
                    };
                    // ComicInfo 写明了系列和序号时以它为准，但同一文件夹已经按路径归成了系列
                    // （例如在 ComicInfo 归组之前入库的卷）时跟着原系列，免得新卷单独成组。
                    let joins_path_series = match &detected {
                        Some(volume) if id.is_none() => writer.series_key_exists(&volume.key)?,
                        _ => false,
                    };
                    let comicinfo = prepared
                        .comicinfo
                        .as_ref()
                        .and_then(|metadata| metadata.volume.clone())
                        .filter(|_| id.is_none() && !joins_path_series)
                        .map(crate::series::from_comicinfo);
                    // 按 ComicInfo 归组时卷号可能和名字里的不一样，标题不按路径改。
                    if id.is_none()
                        && comicinfo.is_none()
                        && let Some(title) =
                            detected.as_ref().and_then(|volume| volume.title.clone())
                    {
                        metadata.title = title;
                    }
                    metadata.path = prepared.path;
                    metadata.file_size = prepared.file_size;
                    metadata.mtime = prepared.mtime;
                    metadata.quick_hash = prepared.quick_hash;
                    // 完整指纹要读整个文件，由后台任务计算。内容变了，旧值也随之作废。
                    metadata.content_hash = None;
                    let path = metadata.path.clone();
                    let saved = writer.save_archive(id, &metadata, &prepared.pages)?;
                    // 迁移工具先于扫描运行时，LRR 的元数据按路径暂存着，在同一个事务里套用。
                    // 只有真正新加入、不是从 LRR 迁移来的书才跑元数据功能，和 LRR 只对新上传的书
                    // 自动运行插件一样；迁移来的元数据已经是 LRR 里跑过插件的结果。
                    let from_lrr =
                        existing_id.is_none() && writer.apply_lrr_pending(saved, &path)?;
                    let fresh = id.is_none() && !from_lrr;
                    // ComicInfo 写明了系列和序号时以它为准，它和别的元数据功能一样只用于真正的新书；
                    // 按路径认卷是目录结构，迁移来的书也用。
                    match id {
                        None => {
                            let volume = if fresh {
                                comicinfo.or(detected)
                            } else {
                                detected
                            };
                            if let Some(volume) = volume {
                                writer.set_series(
                                    saved,
                                    &volume.key,
                                    &volume.series,
                                    volume.volume,
                                )?;
                            }
                        }
                        // 整个系列的文件夹搬走后，归组依据跟着换到新位置，之后加进来的卷才归得进去。
                        Some(_) if moved_id.is_some() => {
                            if let Some(volume) = detected {
                                writer.move_series(saved, &volume.key)?;
                            }
                        }
                        Some(_) => {}
                    }
                    if fresh {
                        // 新书的初始标题就是去掉扩展名的文件名（文件夹是整个文件夹名），
                        // 文件名解析用它，和 LRR 用文件名一样。
                        if let Some(edit) =
                            plugins.edit_for_new(&path, &file_title, prepared.comicinfo)
                        {
                            edit.apply(writer, saved)?;
                        }
                        for provider in plugins.queued_providers() {
                            writer.queue_metadata(saved, provider)?;
                            queued = true;
                        }
                    }
                    if moved_id.is_some() {
                        report.moved += 1;
                    } else if existing_id.is_some() {
                        report.updated += 1;
                    } else {
                        report.added += 1;
                    }
                }
                Ok(queued)
            })?;
            self.listing_changed();
            // 每批提交后就叫醒队列，后面的批次失败也不会让已经入队的书一直等着。
            if queued {
                self.metadata_queued();
            }
            self.tasks(|tasks| tasks.scan.progress(batch.len(), &report));
            #[cfg(test)]
            if let Some(hook) = self
                .0
                .scan_batch_hook
                .lock()
                .map_err(|_| Error::Lock)?
                .as_mut()
            {
                hook();
            }
        }
        let assigned = self.assign_series(&walked, settings.series_from_top_dir)?;
        if assigned > 0 {
            tracing::info!(assigned, "已入库的书按路径认出了卷");
        }
        // 没被新文件认领的失踪记录，文件确实不在了。
        let gone: Vec<(i64, String)> = missing.into_values().flatten().collect();
        self.tasks(|tasks| {
            for (id, path) in gone {
                tasks
                    .scan
                    .issues
                    .missing
                    .push(crate::admin::MissingArchive { id, path });
            }
        });
        // 从只读连接构建，期间进度上报照常写入。
        self.refresh_blocking()?;
        tracing::info!(?report, "扫描完成");
        Ok(report)
    }
}

/// 按设置不扫的名字：以 `.` 开头的隐藏条目（除非打开了扫描隐藏条目），以及“额外跳过”命中的名字。
fn excluded(settings: &crate::settings::ScanSettings, name: &std::ffi::OsStr) -> bool {
    let hidden = name.as_encoded_bytes().starts_with(b".");
    (hidden && !settings.scan_hidden) || name.to_str().is_some_and(|name| settings.skips(name))
}

/// 已入库的记录是否在按设置跳过的地方，规则和 [`skipped_entry`] 一致：路径里的目录按名字，
/// 最后一段是图片扩展名的 ZIP 文件只看隐藏规则。`is_zip_file` 只在最后一段是图片扩展名、
/// 需要分清它是 ZIP 文件还是文件夹书时才调用。
fn record_excluded(
    settings: &crate::settings::ScanSettings,
    relative: &str,
    is_zip_file: impl FnOnce() -> Result<bool>,
) -> Result<bool> {
    let components: Vec<_> = Path::new(relative).components().collect();
    let Some((last, parents)) = components.split_last() else {
        return Ok(false);
    };
    if parents
        .iter()
        .any(|component| excluded(settings, component.as_os_str()))
    {
        return Ok(true);
    }
    let name = last.as_os_str();
    if name.to_str().is_some_and(shuku_archive::is_image) && is_zip_file()? {
        Ok(name.as_encoded_bytes().starts_with(b".") && !settings.scan_hidden)
    } else {
        Ok(excluded(settings, name))
    }
}

/// 遍历时跳过的条目。“额外跳过”不作用于单张图片：文件夹书的页面清单和指纹总是包含其中全部
/// 图片，只跳过一张会让两边对不上；要跳过整本，写文件夹的名字。
fn skipped_entry(settings: &crate::settings::ScanSettings, entry: &walkdir::DirEntry) -> bool {
    let image = entry.file_type().is_file()
        && entry
            .file_name()
            .to_str()
            .is_some_and(shuku_archive::is_image);
    if image {
        entry.file_name().as_encoded_bytes().starts_with(b".") && !settings.scan_hidden
    } else {
        excluded(settings, entry.file_name())
    }
}

enum Kind {
    /// zip、cbz，或者改成图片扩展名的 ZIP（按魔数认出）。
    Archive,
    /// 图片，所在的目录是一本书。
    Image,
    Other,
}

/// 扫描只认 zip、cbz 和散放的图片；7z、RAR、PDF 以后再加。以 `.` 开头的图片不是页面。
fn classify(entry: &walkdir::DirEntry) -> Kind {
    let Some(name) = entry.file_name().to_str() else {
        return Kind::Other;
    };
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("zip" | "cbz")) {
        return Kind::Archive;
    }
    if !shuku_archive::is_image(name) {
        return Kind::Other;
    }
    // 读不了魔数时当归档，交给后面的扫描报错，不悄悄算成图片。
    if shuku_archive::has_zip_magic(entry.path()).unwrap_or(true) {
        Kind::Archive
    } else if name.starts_with('.') {
        // 打开了扫描隐藏条目时才会走到这里；隐藏的图片不是文件夹的页面。
        Kind::Other
    } else {
        Kind::Image
    }
}

/// 问题列表里显示相对内容目录的路径，和数据库里一致。
fn relative(content: &Path, path: &Path) -> String {
    path.strip_prefix(content)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn issue(content: &Path, path: &Path, error: &dyn std::fmt::Display) -> crate::admin::FileIssue {
    crate::admin::FileIssue {
        path: relative(content, path),
        error: error.to_string(),
    }
}

/// 新书的初始标题：去掉扩展名的文件名，文件夹是整个文件夹名。
fn file_title(prepared: &Prepared) -> Result<String> {
    let path = Path::new(&prepared.path);
    // 文件夹名里的点不是扩展名，例如“Vol.1”。
    let title = if prepared.folder {
        path.file_name()
    } else {
        path.file_stem()
    };
    Ok(title
        .ok_or_else(|| Error::File(prepared.path.clone()))?
        .to_string_lossy()
        .into_owned())
}

fn new_metadata(prepared: &Prepared) -> Result<ArchiveMetadata> {
    Ok(ArchiveMetadata {
        path: prepared.path.clone(),
        title: file_title(prepared)?,
        summary: String::new(),
        file_size: prepared.file_size,
        mtime: prepared.mtime,
        quick_hash: prepared.quick_hash.clone(),
        content_hash: None,
        // 用文件的修改时间：第一次扫描已有的书库时，扫描时间会让所有书都显示成今天加入。
        // 修改时间在未来（时钟不准）时用现在。
        date_added: prepared
            .newest
            .min(unix_seconds(std::time::SystemTime::now())?),
        source_url: None,
        is_new: true,
        lrr_id: None,
    })
}

fn prepare(
    path: &Path,
    relative: &str,
    existing: Option<&ScanEntry>,
    plugins: &crate::metadata::ScanPlugins,
) -> Result<Option<Prepared>> {
    let folder = path.is_dir();
    let before = shuku_archive::identity(path)?;
    if existing
        .is_some_and(|entry| entry.file_size == before.size && entry.mtime == before.modified)
    {
        return Ok(None);
    }
    let (quick_hash, (pages, broken)) = if folder {
        (
            shuku_archive::folder_quick_hash(path)?,
            shuku_archive::list_folder_pages_with_broken(path)?,
        )
    } else {
        (
            shuku_archive::quick_hash(path)?,
            shuku_archive::list_pages_with_broken(path)?,
        )
    };
    for page in broken {
        tracing::warn!(path = %path.display(), entry = page.entry_name, reason = page.reason, "页面读不出尺寸，按坏页入库");
    }
    let pages = pages
        .into_iter()
        .map(|page| Page {
            entry_name: page.entry_name,
            data_offset: page.data_offset,
            compressed_size: page.compressed_size,
            size: page.size,
            method: match page.method {
                shuku_archive::CompressionMethod::Stored => CompressionMethod::Stored,
                shuku_archive::CompressionMethod::Deflate => CompressionMethod::Deflate,
            },
            width: page.width,
            height: page.height,
        })
        .collect();
    // 已入库的文件变了不重读：元数据功能只对新书自动运行。
    let comicinfo = existing
        .is_none()
        .then(|| plugins.comicinfo(path))
        .flatten();
    // 下载工具可能正往内容目录里写文件。读的过程中文件变了，就不记录这次的结果，下次扫描再处理。
    if shuku_archive::identity(path)? != before {
        return Err(Error::File(format!(
            "扫描期间文件发生变化：{}",
            path.display()
        )));
    }
    Ok(Some(Prepared {
        path: relative.into(),
        folder,
        file_size: before.size,
        mtime: before.modified,
        newest: unix_seconds(before.newest)?,
        quick_hash,
        pages,
        comicinfo,
    }))
}

pub(crate) fn unix_seconds(time: std::time::SystemTime) -> Result<i64> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::File(error.to_string()))?;
    i64::try_from(duration.as_secs()).map_err(|error| Error::File(error.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::{App, Config};
    use shuku_index::Query;
    use std::{
        io::{Cursor, Write},
        sync::mpsc,
        time::{Duration, Instant},
    };

    fn write_archive(path: &std::path::Path) {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 6))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        zip.start_file("1.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(png.get_ref()).unwrap();
        zip.finish().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_shows_committed_batches_while_the_scan_is_still_running() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            content_dir: root.path().join("content"),
            data_dir: root.path().join("data"),
            web_dir: root.path().join("web"),
            listen: "127.0.0.1:0".parse().unwrap(),
        };
        std::fs::create_dir(&config.content_dir).unwrap();
        // 每批 64 个文件，65 个文件分成两批。
        for number in 0..65 {
            write_archive(&config.content_dir.join(format!("{number:03}.cbz")));
        }
        let app = App::open(config).await.unwrap();
        let (reached, first_batch) = mpsc::channel();
        let (release, resume) = mpsc::channel::<()>();
        let mut batches = 0;
        *app.0.scan_batch_hook.lock().unwrap() = Some(Box::new(move || {
            batches += 1;
            if batches == 1 {
                reached.send(()).unwrap();
                resume.recv().unwrap();
            }
        }));
        let scanning = tokio::spawn({
            let app = app.clone();
            async move { app.scan().await }
        });
        // 扫描在第一批之前就失败时钩子不会被调用，限时等待，不让测试挂住。
        tokio::task::spawn_blocking(move || first_batch.recv_timeout(Duration::from_secs(10)))
            .await
            .unwrap()
            .expect("扫描没有提交第一批");

        // 扫描停在第一批之后，还没有走到结束时的重建；后台的延迟刷新应该已经把第一批发布出去。
        let started = Instant::now();
        while app.index().search(&Query::default()).unwrap().total != 64 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "扫描中途没有刷新"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!scanning.is_finished());

        release.send(()).unwrap();
        assert_eq!(scanning.await.unwrap().unwrap().added, 65);
        assert_eq!(app.index().search(&Query::default()).unwrap().total, 65);
    }
}
