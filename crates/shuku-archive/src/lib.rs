//! ZIP 页面清单、定位读取与内容指纹。

mod fingerprint;
mod header;

pub use fingerprint::{content_hash, content_hash_file, quick_hash, quick_hash_file};

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
    #[error("无法读取图片 {entry_name} 的尺寸：{source}")]
    Image {
        entry_name: String,
        #[source]
        source: imagesize::ImageError,
    },
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

/// 解析中央目录并探测图片尺寸，不解码像素，也不写入内容目录。
pub fn list_pages(path: impl AsRef<Path>) -> Result<Vec<Page>> {
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
        }
        .map_err(|source| Error::Image {
            entry_name: page.entry_name.clone(),
            source,
        })?;
        let (Ok(width), Ok(height)) = (u32::try_from(size.width), u32::try_from(size.height))
        else {
            return Err(Error::CorruptEntry {
                entry_name: page.entry_name,
                reason: "图片尺寸超出 u32 范围",
            });
        };
        page.width = width;
        page.height = height;
        pages.push(page);
    }
    pages.sort_by(|a, b| natural_cmp(&a.entry_name.to_lowercase(), &b.entry_name.to_lowercase()));
    Ok(pages)
}

/// 按清单里的数据区间直接读页，不重新解析 ZIP 中央目录。
pub fn read_page(path: impl AsRef<Path>, page: &Page) -> Result<Vec<u8>> {
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

fn is_image(name: &str) -> bool {
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
        Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "jxl" | "bmp")
    )
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
