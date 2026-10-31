use crate::{App, Error, Result};
use rayon::prelude::*;
use shuku_store::{ArchiveMetadata, CompressionMethod, Page, ScanEntry};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::Path,
    time::UNIX_EPOCH,
};

#[derive(Debug, Default)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub failed: usize,
}

struct Prepared {
    path: String,
    file_size: u64,
    mtime: i64,
    quick_hash: String,
    pages: Vec<Page>,
}

impl App {
    pub async fn scan(&self) -> Result<ScanReport> {
        self.blocking(|app| app.scan_blocking()).await
    }

    fn scan_blocking(&self) -> Result<ScanReport> {
        let _scan = self.0.scan_lock.lock().map_err(|_| Error::Lock)?;
        let content = &self.0.config.content_dir;
        let entries = self.reader()?.scan_entries()?;
        // 不跟随符号链接，避免循环和越出内容目录。
        let mut files = Vec::new();
        let mut unreadable = Vec::new();
        let mut report = ScanReport::default();
        for entry in walkdir::WalkDir::new(content) {
            match entry {
                Ok(entry) if entry.file_type().is_file() => files.push(entry.into_path()),
                Ok(_) => {}
                // 内容目录里可能有别的程序建的、只有它自己能读的子目录，跳过它们继续扫描。
                // 内容目录本身读不了说明配置有误，整次扫描失败。目录读到一半出错时
                // walkdir 不给路径，无法确定受影响的范围，同样整次失败。
                Err(error) => match error.path() {
                    Some(path) if error.depth() > 0 => {
                        tracing::warn!(%error, "无法读取内容目录中的条目");
                        unreadable.push(path.to_path_buf());
                        report.failed += 1;
                    }
                    _ => return Err(error.into()),
                },
            }
        }
        let mut missing: HashMap<String, VecDeque<i64>> = HashMap::new();
        for entry in &entries {
            let path = content.join(&entry.path);
            // 读不了的目录里的记录无法判断还在不在，不当作失踪，免得被同内容的新文件顶替。
            if unreadable.iter().any(|dir| path.starts_with(dir)) {
                continue;
            }
            match path.try_exists() {
                Ok(true) => {}
                Ok(false) => missing
                    .entry(entry.quick_hash.clone())
                    .or_default()
                    .push_back(entry.id),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "无法确认已入库的文件是否存在");
                    report.failed += 1;
                }
            }
        }
        let existing: HashMap<_, _> = entries
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect();
        files.sort();
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
                    prepare(path, relative, existing.get(relative))
                })
                .collect();
            let mut saves = Vec::new();
            for (path, prepared) in batch.iter().zip(prepared) {
                match prepared {
                    Ok(Some(prepared)) => saves.push(prepared),
                    Ok(None) => report.unchanged += 1,
                    Err(Error::Archive(shuku_archive::Error::UnsupportedFormat)) => {
                        tracing::info!(path = %path.display(), "跳过不支持的格式");
                        report.skipped += 1;
                    }
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "扫描文件失败");
                        report.failed += 1;
                    }
                }
            }
            if saves.is_empty() {
                continue;
            }
            // 一批一个事务。已有记录的元数据在同一个事务里读出，迁移工具同时改了标题也不会被覆盖。
            self.write(|writer| {
                for prepared in saves {
                    let existing_id = existing.get(&prepared.path).map(|entry| entry.id);
                    let moved_id = if existing_id.is_none() {
                        missing
                            .get_mut(&prepared.quick_hash)
                            .and_then(VecDeque::pop_front)
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
                    metadata.path = prepared.path;
                    metadata.file_size = prepared.file_size;
                    metadata.mtime = prepared.mtime;
                    metadata.quick_hash = prepared.quick_hash;
                    // 完整指纹要读整个文件，由后台任务计算。内容变了，旧值也随之作废。
                    metadata.content_hash = None;
                    writer.save_archive(id, &metadata, &prepared.pages)?;
                    if moved_id.is_some() {
                        report.moved += 1;
                    } else if existing_id.is_some() {
                        report.updated += 1;
                    } else {
                        report.added += 1;
                    }
                }
                Ok(())
            })?;
            self.listing_changed();
        }
        // 从只读连接构建，期间进度上报照常写入。
        self.refresh_blocking()?;
        tracing::info!(?report, "扫描完成");
        Ok(report)
    }
}

fn new_metadata(prepared: &Prepared) -> Result<ArchiveMetadata> {
    Ok(ArchiveMetadata {
        path: prepared.path.clone(),
        title: Path::new(&prepared.path)
            .file_stem()
            .ok_or_else(|| Error::File(prepared.path.clone()))?
            .to_string_lossy()
            .into_owned(),
        summary: String::new(),
        file_size: prepared.file_size,
        mtime: prepared.mtime,
        quick_hash: prepared.quick_hash.clone(),
        content_hash: None,
        date_added: unix_seconds(std::time::SystemTime::now())?,
        source_url: None,
        is_new: true,
        lrr_id: None,
    })
}

fn prepare(path: &Path, relative: &str, existing: Option<&ScanEntry>) -> Result<Option<Prepared>> {
    let before = fs::metadata(path)?;
    let mtime = unix_seconds(before.modified()?)?;
    if existing.is_some_and(|entry| entry.file_size == before.len() && entry.mtime == mtime) {
        return Ok(None);
    }
    let quick_hash = shuku_archive::quick_hash(path)?;
    let pages = shuku_archive::list_pages(path)?
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
    // 下载工具可能正往内容目录里写文件。读的过程中文件变了，就不记录这次的结果，下次扫描再处理。
    let after = fs::metadata(path)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(Error::File(format!(
            "扫描期间文件发生变化：{}",
            path.display()
        )));
    }
    Ok(Some(Prepared {
        path: relative.into(),
        file_size: before.len(),
        mtime,
        quick_hash,
        pages,
    }))
}

pub(crate) fn unix_seconds(time: std::time::SystemTime) -> Result<i64> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::File(error.to_string()))?;
    i64::try_from(duration.as_secs()).map_err(|error| Error::File(error.to_string()))
}
