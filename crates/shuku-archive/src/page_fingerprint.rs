//! 入库前查重比较图片文件的内容，不比较 ZIP 的布局、条目名或元数据。
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek},
    path::Path,
};

use sha2::{Digest, Sha256};

use crate::{Error, MAX_PAGE_SIZE, Result, content_hash_file, folder, identity, is_image};

/// 固定长度的页摘要按字节排序后再做一次 SHA-256。不要去重：两张相同的页仍是两页。
pub fn page_set_signature(digests: &mut [[u8; 32]]) -> [u8; 32] {
    digests.sort_unstable();
    let mut signature = Sha256::new();
    for digest in digests {
        signature.update(digest);
    }
    signature.finalize().into()
}

/// 只返回属于指定整文件（或散图目录）BLAKE3 的页面签名。文件在后台读取时
/// 可能已被下载器换掉；不能把新内容的页摘要登记到旧的归档版本上。
pub fn verified_page_set(path: &Path, expected: &str) -> Result<Option<(usize, [u8; 32])>> {
    let mut pages = Vec::new();
    if path.is_dir() {
        let before = identity(path)?;
        if folder::hash_folder(path, Some(&mut pages))? != expected || identity(path)? != before {
            return Ok(None);
        }
    } else {
        let mut file = File::open(path)?;
        let before = file.metadata()?;
        if content_hash_file(&mut file)? != expected {
            return Ok(None);
        }
        file.rewind()?;
        let mut zip = zip::ZipArchive::new(BufReader::new(&mut file))?;
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index)?;
            let name = entry.name()?.into_owned();
            if entry.is_dir() || !is_image(&name) {
                continue;
            }
            let size = entry.size();
            drop(entry);
            pages.push(hash_page(zip.by_index(index)?, &name, size, None)?);
        }
        drop(zip);
        let after = file.metadata()?;
        if before.modified()? != after.modified()? || before.len() != after.len() {
            return Ok(None);
        }
    }
    Ok(Some((pages.len(), page_set_signature(&mut pages))))
}

pub(super) fn hash_page(
    reader: impl Read,
    name: &str,
    size: u64,
    mut folder_hash: Option<&mut blake3::Hasher>,
) -> Result<[u8; 32]> {
    if size > MAX_PAGE_SIZE {
        return Err(Error::CorruptEntry {
            entry_name: name.into(),
            reason: "图片超过单页大小上限",
        });
    }
    let mut reader = BufReader::with_capacity(64 * 1024, reader.take(size + 1));
    let mut digest = Sha256::new();
    let mut length = 0;
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            break;
        }
        digest.update(bytes);
        if let Some(content) = &mut folder_hash {
            content.update(bytes);
        }
        length += bytes.len() as u64;
        let consumed = bytes.len();
        reader.consume(consumed);
    }
    if length != size {
        return Err(Error::CorruptEntry {
            entry_name: name.into(),
            reason: "页面长度与清单不符",
        });
    }
    Ok(digest.finalize().into())
}
