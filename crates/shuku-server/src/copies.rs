//! 一个书库条目可以从几份逐字节相同的 ZIP 读取。目录书不参与文件副本合并。
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use axum::{Json, extract::State};
use serde::Deserialize;
use shuku_store::Archive;

use crate::{App, Error, Result, edit::conflict, files};

impl App {
    /// 阅读不等下次扫描。不同下载的修改时间可能不同，要和各自路径入库时的值比较。
    pub(crate) fn source_file<'a>(&self, book: &'a Archive) -> Result<(&'a str, i64)> {
        let primary = (book.metadata.path.as_str(), book.metadata.mtime);
        if book.copies.is_empty() {
            return Ok(primary);
        }
        let mut changed = false;
        let mut recheck = Vec::new();
        for (name, mtime, stamp) in book.file_paths() {
            let path = self.0.config.content_dir.join(name);
            match path.symlink_metadata() {
                Ok(metadata)
                    if metadata.is_file()
                        && metadata.len() == book.metadata.file_size
                        && Some(file_stamp(&metadata)?) == stamp =>
                {
                    return Ok((name, mtime));
                }
                Ok(metadata) => {
                    changed = true;
                    if metadata.is_file() && metadata.len() == book.metadata.file_size {
                        recheck.push(name);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => {
                    tracing::debug!(%error,path=%path.display(),"不能读取副本");
                    changed = true;
                }
            }
        }
        // 删除一个硬链接也会改变其余链接的 ctime。没有可直接用的路径时再验完整指纹，
        // 不让仍在磁盘上的同一份内容一直等到下次扫描。
        if let Some(hash) = &book.metadata.content_hash {
            for name in recheck {
                if let Ok(Some((mtime, _))) =
                    matching_file(&self.0.config.content_dir.join(name), book, None, hash)
                {
                    return Ok((name, mtime));
                }
            }
        }
        if changed {
            return Err(conflict("副本文件已变化或无法确认，请重新扫描"));
        }
        Ok(primary)
    }

    /// 调用方持有扫描锁。在主文件失踪或变成别的内容时，先让原 ID 跟着旧内容，
    /// 再把改写的路径交给普通扫描作为新书处理。
    pub(crate) fn reconcile_copies(
        &self,
        files: &mut Vec<PathBuf>,
        folders: &std::collections::BTreeSet<PathBuf>,
        report: &mut crate::ScanReport,
    ) -> Result<HashSet<i64>> {
        let reader = self.reader()?;
        let books = reader.books_with_copies()?;
        let mut unknown = HashSet::new();
        if books.is_empty() {
            return Ok(unknown);
        }
        let walked: HashSet<_> = files.iter().map(PathBuf::as_path).collect();
        let mut skip = HashSet::new();
        let root = &self.0.config.content_dir;
        for id in books {
            let book = reader.archive(id)?.ok_or_else(Error::not_found)?;
            let hash = book
                .metadata
                .content_hash
                .as_deref()
                .ok_or_else(|| Error::File("副本记录缺少完整指纹".into()))?;
            let mut retained = Vec::new();
            let mut detached = Vec::new();
            let mut updated = Vec::new();
            for (name, mtime, stamp) in &book.copies {
                let path = root.join(name);
                if !walked.contains(path.as_path()) {
                    if folders.contains(&path) {
                        detached.push(name.as_str());
                    } else if !matches!(files::try_exists(&path), Ok(false)) {
                        unknown.insert(id);
                    }
                    continue;
                }
                match matching_file(&path, &book, *stamp, hash) {
                    Ok(Some((current, current_stamp))) => {
                        if current != *mtime || *stamp != Some(current_stamp) {
                            updated.push((name.as_str(), current, Some(current_stamp)));
                        }
                        retained.push((name.as_str(), current, Some(current_stamp)));
                        skip.insert(path);
                    }
                    Ok(None) => detached.push(name.as_str()),
                    Err(error) => {
                        self.scan_file_failed(&path, &error, report);
                        unknown.insert(id);
                        skip.insert(path);
                    }
                }
            }
            let primary = root.join(&book.metadata.path);
            let mut promote = None;
            let mut primary_time = None;
            let mut keep_old = false;
            let replacement = || {
                retained.first().copied().or_else(|| {
                    book.copies
                        .iter()
                        .find(|(name, ..)| !detached.contains(&name.as_str()))
                        .map(|(name, t, stamp)| (name.as_str(), *t, *stamp))
                })
            };
            if walked.contains(primary.as_path()) {
                match matching_file(&primary, &book, book.copy_stamp, hash) {
                    Ok(Some((t, stamp)))
                        if t != book.metadata.mtime || Some(stamp) != book.copy_stamp =>
                    {
                        primary_time = Some((t, stamp))
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => promote = replacement(),
                    Err(error) => {
                        self.scan_file_failed(&primary, &error, report);
                        unknown.insert(id);
                        skip.insert(primary.clone());
                    }
                }
            } else if folders.contains(&primary) {
                promote = replacement();
            } else if matches!(files::try_exists(&primary), Ok(false)) {
                promote = retained.first().copied();
                keep_old = true;
            }
            if !updated.is_empty()
                || !detached.is_empty()
                || promote.is_some()
                || primary_time.is_some()
            {
                self.write(|writer| {
                    for name in detached {
                        writer.remove_copy(id, name)?;
                    }
                    for (name, t, stamp) in updated {
                        writer.record_copy(id, name, t, stamp)?;
                    }
                    if let Some((t, stamp)) = primary_time {
                        writer.set_primary_file(id, &book.metadata.path, t, Some(stamp))?;
                    }
                    if let Some((name, t, stamp)) = promote {
                        writer.set_primary_file(id, name, t, stamp)?;
                        skip.remove(&root.join(name));
                        if keep_old {
                            writer.record_copy(
                                id,
                                &book.metadata.path,
                                book.metadata.mtime,
                                book.copy_stamp,
                            )?;
                        }
                    }
                    Ok(())
                })?;
            }
        }
        files.retain(|p| !skip.contains(p));
        Ok(unknown)
    }

    pub(crate) fn group_hashed_copies(&self) -> Result<()> {
        let _scan = self.0.scan_lock.lock().map_err(|_| Error::Lock)?;
        let pairs = self.reader()?.automatic_copy_pairs()?;
        let mut changed = false;
        for (keep, other) in pairs {
            if self.0.stopping.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            match self.merge_verified_copies(keep, &[other], None) {
                Ok(()) => changed = true,
                Err(Error::Http { status, .. }) if status == axum::http::StatusCode::CONFLICT => {
                    continue;
                }
                Err(error) => {
                    if changed {
                        self.listing_changed();
                    }
                    return Err(error);
                }
            }
        }
        if changed {
            self.refresh_blocking()?;
        }
        Ok(())
    }

    /// 磁盘验证不占写锁。一次手动合并选中的所有归档在同一个事务内迁移。
    fn merge_verified_copies(
        &self,
        keep: i64,
        others: &[i64],
        expected: Option<&str>,
    ) -> Result<()> {
        let reader = self.reader()?;
        let target = reader.archive(keep)?.ok_or_else(Error::not_found)?;
        let hash = target
            .metadata
            .content_hash
            .as_deref()
            .ok_or_else(|| conflict("尚未计算完整指纹"))?;
        if expected.is_some_and(|expected| expected != hash) {
            return Err(conflict("文件指纹已经变化，请刷新预览"));
        }
        if others.is_empty() || others.contains(&keep) {
            return Err(Error::bad_request("请选择不同的归档"));
        }
        let mut available = None;
        let mut verify = |book: &Archive| -> Result<Option<i64>> {
            if book.metadata.content_hash.as_deref() != Some(hash)
                || book.metadata.quick_hash != target.metadata.quick_hash
            {
                return Err(conflict("完整指纹不一致，不能合并"));
            }
            if reader
                .pages(book.id)?
                .first()
                .is_none_or(|page| page.data_offset == 0)
            {
                return Err(conflict("目录书不按文件副本合并"));
            }
            let mut primary_stamp = None;
            for (name, mtime, known_stamp) in book.file_paths() {
                let path = self.0.config.content_dir.join(name);
                if !files::try_exists(&path)? {
                    continue;
                }
                let Some((verified_time, stamp)) = matching_file(&path, book, known_stamp, hash)?
                else {
                    return Err(conflict("文件内容已变化，请重新扫描"));
                };
                if verified_time != mtime {
                    return Err(conflict("文件修改时间已变化，请重新扫描"));
                }
                available.get_or_insert_with(|| (name.to_owned(), mtime, stamp));
                if name == book.metadata.path {
                    primary_stamp = Some(stamp);
                }
            }
            Ok(primary_stamp)
        };
        let target_stamp = verify(&target)?;
        let mut checked = Vec::new();
        for &id in others {
            let book = reader.archive(id)?.ok_or_else(Error::not_found)?;
            checked.push((id, verify(&book)?));
        }
        if available.is_none() {
            self.content_mounted()?;
        }
        self.write(|writer| {
            for (other, stamp) in checked {
                if !writer.merge_copy(keep, other, hash, target_stamp, stamp)? {
                    return Err(conflict("归档已变化，请刷新后再合并"));
                }
            }
            writer.set_missing(keep, available.is_none())?;
            if let Some((name, t, stamp)) = available
                && name != target.metadata.path
            {
                writer.set_primary_file(keep, &name, t, Some(stamp))?;
                writer.record_copy(
                    keep,
                    &target.metadata.path,
                    target.metadata.mtime,
                    target_stamp,
                )?;
            }
            Ok(())
        })
    }
}

/// 扫描身份中的 mtime 只有秒级精度。副本在同一秒被等长替换时也要重新核对完整指纹。
#[cfg(unix)]
pub(crate) fn file_stamp(metadata: &std::fs::Metadata) -> Result<i64> {
    use std::os::unix::fs::MetadataExt;
    Ok(metadata
        .ctime()
        .wrapping_mul(1_000_000_000)
        .wrapping_add(metadata.ctime_nsec()))
}

#[cfg(not(unix))]
pub(crate) fn file_stamp(metadata: &std::fs::Metadata) -> Result<i64> {
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| Error::File(error.to_string()))?;
    Ok(modified.as_nanos() as i64)
}

fn matching_file(
    path: &Path,
    book: &Archive,
    previous: Option<i64>,
    hash: &str,
) -> Result<Option<(i64, i64)>> {
    let meta = path.symlink_metadata()?;
    if !meta.is_file() || meta.len() != book.metadata.file_size {
        return Ok(None);
    }
    let stamp = file_stamp(&meta)?;
    if previous != Some(stamp) {
        if shuku_archive::content_hash(path)? != hash {
            return Ok(None);
        }
        if file_stamp(&path.symlink_metadata()?)? != stamp {
            return Err(Error::File("核对副本期间文件发生变化".into()));
        }
    }
    Ok(Some((crate::scan::unix_seconds(meta.modified()?)?, stamp)))
}

#[derive(Deserialize)]
pub(crate) struct MergeRequest {
    keep: i64,
    others: Vec<i64>,
    hash: String,
}

pub(crate) async fn merge(
    State(app): State<App>,
    Json(req): Json<MergeRequest>,
) -> Result<Json<serde_json::Value>> {
    app.blocking(move |app| {
        let _scan = app.0.scan_lock.lock().map_err(|_| Error::Lock)?;
        app.merge_verified_copies(req.keep, &req.others, Some(&req.hash))?;
        app.refresh_blocking()?;
        Ok(Json(serde_json::json!({"id":req.keep})))
    })
    .await
}

/// 仅给已合并 ID 的读取请求续上旧链接。过期的删除请求不能替用户删除保留的书。
pub(crate) async fn redirect_old_link(
    State(app): State<App>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response> {
    use axum::{
        http::{Method, StatusCode},
        response::{IntoResponse, Redirect},
    };
    if request.method() != Method::GET || !request.uri().path().starts_with("/api/archives/") {
        return Ok(next.run(request).await);
    }
    let uri = request.uri().clone();
    let response = next.run(request).await;
    if response.status() != StatusCode::NOT_FOUND {
        return Ok(response);
    }
    let rest = &uri.path()["/api/archives/".len()..];
    let (old, suffix) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let Ok(id) = old.parse::<i64>() else {
        return Ok(response);
    };
    let Some(id) = app
        .blocking(move |app| Ok(app.reader()?.redirected_archive(id)?))
        .await?
    else {
        return Ok(response);
    };
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    Ok(Redirect::temporary(&format!("/api/archives/{id}{suffix}{query}")).into_response())
}
