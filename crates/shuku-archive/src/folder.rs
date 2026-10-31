//! 散放在文件夹里的图片当成一本书：文件夹直接包含的图片按自然序就是页面。
//! 页面清单的格式和 ZIP 一样，条目名是文件名，偏移为 0、按原样存储。

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{BrokenPage, CompressionMethod, Error, MAX_PAGE_SIZE, Page, Result, is_image, jpeg};

/// 文件夹直接包含的图片，按文件名的自然序。以 `.` 开头的文件和改成图片扩展名的 ZIP 不算页面，
/// 后者由扫描当作单独的归档，和扫描找书时的规则一致。
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
        if metadata.is_file() && !crate::has_zip_magic(entry.path())? {
            images.push((name, metadata));
        }
    }
    images.sort_by(|a, b| crate::page_order(&a.0, &b.0));
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

/// 判断归档有没有变过用的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    /// 文件大小；文件夹是其中图片的总大小。
    pub size: u64,
    /// 文件是修改时间（Unix 秒）。文件夹是文件夹本身的修改时间和每张图的文件名、大小、
    /// 修改时间（都到纳秒）的 64 位摘要：页面被改、改名、互换名字或增删都会让它变，
    /// 即使发生在同一秒、或者有的图带着未来的时间戳。只用来比较，不是一个时刻。
    pub modified: i64,
    /// 最新的修改时间，文件夹里也算上文件夹本身。新书的加入时间用它。
    pub newest: SystemTime,
}

fn unix_nanos(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    }
}

fn unix_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    }
}

/// 归档文件或散放图片的文件夹的身份。扫描、补算指纹和读页都用它判断内容是否变过。
pub fn identity(path: impl AsRef<Path>) -> Result<Identity> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)?;
    let own = metadata.modified()?;
    if !metadata.is_dir() {
        return Ok(Identity {
            size: metadata.len(),
            modified: unix_seconds(own),
            newest: own,
        });
    }
    let mut size = 0;
    let mut newest = own;
    let mut digest = blake3::Hasher::new();
    digest.update(&unix_nanos(own).to_le_bytes());
    for (name, metadata) in images(path)? {
        let modified = metadata.modified()?;
        size += metadata.len();
        newest = newest.max(modified);
        digest.update(&(name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update(&metadata.len().to_le_bytes());
        digest.update(&unix_nanos(modified).to_le_bytes());
    }
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.finalize().as_bytes()[..8]);
    Ok(Identity {
        size,
        modified: i64::from_le_bytes(bytes),
        newest,
    })
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

/// 由每张图片的文件名和 ZIP 同样算法的快速指纹（大小加首尾各 64 KiB）合成。内容不变的文件夹
/// 移动或改名后指纹不变，扫描靠它认出原来的记录；开头的固定前缀让它和 ZIP 的指纹不会撞上。
pub fn folder_quick_hash(dir: impl AsRef<Path>) -> Result<String> {
    let dir = dir.as_ref();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"shuku-folder-v2");
    for (name, _) in images(dir)? {
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        // 和 ZIP 的快速指纹一样带上大小和首尾各一段内容：它也是图片 URL 的版本号，
        // 同名等长的图片换了内容时 URL 要跟着变，不能让浏览器继续用缓存的旧图。
        let sample = crate::fingerprint::quick_hash_file(&mut open_image(&dir.join(&name))?)?;
        hasher.update(sample.as_bytes());
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
    // 只接受恰好一段普通的文件名，排除路径分隔符、`.` 和 `..`。Unix 文件名里的反斜杠是普通字符。
    let mut components = Path::new(name).components();
    let single = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(part)), None) if part == name.as_str()
    );
    if !single {
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
