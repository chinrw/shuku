//! 从内容目录里删除一本书的原文件，以及判断能不能删。书库目录常常以只读方式挂载，
//! 这时网页端不提供删除文件，挂成读写后不用改设置就能删。

use crate::edit::conflict;
use crate::{Error, Result};
use shuku_store::{ArchiveMetadata, Page};
use std::io::ErrorKind;
use std::path::Path;

/// 删除散放图片的文件夹时，页面先挪进这个隐藏目录。扫描默认跳过以 `.` 开头的目录。
const STAGING: &str = ".shuku-deleting";

/// 删除 `path` 上这条记录对应的原文件，`path` 必须存在。调用方持有扫描锁。
pub(crate) fn remove_book(
    content: &Path,
    path: &Path,
    archive: &ArchiveMetadata,
    pages: &[Page],
) -> Result<()> {
    let is_dir = std::fs::symlink_metadata(path)?.is_dir();
    // 扫描不跟随符号链接，但之后上级目录可能被换成指向内容目录以外的链接，所以之后都用
    // 解析过的路径。检查和删除之间再换一次链接的竞争不防：能这样做的人本来就能改内容目录。
    let name = path.file_name().ok_or_else(Error::not_found)?;
    let parent = path.parent().ok_or_else(Error::not_found)?.canonicalize()?;
    if !parent.starts_with(content.canonicalize()?) {
        return Err(conflict("文件不在内容目录里，不删除"));
    }
    let path = parent.join(name);
    // 页面都删了、写库却失败时，文件夹因为还有别的文件留着。这时只剩记录要删，
    // 文件夹已经变了，不能再按身份拦下。
    if is_dir && no_pages_left(&path, pages)? {
        return Ok(());
    }
    if !unchanged(&path, is_dir, archive)? {
        return Err(conflict("文件在上次扫描后变过，扫描后再删除"));
    }
    if is_dir {
        return remove_folder_pages(&path, pages);
    }
    if !deletable(&path) {
        return Err(read_only());
    }
    Ok(std::fs::remove_file(&path)?)
}

/// 同一路径上换了一个文件时，删的就不是这条记录对应的书了。修改时间只精确到秒，
/// 所以再比一次快速指纹，同一秒里换成等长的文件也认得出。
fn unchanged(path: &Path, is_dir: bool, archive: &ArchiveMetadata) -> Result<bool> {
    let identity = shuku_archive::identity(path)?;
    if identity.size != archive.file_size || identity.modified != archive.mtime {
        return Ok(false);
    }
    let quick_hash = if is_dir {
        shuku_archive::folder_quick_hash(path)?
    } else {
        shuku_archive::quick_hash(path)?
    };
    Ok(quick_hash == archive.quick_hash)
}

/// 只删页面清单里的图片，文件夹里别的文件和子目录（可能是另一本书）不动，删完是空文件夹时
/// 才把文件夹也删掉。页面先全部挪进暂存目录再一起删，中途有一张挪不动（例如带粘滞位的目录里
/// 别人的文件）时把挪过的放回去，不留下缺页的书。
fn remove_folder_pages(dir: &Path, pages: &[Page]) -> Result<()> {
    if !writable_dir(dir) {
        return Err(read_only());
    }
    let names = pages
        .iter()
        .map(|page| page_name(&page.entry_name))
        .collect::<Result<Vec<_>>>()?;
    let staging = dir.join(STAGING);
    if let Err(error) = std::fs::create_dir(&staging) {
        return Err(if error.kind() == ErrorKind::AlreadyExists {
            conflict("文件夹里有上次没删完的 .shuku-deleting 目录，先手动处理")
        } else {
            error.into()
        });
    }
    let mut moved = Vec::new();
    for name in names {
        match std::fs::rename(dir.join(name), staging.join(name)) {
            Ok(()) => moved.push(name),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                restore(dir, &staging, &moved);
                return Err(error.into());
            }
        }
    }
    std::fs::remove_dir_all(&staging)?;
    if let Err(error) = std::fs::remove_dir(dir) {
        tracing::info!(dir = %dir.display(), %error, "文件夹里还有别的文件，保留文件夹");
    }
    Ok(())
}

fn restore(dir: &Path, staging: &Path, moved: &[&Path]) {
    for name in moved {
        if let Err(error) = std::fs::rename(staging.join(name), dir.join(name)) {
            tracing::error!(dir = %dir.display(), page = %name.display(), %error, "没能放回页面");
        }
    }
    if let Err(error) = std::fs::remove_dir(staging) {
        tracing::error!(dir = %staging.display(), %error, "没能删掉暂存目录");
    }
}

fn no_pages_left(dir: &Path, pages: &[Page]) -> Result<bool> {
    for page in pages {
        if dir.join(page_name(&page.entry_name)?).try_exists()? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 条目名是扫描时读到的文件名；带路径分隔符的不是这个文件夹里的文件。
fn page_name(entry_name: &str) -> Result<&Path> {
    let name = Path::new(entry_name);
    if name.components().count() != 1 || name.file_name().is_none() {
        return Err(Error::bad_request(format!(
            "页面“{entry_name}”不是文件夹里的文件名"
        )));
    }
    Ok(name)
}

fn read_only() -> Error {
    conflict("书库目录是只读的，不能删除文件")
}

/// 这个路径上的书能不能删：普通文件看所在目录，散放图片的文件夹看文件夹本身
/// （删的是里面的图片）。路径不存在或读不了时为假。
pub(crate) fn can_delete(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => writable_dir(path),
        Ok(_) => deletable(path),
        Err(_) => false,
    }
}

/// 删除一个文件要能写它所在的目录。
pub(crate) fn deletable(file: &Path) -> bool {
    file.parent().is_some_and(writable_dir)
}

/// 用 access(2) 而不是看权限位：只读挂载（EROFS）只有它能报出来，它也按服务进程的
/// 用户和组判断。
#[cfg(unix)]
pub(crate) fn writable_dir(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` 是以 NUL 结尾的有效 C 字符串，调用期间一直存活；access 不保留指针。
    unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

#[cfg(not(unix))]
pub(crate) fn writable_dir(dir: &Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|metadata| !metadata.permissions().readonly())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_in_a_writable_directory_can_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.zip");
        std::fs::write(&file, b"x").unwrap();
        assert!(can_delete(&file));
        assert!(can_delete(dir.path()));
        assert!(!can_delete(&dir.path().join("missing.zip")));
    }

    #[cfg(unix)]
    #[test]
    fn files_in_a_read_only_directory_cannot_be_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.zip");
        std::fs::write(&file, b"x").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        // root 不受权限位限制，这时跳过。
        // SAFETY: geteuid 没有前置条件，总是成功。
        let root = unsafe { libc::geteuid() } == 0;
        if !root {
            assert!(!can_delete(&file));
        }
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
