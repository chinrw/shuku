use crate::{App, Error, Result};
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
    app.blocking(move |app| {
        let mut store = app.store()?;
        let archive = require_archive(&store, id)?;
        if body.page >= archive.page_count {
            return Err(Error::bad_request("page 超出归档页数"));
        }
        let accepted = store.write_progress(id, body.page, body.client_time)?;
        let current = require_archive(&store, id)?;
        Ok(Json(
            json!({"accepted": accepted, "progress": progress(current.progress.as_ref())}),
        ))
    })
    .await
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
        let result = app.index().search(&parsed.query).map_err(Error::bad_request)?;
        let items: Vec<_> = app.store()?.list_metadata(&result.ids)?.into_iter().map(|item| json!({
            "id": item.id, "title": item.title, "page_count": item.page_count,
            "is_new": item.is_new, "progress": progress(item.progress.as_ref()),
            "cover_url": cover_url(item.id, &item.quick_hash), "tags": tag_groups(item.tags),
        })).collect();
        Ok(Json(json!({"total": result.total, "next_after": result.next_after, "degraded": parsed.degraded, "items": items})))
    }).await
}

async fn detail(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let archive = require_archive(&*app.store()?, id)?;
        let tags = tag_groups(archive.tags);
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
        let store = app.store()?;
        let archive = require_archive(&store, id)?;
        let pages: Vec<_> = store.pages(id)?.into_iter().enumerate().map(|(n, page)| json!({
            "width": page.width, "height": page.height,
            "url": format!("/api/archives/{id}/pages/{n}?v={}", archive.metadata.quick_hash),
        })).collect();
        Ok(Json(json!({"pages": pages})))
    })
    .await
}

pub(crate) fn require_archive(store: &shuku_store::Store, id: i64) -> Result<Archive> {
    store.archive(id)?.ok_or_else(Error::not_found)
}

/// 按命名空间分组，组按命名空间名排序，组内保持存储层给出的顺序。
fn tag_groups(tags: Vec<Tag>) -> Vec<Value> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tag in tags {
        groups.entry(tag.namespace).or_default().push(tag.value);
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
