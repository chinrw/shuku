//! ZIP 页面清单、定位读取与内容指纹。

mod fingerprint;
mod folder;
mod header;
mod jpeg;

pub use fingerprint::{content_hash, content_hash_file, quick_hash, quick_hash_file};
pub use folder::{
    Identity, folder_content_hash, folder_quick_hash, identity, list_folder_pages_with_broken,
};

use std::cmp::Ordering;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("不支持的格式：仅支持 ZIP")]
    UnsupportedFormat,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error("不支持的压缩方式：{0}")]
    UnsupportedCompression(zip::CompressionMethod),
    #[error("损坏的 ZIP 条目 {entry_name}：{reason}")]
    CorruptEntry {
        entry_name: String,
        reason: &'static str,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// 原始大小来自中央目录，读页和尺寸探测都按它分配内存。
/// 漫画单页远小于这个值，超过的条目视为损坏，避免伪造的大小耗尽内存。
pub const MAX_PAGE_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMethod {
    Stored,
    Deflate,
}

/// 清单下标就是从 0 开始的页码；偏移以归档文件开头为基准。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub entry_name: String,
    pub data_offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub method: CompressionMethod,
    pub width: u32,
    pub height: u32,
}

/// 读不出尺寸的页。它仍在页面清单里，宽和高记为 0。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenPage {
    pub entry_name: String,
    pub reason: String,
}

/// 解析中央目录并探测图片尺寸，不解码像素，也不写入内容目录。读不出尺寸的页
/// 宽和高记为 0，见 [`list_pages_with_broken`]。
pub fn list_pages(path: impl AsRef<Path>) -> Result<Vec<Page>> {
    Ok(list_pages_with_broken(path)?.0)
}

/// 同 [`list_pages`]，另外返回读不出尺寸的页和原因。和 LRR 一样，个别页坏了
/// （全是 0 的图、0 字节的文件、改了扩展名的别的文件）整本仍然入库，坏页留在清单里，
/// 阅读时显示不出来。ZIP 结构本身的错误仍然让整本失败。
pub fn list_pages_with_broken(path: impl AsRef<Path>) -> Result<(Vec<Page>, Vec<BrokenPage>)> {
    let mut file = File::open(path)?;
    let mut magic = Vec::with_capacity(4);
    file.by_ref().take(4).read_to_end(&mut magic)?;
    // 空 zip 只有结束记录，所以也接受 PK\x05\x06 开头。
    if !matches!(magic.as_slice(), b"PK\x03\x04" | b"PK\x05\x06") {
        return Err(Error::UnsupportedFormat);
    }
    file.rewind()?;
    let file_size = file.metadata()?.len();
    let mut archive = zip::ZipArchive::new(BufReader::new(file))?;
    let mut pages = Vec::new();
    let mut broken = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        let entry_name = entry.name()?.into_owned();
        if entry.is_dir() || !is_image(&entry_name) {
            continue;
        }
        if entry.encrypted() {
            return Err(zip::result::ZipError::UnsupportedArchive("不支持加密的 ZIP 条目").into());
        }
        let method = match entry.compression() {
            zip::CompressionMethod::Stored => CompressionMethod::Stored,
            zip::CompressionMethod::Deflated => CompressionMethod::Deflate,
            method => return Err(Error::UnsupportedCompression(method)),
        };
        let mut page = Page {
            entry_name,
            data_offset: entry
                .data_start()
                .ok_or_else(|| std::io::Error::other("ZIP 条目缺少数据偏移"))?,
            compressed_size: entry.compressed_size(),
            size: entry.size(),
            method,
            width: 0,
            height: 0,
        };
        if page.data_offset > file_size || page.compressed_size > file_size - page.data_offset {
            return Err(Error::CorruptEntry {
                entry_name: page.entry_name,
                reason: "数据区间超出归档文件",
            });
        }
        if method == CompressionMethod::Stored && page.compressed_size != page.size {
            return Err(Error::CorruptEntry {
                entry_name: page.entry_name,
                reason: "stored 条目的压缩大小与原始大小不符",
            });
        }
        if page.size > MAX_PAGE_SIZE {
            return Err(Error::CorruptEntry {
                entry_name: page.entry_name,
                reason: "图片超过单页大小上限",
            });
        }
        drop(entry);
        // 小缓冲避免尺寸探测预读像素；deflate 只保留解析器访问过的前缀。
        let size = match method {
            CompressionMethod::Stored => {
                imagesize::reader_size(BufReader::with_capacity(1, archive.by_index_seek(index)?))
            }
            CompressionMethod::Deflate => {
                // 解压器可能提前处理下一个块。逐字节供给压缩流，使探测停在头部；
                // 归档外层的 BufReader 合并底层文件读取，不逐字节访问磁盘。
                let compressed = BufReader::with_capacity(1, archive.by_index_raw(index)?);
                let decoded = flate2::bufread::DeflateDecoder::new(compressed);
                imagesize::reader_size(BufReader::with_capacity(
                    1,
                    header::HeaderReader::new(decoded, page.size),
                ))
            }
        };
        let dimensions = match size {
            Ok(size) => match (u32::try_from(size.width), u32::try_from(size.height)) {
                (Ok(width), Ok(height)) => Ok((width, height)),
                _ => Err("图片尺寸超出 u32 范围".to_owned()),
            },
            // imagesize 不容忍 JPEG 段之间的多余字节，从头顺序读一遍再试。
            Err(source) => {
                let raw = archive.by_index_raw(index)?.take(page.compressed_size);
                let fallback = match method {
                    CompressionMethod::Stored => jpeg::size(BufReader::new(raw)),
                    CompressionMethod::Deflate => jpeg::size(BufReader::new(
                        flate2::read::DeflateDecoder::new(raw).take(page.size),
                    )),
                };
                fallback.ok_or_else(|| source.to_string())
            }
        };
        match dimensions {
            Ok((width, height)) => {
                page.width = width;
                page.height = height;
            }
            Err(reason) => broken.push(BrokenPage {
                entry_name: page.entry_name.clone(),
                reason,
            }),
        }
        pages.push(page);
    }
    pages.sort_by(|a, b| page_order(&a.entry_name, &b.entry_name));
    Ok((pages, broken))
}

/// 按清单里的数据区间直接读页，不重新解析 ZIP 中央目录。`path` 是文件夹时读其中的图片文件。
pub fn read_page(path: impl AsRef<Path>, page: &Page) -> Result<Vec<u8>> {
    let path = path.as_ref();
    if path.is_dir() {
        return folder::read_folder_page(path, page);
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(page.data_offset))?;
    let mut reader = file.take(page.compressed_size);
    let mut bytes = Vec::new();
    match page.method {
        CompressionMethod::Stored => reader.read_to_end(&mut bytes)?,
        // 多读一个字节即可识别超出声明大小的流，不继续膨胀损坏条目的内容。
        CompressionMethod::Deflate => flate2::read::DeflateDecoder::new(reader)
            .take(page.size.saturating_add(1))
            .read_to_end(&mut bytes)?,
    };
    if bytes.len() as u64 != page.size {
        return Err(Error::CorruptEntry {
            entry_name: page.entry_name.clone(),
            reason: "实际解压大小与中央目录不符",
        });
    }
    Ok(bytes)
}

/// 读名为 `file_name` 的条目（不分大小写，可以在子目录里），有好几个时取路径最短的。
/// 没有这个条目时返回 `None`。用来读 ComicInfo.xml 这类小文件，超过 `limit` 字节视为损坏。
pub fn read_named_entry(
    path: impl AsRef<Path>,
    file_name: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>> {
    let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
    let wanted = file_name.to_lowercase();
    let found = (0..archive.len())
        .filter_map(|index| {
            // 名字不是合法编码的条目不可能是要找的文件，跳过。
            let name = archive.name_for_index(index)?.ok()?;
            let base = name.rsplit('/').next().unwrap_or(&name);
            (!name.ends_with('/') && base.to_lowercase() == wanted)
                .then(|| (name.chars().count(), index))
        })
        .min();
    let Some((_, index)) = found else {
        return Ok(None);
    };
    let entry = archive.by_index(index)?;
    let entry_name = entry.name()?.into_owned();
    if entry.size() > limit {
        return Err(Error::CorruptEntry {
            entry_name,
            reason: "条目超过大小上限",
        });
    }
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::CorruptEntry {
            entry_name,
            reason: "条目超过大小上限",
        });
    }
    Ok(Some(bytes))
}

/// 按扩展名判断是不是图片，忽略 macOS 的 `__MACOSX/` 和 `._` 文件。扫描用它找散放的图片。
pub fn is_image(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Thumbs.db 和 .DS_Store 没有图片扩展名，下面的扩展名检查已经排除它们。
    if lower
        .split('/')
        .any(|part| part == "__macosx" || part.starts_with("._"))
    {
        return false;
    }
    matches!(
        lower.rsplit_once('.').map(|(_, extension)| extension),
        Some("jpg" | "jpeg" | "jfif" | "png" | "gif" | "webp" | "avif" | "jxl" | "bmp")
    )
}

/// 页面顺序：不分大小写的自然序；`1.jpg` 和 `01.jpg`、`A.jpg` 和 `a.jpg` 这样相等的再按原名排，
/// 结果不随目录或 ZIP 里的枚举顺序变化，文件夹的指纹按这个顺序算才是确定的。
pub(crate) fn page_order(a: &str, b: &str) -> Ordering {
    natural_cmp(&a.to_lowercase(), &b.to_lowercase()).then_with(|| a.cmp(b))
}

/// 文件开头是不是 ZIP 的魔数。空 zip 只有结束记录，所以也接受 PK\x05\x06。
pub fn has_zip_magic(path: impl AsRef<Path>) -> Result<bool> {
    let mut magic = Vec::with_capacity(4);
    File::open(path)?.take(4).read_to_end(&mut magic)?;
    Ok(matches!(magic.as_slice(), b"PK\x03\x04" | b"PK\x05\x06"))
}

fn natural_cmp(mut a: &str, mut b: &str) -> Ordering {
    while !a.is_empty() && !b.is_empty() {
        if a.as_bytes()[0].is_ascii_digit() && b.as_bytes()[0].is_ascii_digit() {
            let a_end = a.bytes().take_while(u8::is_ascii_digit).count();
            let b_end = b.bytes().take_while(u8::is_ascii_digit).count();
            let a_number = a[..a_end].trim_start_matches('0');
            let b_number = b[..b_end].trim_start_matches('0');
            let order = a_number
                .len()
                .cmp(&b_number.len())
                .then_with(|| a_number.cmp(b_number));
            if order != Ordering::Equal {
                return order;
            }
            a = &a[a_end..];
            b = &b[b_end..];
        } else {
            let mut a_chars = a.chars();
            let mut b_chars = b.chars();
            let order = a_chars.next().cmp(&b_chars.next());
            if order != Ordering::Equal {
                return order;
            }
            a = a_chars.as_str();
            b = b_chars.as_str();
        }
    }
    a.len().cmp(&b.len())
}
