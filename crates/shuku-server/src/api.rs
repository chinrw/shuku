use crate::{App, Error, Result, db};
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use shuku_index::Sort;
use shuku_store::{Archive, Progress, Tag};
use std::collections::BTreeMap;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Http { status, .. } => *status,
            Self::Store(shuku_store::Error::ArchiveNotFound(_)) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "请求失败");
        }
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}

impl App {
    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/search", get(search))
            .route("/api/facets", get(facets))
            .route("/api/tags", get(suggest_tags))
            .route("/api/recent", get(recent))
            .route("/api/archives/{id}", get(detail))
            .route("/api/archives/{id}/pages", get(pages))
            .route("/api/archives/{id}/pages/{n}", get(crate::images::page))
            .route(
                "/api/archives/{id}/thumbnail",
                get(crate::images::thumbnail),
            )
            .route("/api/archives/{id}/progress", put(write_progress))
            .route("/api", any(|| async { Error::not_found() }))
            .route("/api/", any(|| async { Error::not_found() }))
            .route("/api/{*path}", any(|| async { Error::not_found() }))
            .fallback_service(
                tower_http::services::ServeDir::new(&self.0.config.web_dir).fallback(
                    tower_http::services::ServeFile::new(self.0.config.web_dir.join("index.html")),
                ),
            )
            .method_not_allowed_fallback(|| async {
                Error::Http {
                    status: StatusCode::METHOD_NOT_ALLOWED,
                    message: "不支持的请求方法".into(),
                }
            })
            .with_state(self.clone())
    }
}

#[derive(Deserialize)]
struct ProgressInput {
    page: u32,
    client_time: i64,
}

async fn write_progress(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
    body: std::result::Result<Json<ProgressInput>, JsonRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    let Json(body) = body.map_err(Error::bad_request)?;
    let (reply, outcome) = tokio::sync::oneshot::channel();
    app.blocking(move |app| {
        app.enqueue(db::Request::Progress(db::ProgressRequest {
            id,
            page: body.page,
            client_time: body.client_time,
            reply,
        }))
    })
    .await?;
    let outcome = outcome.await.map_err(|_| db::queue_stopped())??;
    Ok(Json(
        json!({"accepted": outcome.accepted, "progress": progress(outcome.progress.as_ref())}),
    ))
}

#[derive(Default, Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    sort: Option<String>,
    after: Option<i64>,
    limit: Option<usize>,
}

async fn search(
    State(app): State<App>,
    params: std::result::Result<Query<SearchParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(Error::bad_request("limit 必须在 1 到 200 之间"));
    }
    let sort = match params.sort.as_deref() {
        None | Some("title") => Sort::Title,
        Some("date") => Sort::DateAddedNewest,
        _ => return Err(Error::bad_request("sort 必须是 title 或 date")),
    };
    app.blocking(move |app| {
        let mut parsed = shuku_index::parse_query(&params.q).map_err(Error::bad_request)?;
        parsed.query.sort = sort;
        parsed.query.after = params.after;
        parsed.query.limit = limit;
        let snapshot = app.snapshot();
        let result = snapshot.index.search(&parsed.query).map_err(Error::bad_request)?;
        // 结果和列表元数据来自同一个快照，结果里的每本都查得到。
        let items: Vec<_> = result.ids.iter().filter_map(|id| snapshot.item(*id)).map(|item| list_item(&item)).collect();
        Ok(Json(json!({"total": result.total, "next_after": result.next_after, "degraded": parsed.degraded, "items": items})))
    }).await
}

#[derive(Default, Deserialize)]
struct RecentParams {
    limit: Option<usize>,
}

/// 最近读过的归档，按翻页时间从新到旧。
async fn recent(
    State(app): State<App>,
    params: std::result::Result<Query<RecentParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let limit = tag_limit(params.limit, 20)?;
    app.blocking(move |app| {
        let snapshot = app.snapshot();
        let items: Vec<_> = snapshot
            .recently_read(limit)
            .iter()
            .map(list_item)
            .collect();
        Ok(Json(json!({ "items": items })))
    })
    .await
}

/// 搜索结果和最近阅读里的一项。
fn list_item(item: &crate::snapshot::Item<'_>) -> Value {
    json!({
        "id": item.archive.id, "title": item.archive.title,
        "page_count": item.archive.page_count, "is_new": item.archive.is_new,
        "progress": progress(item.progress), "tags": tag_groups(item.tags()),
        "cover_url": cover_url(item.archive.id, &item.archive.quick_hash),
        "date_added": item.archive.date_added, "source_url": item.archive.source_url,
    })
}

#[derive(Default, Deserialize)]
struct FacetParams {
    #[serde(default)]
    q: String,
    limit: Option<usize>,
}

async fn facets(
    State(app): State<App>,
    params: std::result::Result<Query<FacetParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let limit = tag_limit(params.limit, 20)?;
    app.blocking(move |app| {
        let parsed = shuku_index::parse_query(&params.q).map_err(Error::bad_request)?;
        Ok(Json(tag_counts(
            app.snapshot().index.facets(&parsed.query, limit),
        )))
    })
    .await
}

#[derive(Deserialize)]
struct SuggestParams {
    prefix: String,
    limit: Option<usize>,
}

async fn suggest_tags(
    State(app): State<App>,
    params: std::result::Result<Query<SuggestParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let limit = tag_limit(params.limit, 10)?;
    app.blocking(move |app| {
        Ok(Json(tag_counts(
            app.snapshot().index.suggest_tags(&params.prefix, limit),
        )))
    })
    .await
}

/// 标签统计、标签补全和最近阅读共用的 `limit` 检查。
fn tag_limit(limit: Option<usize>, default: usize) -> Result<usize> {
    let limit = limit.unwrap_or(default);
    if !(1..=100).contains(&limit) {
        return Err(Error::bad_request("limit 必须在 1 到 100 之间"));
    }
    Ok(limit)
}

fn tag_counts(tags: Vec<shuku_index::TagCount>) -> Value {
    let tags: Vec<_> = tags
        .into_iter()
        .map(|tag| json!({"tag": tag.tag, "count": tag.count}))
        .collect();
    json!({"tags": tags})
}

async fn detail(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let archive = require_archive(&*app.reader()?, id)?;
        let tags = tag_groups(&archive.tags);
        Ok(Json(json!({
            "id": archive.id, "title": archive.metadata.title,
            "summary": archive.metadata.summary, "page_count": archive.page_count,
            "is_new": archive.metadata.is_new, "date_added": archive.metadata.date_added,
            "source_url": archive.metadata.source_url, "file_size": archive.metadata.file_size,
            "progress": progress(archive.progress.as_ref()), "tags": tags,
            "cover_url": cover_url(id, &archive.metadata.quick_hash),
        })))
    })
    .await
}

async fn pages(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let (archive, pages) = app
            .reader()?
            .archive_with_pages(id)?
            .ok_or_else(Error::not_found)?;
        let pages: Vec<_> = pages.into_iter().enumerate().map(|(n, page)| json!({
            "width": page.width, "height": page.height,
            "url": format!("/api/archives/{id}/pages/{n}?v={}", archive.metadata.quick_hash),
        })).collect();
        Ok(Json(json!({"pages": pages})))
    })
    .await
}

pub(crate) fn require_archive(reader: &shuku_store::Reader, id: i64) -> Result<Archive> {
    reader.archive(id)?.ok_or_else(Error::not_found)
}

/// 按命名空间分组，组按命名空间名排序，组内保持存储层给出的顺序。
fn tag_groups<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> Vec<Value> {
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for tag in tags {
        groups.entry(&tag.namespace).or_default().push(&tag.value);
    }
    groups
        .into_iter()
        .map(|(namespace, values)| json!({"namespace": namespace, "values": values}))
        .collect()
}

fn progress(value: Option<&Progress>) -> Value {
    match value {
        Some(value) => json!({"page": value.page, "client_time": value.client_time}),
        None => Value::Null,
    }
}

fn cover_url(id: i64, hash: &str) -> String {
    format!("/api/archives/{id}/thumbnail?v={hash}")
}
