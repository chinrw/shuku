//! 散放在文件夹里的图片当成一本书：文件夹直接包含的图片按自然序就是页面。
//! 页面清单的格式和 ZIP 一样，条目名是文件名，偏移为 0、按原样存储。

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::SystemTime;

use crate::{BrokenPage, CompressionMethod, Error, MAX_PAGE_SIZE, Page, Result, is_image, jpeg};

/// 文件夹直接包含的图片，按文件名的自然序。以 `.` 开头的文件不算页面。
fn images(dir: &Path) -> Result<Vec<(String, fs::Metadata)>> {
    let mut images = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') || !is_image(&name) {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            images.push((name, metadata));
        }
    }
    images.sort_by(|a, b| crate::natural_cmp(&a.0.to_lowercase(), &b.0.to_lowercase()));
    Ok(images)
}

/// 打开文件夹里的一张图片。不跟随符号链接，打开的必须是普通文件：扫描只把普通文件记成页面，
/// 之后某页被换成指向别处的链接时，不能借它读到内容目录以外的文件。
fn open_image(path: &Path) -> Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW);
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::CorruptEntry {
            entry_name: path.display().to_string(),
            reason: "文件夹里的页面不是普通文件",
        });
    }
    Ok(file)
}

/// 归档文件的大小和修改时间；文件夹是其中图片的总大小和最新的修改时间。
/// 扫描和读页都用它判断内容是否变过。
pub fn identity(path: impl AsRef<Path>) -> Result<(u64, SystemTime)> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Ok((metadata.len(), metadata.modified()?));
    }
    let mut size = 0;
    let mut newest = SystemTime::UNIX_EPOCH;
    for (_, metadata) in images(path)? {
        size += metadata.len();
        newest = newest.max(metadata.modified()?);
    }
    Ok((size, newest))
}

/// 和 [`crate::list_pages_with_broken`] 一样，坏页的宽和高记为 0。
pub fn list_folder_pages_with_broken(
    dir: impl AsRef<Path>,
) -> Result<(Vec<Page>, Vec<BrokenPage>)> {
    let dir = dir.as_ref();
    let mut pages = Vec::new();
    let mut broken = Vec::new();
    for (name, metadata) in images(dir)? {
        if metadata.len() > MAX_PAGE_SIZE {
            return Err(Error::CorruptEntry {
                entry_name: name,
                reason: "图片超过单页大小上限",
            });
        }
        let path = dir.join(&name);
        // 小缓冲避免尺寸探测预读像素；imagesize 失败时和 ZIP 一样再用容错的 JPEG 解析试一次。
        let size = imagesize::reader_size(BufReader::with_capacity(1024, open_image(&path)?))
            .ok()
            .and_then(|size| {
                Some((
                    u32::try_from(size.width).ok()?,
                    u32::try_from(size.height).ok()?,
                ))
            })
            .or_else(|| jpeg::size(BufReader::new(open_image(&path).ok()?)));
        let (width, height) = size.unwrap_or_else(|| {
            broken.push(BrokenPage {
                entry_name: name.clone(),
                reason: "读不出图片尺寸".into(),
            });
            (0, 0)
        });
        pages.push(Page {
            entry_name: name,
            data_offset: 0,
            compressed_size: metadata.len(),
            size: metadata.len(),
            method: CompressionMethod::Stored,
            width,
            height,
        });
    }
    Ok((pages, broken))
}

/// 由文件名和大小算出的快速指纹，不读图片内容。内容不变的文件夹移动或改名后指纹不变，
/// 扫描靠它认出原来的记录；和 ZIP 的快速指纹不会撞上，因为开头写了固定的前缀。
pub fn folder_quick_hash(dir: impl AsRef<Path>) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"shuku-folder-v1");
    for (name, metadata) in images(dir.as_ref())? {
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&metadata.len().to_le_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// 依次读完每张图片的完整指纹，文件名和长度也算进去，页面换了顺序或改了名指纹就不同。
pub fn folder_content_hash(dir: impl AsRef<Path>) -> Result<String> {
    let dir = dir.as_ref();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"shuku-folder-v1");
    for (name, _) in images(dir)? {
        let mut file = open_image(&dir.join(&name))?;
        let size = file.metadata()?.len();
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&size.to_le_bytes());
        hasher.update_reader((&mut file).take(size))?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// 文件名只能是扫描时记下的单层文件名，不能带路径分隔符。
pub(crate) fn read_folder_page(dir: &Path, page: &Page) -> Result<Vec<u8>> {
    let name = &page.entry_name;
    if name.is_empty() || name.contains(['/', '\\']) || name == ".." {
        return Err(Error::CorruptEntry {
            entry_name: name.clone(),
            reason: "文件夹页面的文件名无效",
        });
    }
    let file = open_image(&dir.join(name))?;
    if file.metadata()?.len() != page.size {
        return Err(Error::CorruptEntry {
            entry_name: name.clone(),
            reason: "图片大小与清单不符",
        });
    }
    let mut bytes = Vec::with_capacity(page.size as usize);
    file.take(page.size).read_to_end(&mut bytes)?;
    Ok(bytes)
}
