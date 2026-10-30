use crate::{App, Result, scan::unix_seconds};
use shuku_store::ScanEntry;
use std::{fs::File, io::ErrorKind, path::Path, sync::atomic::Ordering};

/// 每次从数据库取这么多本，取的时候不持有锁太久，也不把 30 万条一次读进内存。
const BATCH: usize = 256;

#[derive(Debug, Default)]
pub struct HashReport {
    pub hashed: usize,
    /// 文件不在了，或者自扫描以来变过，留给下次扫描后再算。
    pub skipped: usize,
    pub failed: usize,
}

impl App {
    /// 给还没有完整指纹的归档补算整文件 BLAKE3。逐本顺序读，不和读页争抢磁盘；
    /// 计算时不持有数据库锁，写入时再核对文件身份。调用 [`App::stop`] 后在
    /// 当前文件算完时返回，所以可以随时中断，下次从没算过的归档继续。
    pub async fn hash_contents(&self) -> Result<HashReport> {
        self.blocking(|app| app.hash_contents_blocking()).await
    }

    fn hash_contents_blocking(&self) -> Result<HashReport> {
        let mut report = HashReport::default();
        let mut after = 0;
        while !self.0.stopping.load(Ordering::Relaxed) {
            let batch = self.store()?.missing_content_hashes(after, BATCH)?;
            let Some(last) = batch.last() else { break };
            after = last.id;
            for entry in batch {
                if self.0.stopping.load(Ordering::Relaxed) {
                    break;
                }
                let path = self.0.config.content_dir.join(&entry.path);
                match hash_unchanged(&path, &entry) {
                    Ok(Some(hash)) if self.store()?.set_content_hash(&entry, &hash)? => {
                        report.hashed += 1
                    }
                    Ok(_) => report.skipped += 1,
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "计算完整指纹失败");
                        report.failed += 1;
                    }
                }
            }
        }
        tracing::info!(?report, "完整指纹计算结束");
        Ok(report)
    }
}

/// 文件仍是扫描时记录的大小、修改时间和快速指纹，并且读的过程中没有变化时，返回整文件指纹。
/// 元数据核对和两种指纹都在同一个打开的句柄上做：按路径分几次打开的话，
/// 路径中途被原子替换成大小和修改时间都相同的另一个文件时，会把它的指纹存到这条记录上。
fn hash_unchanged(path: &Path, entry: &ScanEntry) -> Result<Option<String>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let before = file.metadata()?;
    if before.len() != entry.file_size || unix_seconds(before.modified()?)? != entry.mtime {
        return Ok(None);
    }
    // 修改时间只记到秒，扫描后同一秒内被等长重写的文件靠大小和修改时间认不出来，
    // 再核对快速指纹，免得把新内容的指纹写到描述旧内容的记录上。
    if shuku_archive::quick_hash_file(&mut file)? != entry.quick_hash {
        return Ok(None);
    }
    let hash = shuku_archive::content_hash_file(&mut file)?;
    let after = file.metadata()?;
    if after.len() != before.len() || after.modified()? != before.modified()? {
        return Ok(None);
    }
    Ok(Some(hash))
}
