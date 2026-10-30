use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::Result;

/// 计算 BLAKE3(size_le64 || head || tail)，返回小写十六进制指纹。
/// 首尾各取最多 64 KiB；小于 128 KiB 时保留两段中重叠的字节。
pub fn quick_hash(path: impl AsRef<Path>) -> Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut sample = [0_u8; 64 * 1024];
    let sample_len = size.min(sample.len() as u64) as usize;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    file.read_exact(&mut sample[..sample_len])?;
    hasher.update(&sample[..sample_len]);
    file.seek(SeekFrom::Start(size - sample_len as u64))?;
    file.read_exact(&mut sample[..sample_len])?;
    hasher.update(&sample[..sample_len]);
    Ok(hasher.finalize().to_hex().to_string())
}

/// 流式计算整文件 BLAKE3，返回小写十六进制指纹。
pub fn content_hash(path: impl AsRef<Path>) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok(hasher.finalize().to_hex().to_string())
}
