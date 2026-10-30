use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::Result;

/// 计算 BLAKE3(size_le64 || head || tail)，返回小写十六进制指纹。
/// 首尾各取最多 64 KiB；小于 128 KiB 时保留两段中重叠的字节。
pub fn quick_hash(path: impl AsRef<Path>) -> Result<String> {
    quick_hash_file(&mut File::open(path)?)
}

/// 同 [`quick_hash`]，但在已打开的句柄上计算，调用方可以用同一个句柄
/// 核对元数据和算完整指纹，路径在中途被替换也不会混到另一个文件。
pub fn quick_hash_file(file: &mut File) -> Result<String> {
    let size = file.metadata()?.len();
    let mut sample = [0_u8; 64 * 1024];
    let sample_len = size.min(sample.len() as u64) as usize;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut sample[..sample_len])?;
    hasher.update(&sample[..sample_len]);
    file.seek(SeekFrom::Start(size - sample_len as u64))?;
    file.read_exact(&mut sample[..sample_len])?;
    hasher.update(&sample[..sample_len]);
    Ok(hasher.finalize().to_hex().to_string())
}

/// 流式计算整文件 BLAKE3，返回小写十六进制指纹。
pub fn content_hash(path: impl AsRef<Path>) -> Result<String> {
    content_hash_file(&mut File::open(path)?)
}

/// 同 [`content_hash`]，但在已打开的句柄上计算。先回到文件开头，
/// 所以句柄之前读到哪里都不影响结果。
pub fn content_hash_file(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file)?;
    Ok(hasher.finalize().to_hex().to_string())
}
