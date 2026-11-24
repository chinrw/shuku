//! 入库前只读查重。查到相同页面不是整文件相同的证据，不能据此合并副本。
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
};
use serde::{Deserialize, Serialize};
use shuku_store::Archive;
use std::sync::atomic::Ordering;

use crate::{App, Error, Result, copies::file_stamp};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    page_sha256: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(crate) enum Coverage {
    ScanNotStarted,
    ScanInProgress,
    ScanChanged,
    ScanFailed,
    ArchiveFormatFailures { failed_archives: usize },
    PendingFingerprints,
    IndexedMatchChanged,
}

#[derive(Serialize)]
pub(crate) struct Response {
    status: &'static str,
    matches: Vec<Match>,
    pending_archives: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<Coverage>,
}

#[derive(Serialize)]
struct Match {
    id: i64,
    path: String,
}

fn sha256_bytes(hex: &str) -> Result<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::bad_request(
            "page_sha256 的每项必须是 64 位十六进制 SHA-256",
        ));
    }
    let mut digest = [0; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(Error::bad_request)?;
    }
    Ok(digest)
}

pub(crate) async fn check(
    State(app): State<App>,
    body: std::result::Result<Json<Request>, JsonRejection>,
) -> Result<Json<Response>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    if body.page_sha256.is_empty() {
        return Err(Error::bad_request("至少需要一页的 SHA-256"));
    }
    app.blocking(move |app| {
        let mut digests = body
            .page_sha256
            .iter()
            .map(|hash| sha256_bytes(hash))
            .collect::<Result<Vec<_>>>()?;
        let signature = shuku_archive::page_set_signature(&mut digests);
        // 跨过扫描边界时可以返回已知命中，不能把逐批提交中的清单当作全库。
        let inventory = app.tasks(|tasks| tasks.scan.inventory());
        let (pending_archives, books) = app.reader()?.lookup_page_fingerprint(&signature)?;
        let mut indexed_match_changed = false;
        let mut matches = Vec::new();
        for book in books {
            if let Some(path) = app.indexed_file(&book)? {
                matches.push(Match { id: book.id, path });
            } else {
                indexed_match_changed = true;
            }
        }
        let coverage = if inventory != app.tasks(|tasks| tasks.scan.inventory()) {
            Some(Coverage::ScanChanged)
        } else if indexed_match_changed {
            Some(Coverage::IndexedMatchChanged)
        } else if pending_archives > 0
            && matches!(
                inventory.1,
                None | Some(Coverage::ArchiveFormatFailures { .. })
            )
        {
            Some(Coverage::PendingFingerprints)
        } else {
            inventory.1
        };
        let status = if coverage.is_some() {
            "unknown"
        } else if matches.is_empty() {
            "no_duplicate"
        } else {
            "duplicate"
        };
        Ok(Json(Response {
            status,
            matches,
            pending_archives,
            coverage,
        }))
    })
    .await
}

impl App {
    /// 只核实命中的路径，不把未扫描到的改写或移走的文件作为跳过下载的依据。
    fn indexed_file(&self, book: &Archive) -> Result<Option<String>> {
        for (name, mtime, stamp) in book.file_paths() {
            let path = self.0.config.content_dir.join(name);
            let Ok(meta) = path.symlink_metadata() else {
                continue;
            };
            let present = if meta.is_file() {
                meta.len() == book.metadata.file_size && Some(file_stamp(&meta)?) == stamp
            } else {
                meta.is_dir()
                    && shuku_archive::identity(&path)
                        .is_ok_and(|i| i.modified == mtime && i.size == book.metadata.file_size)
            };
            if present {
                return Ok(Some(name.to_owned()));
            }
        }
        Ok(None)
    }

    pub(crate) fn backfill_page_fingerprints(&self) -> Result<()> {
        let mut after = 0;
        while !self.0.stopping.load(Ordering::Relaxed) {
            let batch = self.reader()?.pending_page_fingerprints(after, 32)?;
            let Some(&last) = batch.last() else { break };
            after = last;
            let mut verified = Vec::new();
            for id in batch {
                if self.0.stopping.load(Ordering::Relaxed) {
                    break;
                }
                let Some(book) = self.reader()?.archive(id)? else {
                    continue;
                };
                match self.page_fingerprint(&book) {
                    Ok(Some((digest, stamp))) => verified.push((book, digest, stamp)),
                    Ok(None) => {}
                    Err(error) => tracing::warn!(id,%error,"没有取得完整的页面指纹"),
                }
            }
            if !verified.is_empty() {
                self.write(|writer| {
                    for (book, digest, stamp) in &verified {
                        writer.record_page_fingerprint(book, digest, *stamp)?;
                    }
                    Ok(())
                })?;
            }
        }
        Ok(())
    }

    fn page_fingerprint(&self, book: &Archive) -> Result<Option<([u8; 32], Option<i64>)>> {
        let Some(expected) = book.metadata.content_hash.as_deref() else {
            return Ok(None);
        };
        let (name, mtime) = self.source_file(book)?;
        let path = self.0.config.content_dir.join(name);
        let meta = path.symlink_metadata()?;
        let stamp = if meta.is_file() {
            Some(file_stamp(&meta)?)
        } else if meta.is_dir() {
            None
        } else {
            return Ok(None);
        };
        let before = shuku_archive::identity(&path)?;
        if before.modified != mtime || before.size != book.metadata.file_size {
            return Ok(None);
        }
        let Some((count, digest)) = shuku_archive::verified_page_set(&path, expected)? else {
            return Ok(None);
        };
        if count != book.page_count as usize || shuku_archive::identity(&path)? != before {
            return Ok(None);
        }
        if let Some(stamp) = stamp
            && file_stamp(&path.symlink_metadata()?)? != stamp
        {
            return Ok(None);
        }
        Ok(Some((digest, stamp.filter(|_| name == book.metadata.path))))
    }
}
