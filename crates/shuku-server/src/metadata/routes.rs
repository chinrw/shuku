//! 元数据的 HTTP 接口。`/api/archives/{id}/metadata` 对一本书运行一个来源；
//! `/api/metadata/*` 是给人和 agent 处理队列和候选用的；设置和状态在 `/api/admin/*` 下。

use super::{Provider, apply_in, metadata_json, settings_json};
use crate::{App, Error, Result};
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use shuku_metadata::Metadata;
use std::time::{SystemTime, UNIX_EPOCH};

/// 一次最多放进队列的本数。
const QUEUE_LIMIT: usize = 1000;
/// 列表默认和最多给的条数。
const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 500;

impl App {
    pub(crate) fn metadata_router(&self) -> Router<App> {
        Router::new()
            .route("/api/archives/{id}/metadata", post(run))
            .route(
                "/api/admin/settings/metadata",
                get(get_settings).put(put_settings),
            )
            .route("/api/admin/metadata", get(status))
            .route("/api/admin/metadata/resume", post(resume))
            .route(
                "/api/metadata/queue",
                get(list_queue).post(add_to_queue).delete(clear_queue),
            )
            .route("/api/metadata/candidates", get(list_candidates))
            .route(
                "/api/metadata/candidates/{id}/{provider}",
                get(get_candidate).put(put_candidate).delete(reject),
            )
            .route(
                "/api/metadata/candidates/{id}/{provider}/approve",
                post(approve),
            )
            // 兼容第一版控制台清空 E-Hentai 队列的地址。
            .route("/api/admin/metadata/queue", delete(clear_queue))
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }

    /// 把查到的结果存成候选，阻塞调用。
    pub(super) fn store_candidate(
        &self,
        id: i64,
        provider: Provider,
        found: &Metadata,
    ) -> Result<()> {
        let json = serde_json::to_string(found).map_err(|error| Error::Write(error.to_string()))?;
        self.write(|writer| {
            // 联网期间书可能被删了，这只是这一本查不成，不是存储出了问题。
            writer.archive(id)?.ok_or_else(Error::not_found)?;
            Ok(writer.save_metadata_candidate(id, provider.as_str(), &json, now_ms())?)
        })
    }

    /// 批准一条候选：删掉它并写入（`found` 不为空时写它，例如 agent 译过的），在一个事务里。
    fn approve_candidate(
        &self,
        id: i64,
        provider: Provider,
        found: Option<Metadata>,
    ) -> Result<Metadata> {
        let applied = self.write(|writer| {
            let candidate = writer
                .take_metadata_candidate(id, provider.as_str())?
                .ok_or_else(Error::not_found)?;
            let metadata = match found {
                Some(found) => found,
                None => parse_found(&candidate.found)?,
            };
            apply_in(writer, id, metadata)
        })?;
        self.listing_changed();
        Ok(applied)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn parse_found(json: &str) -> Result<Metadata> {
    serde_json::from_str(json).map_err(|error| Error::Write(format!("候选内容损坏：{error}")))
}

/// 人或 agent 改过的候选先按写入的规则校验，免得批准时才发现写不进去。
fn validated(found: Metadata) -> Result<Metadata> {
    super::edit_from(found.clone(), false, false).validate()?;
    Ok(found)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunBody {
    provider: Provider,
    /// 只对在线来源有效：直接用这个画廊或条目，不搜索。
    url: Option<String>,
    /// 为假时只返回查到的内容，不写入。默认写入。
    #[serde(default = "yes")]
    apply: bool,
}

fn yes() -> bool {
    true
}

/// 对一本书运行一个元数据功能，返回写入（或预览时将要写入）的内容。
async fn run(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
    body: std::result::Result<Json<RunBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    let Json(body) = body.map_err(Error::bad_request)?;
    let url = body
        .url
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty());
    let provider = body.provider;
    let found = match url {
        _ if provider.is_network() => app.run_network(provider, id, url).await?,
        Some(_) => return Err(Error::bad_request("只有在线来源能指定地址")),
        None => app.blocking(move |app| app.run_local(id, provider)).await?,
    };
    let apply = body.apply;
    let applied = app
        .blocking(move |app| {
            if apply {
                app.apply_found(id, found)
            } else {
                app.preview_found(id, found)
            }
        })
        .await?;
    Ok(Json(metadata_json(&applied)))
}

async fn get_settings(State(app): State<App>) -> Result<Json<Value>> {
    Ok(Json(settings_json(&app.metadata_settings())?))
}

async fn put_settings(
    State(app): State<App>,
    body: std::result::Result<Json<Value>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(patch) = body.map_err(Error::bad_request)?;
    let settings = app
        .blocking(move |app| app.save_metadata_settings(&patch))
        .await?;
    Ok(Json(settings_json(&settings)?))
}

/// 每个在线来源的队列、候选和运行状态，以及翻译数据库的状态。
async fn status(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let reader = app.reader()?;
        let queued = reader.metadata_queue_counts()?;
        let candidates = reader.metadata_candidate_counts()?;
        drop(reader);
        let count = |counts: &[(String, u64)], provider: Provider| {
            counts
                .iter()
                .find(|(name, _)| name == provider.as_str())
                .map_or(0, |(_, count)| *count)
        };
        let settings = app.metadata_settings();
        let mut providers = serde_json::Map::new();
        for provider in Provider::NETWORK {
            let status = app.queue_status(provider, |status| status.clone());
            providers.insert(
                provider.as_str().into(),
                json!({
                    "auto": settings.auto(provider), "review": settings.review(provider),
                    "queued": count(&queued, provider), "candidates": count(&candidates, provider),
                    "current": status.current, "done": status.done, "failed": status.failed,
                    "last_error": status.last_error, "paused": status.paused,
                }),
            );
        }
        let translation = if settings.translation.enabled {
            match app.translations() {
                Ok(translations) => json!({"entries": translations.len(), "error": null}),
                Err(error) => json!({"entries": null, "error": error.to_string()}),
            }
        } else {
            json!({"entries": null, "error": null})
        };
        Ok(Json(json!({
            "providers": providers,
            "translation": {
                "enabled": settings.translation.enabled, "path": app.translation_path(),
                "entries": translation["entries"], "error": translation["error"],
            },
            "cookies": settings.ehentai.cookies.is_set(),
        })))
    })
    .await
}

#[derive(Deserialize)]
struct ProviderParam {
    provider: Option<String>,
}

fn provider_param(provider: Option<&str>) -> Result<Option<Provider>> {
    provider.map(Provider::network).transpose()
}

async fn resume(
    State(app): State<App>,
    params: std::result::Result<Query<ProviderParam>, QueryRejection>,
) -> Result<StatusCode> {
    let Query(params) = params.map_err(Error::bad_request)?;
    app.resume_metadata(provider_param(params.provider.as_deref())?);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PageParams {
    provider: Option<String>,
    /// 上一页最后一条的归档 ID 和来源。
    after: Option<i64>,
    after_provider: Option<String>,
    limit: Option<usize>,
}

impl PageParams {
    fn limit(&self) -> Result<usize> {
        let limit = self.limit.unwrap_or(DEFAULT_PAGE);
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(Error::bad_request(format!(
                "limit 必须在 1 到 {MAX_PAGE} 之间"
            )));
        }
        Ok(limit)
    }

    /// 只给 `after` 时从这本的第一个来源之后接着，也就是跳过这本的全部来源。
    fn after(&self) -> Option<(i64, &str)> {
        self.after
            .map(|id| (id, self.after_provider.as_deref().unwrap_or("\u{10ffff}")))
    }
}

/// 归档在快照里的标题，快照还没刷新到它时为 `null`。
fn snapshot_title(snapshot: &crate::snapshot::Snapshot, id: i64) -> Value {
    snapshot
        .item(id)
        .map_or(Value::Null, |item| item.archive.title.clone().into())
}

/// 队列里的书，按归档 ID、来源排序，用 `after` 和 `after_provider` 翻页。
async fn list_queue(
    State(app): State<App>,
    params: std::result::Result<Query<PageParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let provider = provider_param(params.provider.as_deref())?;
    let limit = params.limit()?;
    app.blocking(move |app| {
        let reader = app.reader()?;
        let rows =
            reader.metadata_queue(provider.map(Provider::as_str), params.after(), limit + 1)?;
        let counts = reader.metadata_queue_counts()?;
        drop(reader);
        let more = rows.len() > limit;
        let snapshot = app.snapshot();
        let items: Vec<_> = rows
            .iter()
            .take(limit)
            .map(|(id, provider)| {
                json!({"id": id, "provider": provider, "title": snapshot_title(&snapshot, *id)})
            })
            .collect();
        let counts: serde_json::Map<_, _> = counts
            .into_iter()
            .map(|(provider, count)| (provider, count.into()))
            .collect();
        Ok(Json(
            json!({"counts": counts, "items": items, "more": more}),
        ))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueBody {
    provider: String,
    ids: Vec<i64>,
}

/// 把书放进一个在线来源的队列，已经在里面的不变。返回放进去的和不存在的书。
async fn add_to_queue(
    State(app): State<App>,
    body: std::result::Result<Json<QueueBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    let provider = Provider::network(&body.provider)?;
    if body.ids.is_empty() || body.ids.len() > QUEUE_LIMIT {
        return Err(Error::bad_request(format!(
            "ids 要有 1 到 {QUEUE_LIMIT} 本"
        )));
    }
    let (queued, missing) = app
        .blocking(move |app| {
            app.write(|writer| {
                let (mut queued, mut missing) = (Vec::new(), Vec::new());
                for id in body.ids {
                    if writer.archive(id)?.is_none() {
                        missing.push(id);
                        continue;
                    }
                    writer.queue_metadata(id, provider.as_str())?;
                    queued.push(id);
                }
                Ok((queued, missing))
            })
        })
        .await?;
    if !queued.is_empty() {
        app.metadata_queued();
    }
    Ok(Json(json!({"queued": queued, "not_found": missing})))
}

async fn clear_queue(
    State(app): State<App>,
    params: std::result::Result<Query<ProviderParam>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let provider = provider_param(params.provider.as_deref())?;
    app.blocking(move |app| {
        let removed =
            app.write(|writer| Ok(writer.clear_metadata_queue(provider.map(Provider::as_str))?))?;
        Ok(Json(json!({ "removed": removed })))
    })
    .await
}

/// 一条候选连同书现在的标题、简介、来源和标签，agent 拿它对比、翻译。
fn candidate_json(
    reader: &shuku_store::Reader,
    candidate: &shuku_store::MetadataCandidate,
) -> Result<Value> {
    let found = parse_found(&candidate.found)?;
    let archive = reader.archive(candidate.archive_id)?;
    let current = archive.map(|archive| {
        let tags: Vec<_> = archive
            .tags
            .iter()
            .map(|tag| json!({"namespace": tag.namespace, "value": tag.value}))
            .collect();
        json!({
            "title": archive.metadata.title, "summary": archive.metadata.summary,
            "source_url": archive.metadata.source_url, "tags": tags,
        })
    });
    Ok(json!({
        "id": candidate.archive_id, "provider": candidate.provider,
        "fetched_at": candidate.fetched_at, "archive": current, "found": metadata_json(&found),
    }))
}

async fn list_candidates(
    State(app): State<App>,
    params: std::result::Result<Query<PageParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let provider = provider_param(params.provider.as_deref())?;
    let limit = params.limit()?;
    app.blocking(move |app| {
        let reader = app.reader()?;
        let rows = reader.metadata_candidates(
            provider.map(Provider::as_str),
            params.after(),
            limit + 1,
        )?;
        let more = rows.len() > limit;
        let items = rows
            .iter()
            .take(limit)
            .map(|candidate| candidate_json(&reader, candidate))
            .collect::<Result<Vec<_>>>()?;
        Ok(Json(json!({"items": items, "more": more})))
    })
    .await
}

fn candidate_path(
    path: std::result::Result<Path<(i64, String)>, PathRejection>,
) -> Result<(i64, Provider)> {
    let Path((id, provider)) = path.map_err(Error::bad_request)?;
    Ok((id, Provider::network(&provider)?))
}

async fn get_candidate(
    State(app): State<App>,
    path: std::result::Result<Path<(i64, String)>, PathRejection>,
) -> Result<Json<Value>> {
    let (id, provider) = candidate_path(path)?;
    app.blocking(move |app| {
        let reader = app.reader()?;
        let candidate = reader
            .metadata_candidate(id, provider.as_str())?
            .ok_or_else(Error::not_found)?;
        Ok(Json(candidate_json(&reader, &candidate)?))
    })
    .await
}

/// 整个替换一条候选的内容，例如 agent 把标签译成中文后写回。查到的时间不变。
async fn put_candidate(
    State(app): State<App>,
    path: std::result::Result<Path<(i64, String)>, PathRejection>,
    body: std::result::Result<Json<Metadata>, JsonRejection>,
) -> Result<StatusCode> {
    let (id, provider) = candidate_path(path)?;
    let Json(found) = body.map_err(Error::bad_request)?;
    let json = serde_json::to_string(&validated(found)?)
        .map_err(|error| Error::Write(error.to_string()))?;
    let updated = app
        .blocking(move |app| {
            app.write(|writer| {
                Ok(writer.update_metadata_candidate(id, provider.as_str(), &json)?)
            })
        })
        .await?;
    if !updated {
        return Err(Error::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 批准并写入。请求体可以省略或者是 `null`，也可以是要写的内容（不改存着的候选，直接写这个）。
async fn approve(
    State(app): State<App>,
    path: std::result::Result<Path<(i64, String)>, PathRejection>,
    body: axum::body::Bytes,
) -> Result<Json<Value>> {
    let (id, provider) = candidate_path(path)?;
    let found: Option<Metadata> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        serde_json::from_slice(&body).map_err(Error::bad_request)?
    };
    let found = found.map(validated).transpose()?;
    let applied = app
        .blocking(move |app| app.approve_candidate(id, provider, found))
        .await?;
    Ok(Json(metadata_json(&applied)))
}

async fn reject(
    State(app): State<App>,
    path: std::result::Result<Path<(i64, String)>, PathRejection>,
) -> Result<StatusCode> {
    let (id, provider) = candidate_path(path)?;
    let removed = app
        .blocking(move |app| {
            app.write(|writer| Ok(writer.take_metadata_candidate(id, provider.as_str())?))
        })
        .await?;
    if removed.is_none() {
        return Err(Error::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
