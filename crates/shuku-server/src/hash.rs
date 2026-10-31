use crate::{App, Result, db, scan::unix_seconds};
use shuku_store::ScanEntry;
use std::{
    fs::File,
    io::ErrorKind,
    path::Path,
    sync::{atomic::Ordering, mpsc},
};

/// 每次从数据库取这么多本，取的时候不持有锁太久，也不把 30 万条一次读进内存。
const BATCH: usize = 256;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct HashReport {
    pub hashed: usize,
    /// 文件不在了，或者自扫描以来变过，留给下次扫描后再算。
    pub skipped: usize,
    pub failed: usize,
}

impl App {
    /// 给还没有完整指纹的归档补算整文件 BLAKE3。逐本顺序读，不和读页争抢磁盘；
    /// 结果经写队列合并提交，写入时再核对文件身份。调用 [`App::stop`] 后在
    /// 当前文件算完时返回，所以可以随时中断，下次从没算过的归档继续。
    pub async fn hash_contents(&self) -> Result<HashReport> {
        self.blocking(|app| app.hash_contents_blocking()).await
    }

    fn hash_contents_blocking(&self) -> Result<HashReport> {
        let Ok(_hash) = self.0.hash_lock.lock() else {
            self.tasks(|tasks| tasks.hash.fail(&crate::Error::Lock));
            return Err(crate::Error::Lock);
        };
        // 和扫描一样，结束状态在持有锁时写入。
        let _interrupted = crate::admin::FailOnUnwind::hash(self);
        let result = self.hash_pending().and_then(|pending| {
            self.tasks(|tasks| tasks.hash.start(pending));
            self.hash_contents_running()
        });
        self.tasks(|tasks| tasks.hash.finish(&result));
        result
    }

    fn hash_pending(&self) -> Result<u64> {
        let stats = self.reader()?.stats()?;
        Ok(stats.archives.saturating_sub(stats.hashed))
    }

    fn hash_contents_running(&self) -> Result<HashReport> {
        let mut report = HashReport::default();
        // 算好的指纹交给写队列合并提交，不等它写完就算下一本；回复在这里清点。
        let (reply, replies) = mpsc::channel();
        let mut queued = 0;
        let mut after = 0;
        while !self.0.stopping.load(Ordering::Relaxed) {
            let batch = self.reader()?.missing_content_hashes(after, BATCH)?;
            let Some(last) = batch.last() else { break };
            after = last.id;
            for entry in batch {
                if self.0.stopping.load(Ordering::Relaxed) {
                    break;
                }
                let path = self.0.config.content_dir.join(&entry.path);
                match hash_unchanged(&path, &entry) {
                    Ok(Some(hash)) => {
                        self.enqueue(db::Request::ContentHash(db::HashRequest {
                            entry,
                            hash,
                            reply: reply.clone(),
                        }))?;
                        queued += 1;
                    }
                    Ok(None) => report.skipped += 1,
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "计算完整指纹失败");
                        report.failed += 1;
                    }
                }
            }
            for written in replies.try_iter() {
                count(&mut report, written?);
                queued -= 1;
            }
            self.tasks(|tasks| tasks.hash.report = report.clone());
        }
        drop(reply);
        for written in replies {
            count(&mut report, written?);
            queued -= 1;
        }
        // 写队列线程退出时会丢掉没回复的请求。
        if queued > 0 {
            return Err(db::queue_stopped());
        }
        tracing::info!(?report, "完整指纹计算结束");
        Ok(report)
    }
}

/// 写入时文件身份已经和算指纹时不同，说明期间扫描换了文件，算作跳过。
fn count(report: &mut HashReport, written: bool) {
    if written {
        report.hashed += 1;
    } else {
        report.skipped += 1;
    }
}

/// 文件仍是扫描时记录的大小、修改时间和快速指纹，并且读的过程中没有变化时，返回整文件指纹。
/// 元数据核对和两种指纹都在同一个打开的句柄上做：按路径分几次打开的话，
/// 路径中途被原子替换成大小和修改时间都相同的另一个文件时，会把它的指纹存到这条记录上。
fn hash_unchanged(path: &Path, entry: &ScanEntry) -> Result<Option<String>> {
    if path.is_dir() {
        return hash_folder_unchanged(path, entry);
    }
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

/// 散放图片的文件夹：没法像单个文件那样在同一个句柄上核对，就在读之前和之后各核对一次
/// 文件夹的大小和修改时间，再核对快速指纹。期间有图片被等长替换且修改时间不变的情况认不出来，
/// 这和扫描判断文件夹是否变过的依据一致。
fn hash_folder_unchanged(path: &Path, entry: &ScanEntry) -> Result<Option<String>> {
    let identity = |path: &Path| -> Result<Option<(u64, i64)>> {
        match shuku_archive::identity(path) {
            Ok((size, modified)) => Ok(Some((size, unix_seconds(modified)?))),
            Err(shuku_archive::Error::Io(error)) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    };
    let before = identity(path)?;
    if before != Some((entry.file_size, entry.mtime))
        || shuku_archive::folder_quick_hash(path)? != entry.quick_hash
    {
        return Ok(None);
    }
    let hash = shuku_archive::folder_content_hash(path)?;
    if identity(path)? != before {
        return Ok(None);
    }
    Ok(Some(hash))
}
