//! 改一本书的标题、简介和标签，标签改名合并，删除文件已经不在的记录。
//! 网页端、以后的内置插件和 MCP 都经过 [`App::edit_archive`] 写元数据，校验只在这里做一次。
//! M1 没有认证，这些写操作和进度上报一样只在局域网里用。
//!
//! 写入提交后只安排一次延迟的快照刷新：详情直接读库，马上是新的；搜索结果和列表最多
//! 落后一个刷新延迟。插件一次改很多本时，这些改动合并成一次重建。

use crate::{App, Error, Result};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use shuku_store::Tag;
use std::sync::TryLockError;

/// 标题的长度上限（字符数）。E-Hentai 的标题一般在 300 字以内。
const TITLE_LIMIT: usize = 1000;
const SUMMARY_LIMIT: usize = 20_000;
const SOURCE_LIMIT: usize = 2000;
/// 单个标签（含命名空间）的长度上限。
const TAG_LIMIT: usize = 500;
/// 一次请求里最多给出的标签数，也是一次最多删除的记录数。
const BATCH_LIMIT: usize = 1000;

/// 一本书的元数据改动，`None` 的字段不变。
#[derive(Debug, Default)]
pub(crate) struct ArchiveEdit {
    pub(crate) title: Option<String>,
    pub(crate) summary: Option<String>,
    /// `Some(None)` 清除来源。
    pub(crate) source_url: Option<Option<String>>,
    pub(crate) tags: TagEdit,
}

#[derive(Debug, Default)]
pub(crate) enum TagEdit {
    #[default]
    Keep,
    /// 整体替换全部标签，网页端的编辑表单用它。
    Replace(Vec<Tag>),
    /// 先加后删，插件补标签时用它，不碰用户自己加的标签。
    Merge { add: Vec<Tag>, remove: Vec<Tag> },
}

impl ArchiveEdit {
    /// 字段要求已经去掉首尾空白，[`EditBody`] 在解析请求时做了这一步。
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(title) = &self.title {
            valid_title(title)?;
        }
        if let Some(summary) = &self.summary
            && summary.chars().count() > SUMMARY_LIMIT
        {
            return Err(Error::bad_request(format!(
                "简介不能超过 {SUMMARY_LIMIT} 个字"
            )));
        }
        if let Some(Some(url)) = &self.source_url {
            valid_source(url)?;
        }
        match &self.tags {
            TagEdit::Keep => Ok(()),
            TagEdit::Replace(tags) => valid_tags(tags),
            TagEdit::Merge { add, remove } => {
                valid_tags(add)?;
                valid_tags(remove)
            }
        }
    }

    /// 在调用方的事务里写入，不校验，也不安排快照刷新。扫描在自己的批次事务里
    /// 套用插件结果时用它，之前要先调用 [`ArchiveEdit::validate`]。
    pub(crate) fn apply(&self, writer: &mut shuku_store::Writer<'_>, id: i64) -> Result<()> {
        if self.title.is_some() || self.summary.is_some() {
            writer.edit_archive(id, self.title.as_deref(), self.summary.as_deref())?;
        }
        if let Some(url) = &self.source_url {
            writer.set_source_url(id, url.as_deref())?;
        }
        match &self.tags {
            TagEdit::Keep => {}
            TagEdit::Replace(tags) => writer.replace_tags(id, tags)?,
            TagEdit::Merge { add, remove } => {
                writer.add_tags(id, add)?;
                writer.remove_tags(id, remove)?;
            }
        }
        Ok(())
    }
}

/// 删除一条记录的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Deletion {
    Deleted,
    /// 文件还在内容目录里，或者无法确认它不在。删了记录，下次扫描会把它当作新书入库。
    FilePresent,
    NotFound,
}

impl App {
    pub(crate) fn edit_router(&self) -> Router<App> {
        Router::new()
            .route(
                "/api/archives/{id}",
                get(crate::api::detail).patch(edit).delete(delete),
            )
            .route("/api/admin/missing/delete", post(delete_missing))
            .route("/api/admin/tags/rename", post(rename_tag))
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }

    /// 校验后在一个事务里写入，阻塞调用。
    pub(crate) fn edit_archive(&self, id: i64, edit: &ArchiveEdit) -> Result<()> {
        edit.validate()?;
        self.write(|writer| edit.apply(writer, id))?;
        self.listing_changed();
        Ok(())
    }

    /// 删除文件已经不在内容目录里的记录，阻塞调用。扫描进行中时拒绝：扫描开始时读出的
    /// 记录可能正要被同内容的新文件认领，删掉会让那一批提交失败。持有扫描锁期间检查
    /// 文件和删除，中间不会有扫描插进来。
    pub(crate) fn delete_missing(&self, ids: &[i64]) -> Result<Vec<Deletion>> {
        let _scan = match self.0.scan_lock.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Err(conflict("扫描正在进行，结束后再删除"));
            }
            Err(TryLockError::Poisoned(_)) => return Err(Error::Lock),
        };
        let content = &self.0.config.content_dir;
        // 内容目录在网络盘上、没挂载时看起来是空的，这时每本书都像是丢了。
        if std::fs::read_dir(content)?.next().is_none() {
            return Err(conflict("内容目录是空的，可能没有挂载，不删除记录"));
        }
        let outcomes = {
            let reader = self.reader()?;
            ids.iter()
                .map(|&id| {
                    Ok(match reader.archive(id)? {
                        None => Deletion::NotFound,
                        // 读不了的目录里的文件无法确认不在，按还在处理。
                        Some(archive) => match content.join(&archive.metadata.path).try_exists() {
                            Ok(false) => Deletion::Deleted,
                            Ok(true) | Err(_) => Deletion::FilePresent,
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?
        };
        // 同一个 ID 给了两次时只删一次，第二次删除会因为找不到记录让整批回滚。
        let mut deleted: Vec<i64> = ids
            .iter()
            .zip(&outcomes)
            .filter(|(_, outcome)| **outcome == Deletion::Deleted)
            .map(|(&id, _)| id)
            .collect();
        deleted.sort_unstable();
        deleted.dedup();
        if deleted.is_empty() {
            return Ok(outcomes);
        }
        self.write(|writer| {
            for &id in &deleted {
                writer.delete_archive(id)?;
            }
            Ok(())
        })?;
        tracing::info!(count = deleted.len(), ids = ?deleted, "删除了文件已不在的记录");
        self.tasks(|tasks| tasks.scan.issues.missing.remove(&deleted));
        self.listing_changed();
        Ok(outcomes)
    }
}

fn conflict(message: &str) -> Error {
    Error::Http {
        status: StatusCode::CONFLICT,
        message: message.into(),
    }
}

/// 网页端和 API 用 `命名空间:值` 的写法，和 LRR 一样在第一个冒号处分开；没有冒号的
/// 是不带命名空间的标签。首尾和冒号两边的空白去掉。
pub(crate) fn parse_tag(text: &str) -> Result<Tag> {
    let (namespace, value) = text.split_once(':').unwrap_or(("", text));
    let tag = Tag {
        namespace: namespace.trim().into(),
        value: value.trim().into(),
    };
    valid_tags(std::slice::from_ref(&tag))?;
    Ok(tag)
}

fn parse_tags(texts: &[String]) -> Result<Vec<Tag>> {
    texts.iter().map(|text| parse_tag(text)).collect()
}

fn valid_title(title: &str) -> Result<()> {
    if title.trim().is_empty() || title.trim() != title {
        return Err(Error::bad_request("标题为空或带首尾空白"));
    }
    if title.chars().count() > TITLE_LIMIT {
        return Err(Error::bad_request(format!(
            "标题不能超过 {TITLE_LIMIT} 个字"
        )));
    }
    Ok(())
}

/// LRR 的来源标签常常不带协议（`exhentai.org/g/…`），所以不要求是完整的 URL。
fn valid_source(url: &str) -> Result<()> {
    if url.trim() != url || url.is_empty() || url.contains(char::is_whitespace) {
        return Err(Error::bad_request("来源不能为空或含空白"));
    }
    if url.chars().count() > SOURCE_LIMIT {
        return Err(Error::bad_request(format!(
            "来源不能超过 {SOURCE_LIMIT} 个字"
        )));
    }
    Ok(())
}

/// LRR 用逗号分隔标签，网页端的编辑框也是，所以标签里不能有逗号，也不能有换行。
fn valid_tags(tags: &[Tag]) -> Result<()> {
    if tags.len() > BATCH_LIMIT {
        return Err(Error::bad_request(format!("一次最多 {BATCH_LIMIT} 个标签")));
    }
    for tag in tags {
        let shown = if tag.namespace.is_empty() {
            tag.value.clone()
        } else {
            format!("{}:{}", tag.namespace, tag.value)
        };
        if tag.value.trim().is_empty() || tag.value.trim() != tag.value {
            return Err(Error::bad_request(format!(
                "标签“{shown}”的值为空或带首尾空白"
            )));
        }
        if tag.namespace.trim() != tag.namespace {
            return Err(Error::bad_request(format!(
                "标签“{shown}”的命名空间带首尾空白"
            )));
        }
        if shown.contains(',') {
            return Err(Error::bad_request(format!("标签“{shown}”里不能有逗号")));
        }
        // 网页端的编辑框是一段文本，标签里的换行等控制字符没法原样改回去。
        if shown.chars().any(char::is_control) {
            return Err(Error::bad_request(format!(
                "标签“{shown}”里有换行或控制字符"
            )));
        }
        if tag.namespace.contains(':') {
            return Err(Error::bad_request(format!(
                "标签“{shown}”的命名空间里不能有冒号"
            )));
        }
        if shown.chars().count() > TAG_LIMIT {
            return Err(Error::bad_request(format!("标签不能超过 {TAG_LIMIT} 个字")));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditBody {
    title: Option<String>,
    summary: Option<String>,
    /// 空字符串清除来源。
    source_url: Option<String>,
    tags: Option<Vec<String>>,
    add_tags: Option<Vec<String>>,
    remove_tags: Option<Vec<String>>,
}

impl EditBody {
    fn into_edit(self) -> Result<ArchiveEdit> {
        let tags = match (self.tags, self.add_tags, self.remove_tags) {
            (None, None, None) => TagEdit::Keep,
            (Some(tags), None, None) => TagEdit::Replace(parse_tags(&tags)?),
            (None, add, remove) => TagEdit::Merge {
                add: parse_tags(&add.unwrap_or_default())?,
                remove: parse_tags(&remove.unwrap_or_default())?,
            },
            (Some(_), _, _) => {
                return Err(Error::bad_request(
                    "tags 整体替换标签，不能和 add_tags、remove_tags 同时给出",
                ));
            }
        };
        Ok(ArchiveEdit {
            title: self.title.map(|title| title.trim().to_owned()),
            summary: self.summary,
            source_url: self
                .source_url
                .map(|url| Some(url.trim().to_owned()).filter(|url| !url.is_empty())),
            tags,
        })
    }
}

/// 返回改完的详情，格式同 `GET /api/archives/{id}`。
async fn edit(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
    body: std::result::Result<Json<EditBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    let Json(body) = body.map_err(Error::bad_request)?;
    let edit = body.into_edit()?;
    app.blocking(move |app| {
        app.edit_archive(id, &edit)?;
        crate::api::detail_json(&app, id).map(Json)
    })
    .await
}

async fn delete(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<StatusCode> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| match app.delete_missing(&[id])?[..] {
        [Deletion::Deleted] => Ok(StatusCode::NO_CONTENT),
        [Deletion::NotFound] => Err(Error::not_found()),
        _ => Err(conflict(
            "文件还在内容目录里，删掉记录后下次扫描会把它当作新书重新入库",
        )),
    })
    .await
}

#[derive(Deserialize)]
struct DeleteMissingBody {
    ids: Vec<i64>,
}

async fn delete_missing(
    State(app): State<App>,
    body: std::result::Result<Json<DeleteMissingBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    if body.ids.len() > BATCH_LIMIT {
        return Err(Error::bad_request(format!(
            "一次最多删除 {BATCH_LIMIT} 条记录"
        )));
    }
    app.blocking(move |app| {
        let outcomes = app.delete_missing(&body.ids)?;
        let pick = |wanted: Deletion| -> Vec<i64> {
            body.ids
                .iter()
                .zip(&outcomes)
                .filter(|(_, outcome)| **outcome == wanted)
                .map(|(&id, _)| id)
                .collect()
        };
        Ok(Json(json!({
            "deleted": pick(Deletion::Deleted),
            "present": pick(Deletion::FilePresent),
            "not_found": pick(Deletion::NotFound),
        })))
    })
    .await
}

#[derive(Deserialize)]
struct RenameBody {
    from: String,
    to: String,
}

/// 改名很少用，提交后立即重建快照，控制台的标签统计和搜索马上是新的。
async fn rename_tag(
    State(app): State<App>,
    body: std::result::Result<Json<RenameBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    let from = parse_tag(&body.from)?;
    let to = parse_tag(&body.to)?;
    app.blocking(move |app| {
        let archives = app.write(|writer| Ok(writer.rename_tag(&from, &to)?))?;
        tracing::info!(from = %body.from, to = %body.to, archives, "标签改名");
        if let Err(error) = app.refresh_blocking() {
            tracing::error!(%error, "改名后重建快照失败，稍后重试");
            app.listing_changed();
        }
        Ok(Json(json!({ "archives": archives })))
    })
    .await
}

#[cfg(test)]
mod tests {
    use crate::{App, Config, Error};

    #[tokio::test]
    async fn records_are_not_deleted_while_a_scan_holds_the_lock() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            content_dir: root.path().join("content"),
            data_dir: root.path().join("data"),
            web_dir: root.path().join("web"),
            listen: "127.0.0.1:0".parse().unwrap(),
            scan_hidden: false,
            detect_webtoons: true,
        };
        std::fs::create_dir(&config.content_dir).unwrap();
        let app = App::open(config).await.unwrap();
        let _scan = app.0.scan_lock.lock().unwrap();
        let result = app.delete_missing(&[1]);
        assert!(
            matches!(&result, Err(Error::Http { status, .. }) if status.as_u16() == 409),
            "{result:?}"
        );
    }
}
