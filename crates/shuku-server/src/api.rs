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
            Self::Store(
                shuku_store::Error::ArchiveNotFound(_)
                | shuku_store::Error::CategoryNotFound(_)
                | shuku_store::Error::TagNotFound(_),
            ) => StatusCode::NOT_FOUND,
            Self::Store(
                shuku_store::Error::DynamicCategory(_) | shuku_store::Error::CategoryKindChange(_),
            ) => StatusCode::BAD_REQUEST,
            Self::Metadata(error) => metadata_status(error),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "请求失败");
        }
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}

/// E-Hentai 那边的问题算作上游错误，不是服务端自己出错。
fn metadata_status(error: &shuku_metadata::Error) -> StatusCode {
    use shuku_metadata::Error;
    match error {
        Error::NotFound => StatusCode::UNPROCESSABLE_ENTITY,
        Error::GalleryUrl(_) | Error::Regex(_) => StatusCode::BAD_REQUEST,
        Error::Banned | Error::Throttled => StatusCode::SERVICE_UNAVAILABLE,
        Error::Http(_) | Error::Status(_) | Error::Api(_) | Error::EmptyPage => {
            StatusCode::BAD_GATEWAY
        }
        Error::Xml(_) | Error::XmlEncoding => StatusCode::UNPROCESSABLE_ENTITY,
        Error::Translation(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl App {
    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/search", get(search))
            .route("/api/facets", get(facets))
            .route("/api/tags", get(suggest_tags))
            .route("/api/recent", get(recent))
            .route("/api/random", get(random))
            .route("/api/series/{id}", get(series))
            .route("/api/archives/{id}/pages", get(pages))
            .route("/api/archives/{id}/pages/{n}", get(crate::images::page))
            .route(
                "/api/archives/{id}/pages/{n}/thumbnail",
                get(crate::images::page_thumbnail),
            )
            .route(
                "/api/archives/{id}/thumbnail",
                get(crate::images::thumbnail),
            )
            .route("/api/archives/{id}/file", get(crate::download::file))
            .route("/api/archives/{id}/progress", put(write_progress))
            .merge(self.edit_router())
            .merge(self.categories_router())
            .merge(self.metadata_router())
            .merge(self.batch_router())
            .merge(self.admin_router())
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
            .layer(axum::middleware::from_fn_with_state(
                self.clone(),
                crate::hosts::check_host,
            ))
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
    category: Option<i64>,
    #[serde(default)]
    filter: String,
    sort: Option<String>,
    order: Option<String>,
    after: Option<i64>,
    limit: Option<usize>,
    /// `series` 时同一系列只返回卷号最小的一卷，见 [`shuku_index::Query::collapse_series`]。
    group: Option<String>,
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
    let (sort, reverse) = parse_sort(params.sort.as_deref(), params.order.as_deref())?;
    let filters = parse_filters(&params.filter)?;
    let collapse_series = match params.group.as_deref() {
        None | Some("") => false,
        Some("series") => true,
        Some(group) => return Err(Error::bad_request(format!("不支持的 group：{group}"))),
    };
    app.blocking(move |app| {
        let snapshot = app.snapshot();
        let (mut query, degraded) = build_query(&snapshot, &params.q, params.category, &filters)?;
        query.sort = sort;
        query.reverse = reverse;
        query.after = params.after;
        query.limit = limit;
        query.collapse_series = collapse_series;
        let result = snapshot.index.search(&query).map_err(Error::bad_request)?;
        // 结果和列表元数据来自同一个快照，结果里的每本都查得到。
        let items: Vec<_> = result.ids.iter().filter_map(|id| snapshot.item(*id)).map(|item| list_item(&item)).collect();
        Ok(Json(json!({"total": result.total, "next_after": result.next_after, "degraded": degraded, "items": items})))
    }).await
}

/// `sort` 是 `title`、`date`、`pages` 或 `ns:<命名空间>`；`order` 是 `asc` 或 `desc`，
/// 不给时按各自的默认方向：加入时间默认新的在前，其余默认从小到大。
fn parse_sort(sort: Option<&str>, order: Option<&str>) -> Result<(Sort, bool)> {
    let sort = match sort {
        None | Some("title") => Sort::Title,
        Some("date") => Sort::DateAddedNewest,
        Some("pages") => Sort::Pages,
        Some(other) => match other.strip_prefix("ns:") {
            Some(namespace) if !namespace.trim().is_empty() => {
                Sort::Namespace(namespace.trim().to_owned())
            }
            _ => {
                return Err(Error::bad_request(
                    "sort 必须是 title、date、pages 或 ns:<命名空间>",
                ));
            }
        },
    };
    let descending_by_default = sort == Sort::DateAddedNewest;
    let reverse = match order {
        None => false,
        Some("asc") => descending_by_default,
        Some("desc") => !descending_by_default,
        Some(_) => return Err(Error::bad_request("order 必须是 asc 或 desc")),
    };
    Ok((sort, reverse))
}

#[derive(Default, Deserialize)]
struct RandomParams {
    #[serde(default)]
    q: String,
    category: Option<i64>,
    #[serde(default)]
    filter: String,
}

/// 搜索结果里随机的一本，没有结果时返回 404。
async fn random(
    State(app): State<App>,
    params: std::result::Result<Query<RandomParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let filters = parse_filters(&params.filter)?;
    // 不为一个随机数引入 rand：标准库每个 RandomState 的种子都不同。
    let seed = std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), 0);
    app.blocking(move |app| {
        let snapshot = app.snapshot();
        let (query, _) = build_query(&snapshot, &params.q, params.category, &filters)?;
        let id = snapshot
            .index
            .random(&query, seed)
            .ok_or_else(Error::not_found)?;
        Ok(Json(json!({ "id": id })))
    })
    .await
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

/// 把搜索限制在一个分类里：静态分类按成员，动态分类再加上它的搜索语句。
/// 返回动态分类的语句是否被降级（见搜索接口的 `degraded`）。
fn restrict_to_category(
    snapshot: &crate::snapshot::Snapshot,
    query: &mut shuku_index::Query,
    id: i64,
) -> Result<bool> {
    let category = snapshot
        .categories()
        .iter()
        .find(|category| category.id == id)
        .ok_or_else(|| Error::bad_request(format!("分类 {id} 不存在")))?;
    if category.search.is_empty() {
        query
            .include
            .push(shuku_index::Term::Archives(category.archives.clone()));
        return Ok(false);
    }
    // 分类的语句来自 LRR，写法 Shuku 还不支持时当作这个分类是空的，不让整个请求失败。
    match shuku_index::parse_query(&category.search) {
        Ok(parsed) => {
            query.include.extend(parsed.query.include);
            query.exclude.extend(parsed.query.exclude);
            for condition in &parsed.read {
                restrict_by_read(snapshot, query, *condition);
            }
            Ok(parsed.degraded)
        }
        Err(error) => {
            tracing::warn!(id, search = %category.search, %error, "分类的搜索语句无法解析");
            query.include.push(shuku_index::Term::Archives(Vec::new()));
            Ok(false)
        }
    }
}

/// 书库页的快捷筛选，搜索和标签统计的 `filter` 参数，逗号分隔，同时生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filter {
    /// 新入库、还没读过。
    New,
    Untagged,
    Unread,
    /// 有进度、还没读到最后一页。
    Reading,
    /// 读到了最后一页。
    Finished,
}

pub(crate) fn parse_filters(raw: &str) -> Result<Vec<Filter>> {
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| match name {
            "new" => Ok(Filter::New),
            "untagged" => Ok(Filter::Untagged),
            "unread" => Ok(Filter::Unread),
            "reading" => Ok(Filter::Reading),
            "finished" => Ok(Filter::Finished),
            _ => Err(Error::bad_request(format!(
                "filter 只能是 new、untagged、unread、reading、finished：{name}"
            ))),
        })
        .collect()
}

/// 把搜索语句、分类和快捷筛选合成一个查询，返回查询和语句是否被降级。
pub(crate) fn build_query(
    snapshot: &crate::snapshot::Snapshot,
    q: &str,
    category: Option<i64>,
    filters: &[Filter],
) -> Result<(shuku_index::Query, bool)> {
    let parsed = shuku_index::parse_query(q).map_err(Error::bad_request)?;
    let mut query = parsed.query;
    let mut degraded = parsed.degraded;
    for condition in parsed.read {
        restrict_by_read(snapshot, &mut query, condition);
    }
    if let Some(id) = category {
        degraded |= restrict_to_category(snapshot, &mut query, id)?;
    }
    for filter in filters {
        restrict_by_filter(snapshot, &mut query, *filter);
    }
    Ok((query, degraded))
}

/// `read:` 条件按快照里的进度换成归档列表。已读页数是读到的页码，从 1 开始，没读过是 0，
/// 和 LRR 一样。读过的书只有几百本，所以只列出有进度的归档：没读过的书满足条件时，
/// 改成排除不满足条件的那些有进度的归档，不用列出几十万本没读过的书。
fn restrict_by_read(
    snapshot: &crate::snapshot::Snapshot,
    query: &mut shuku_index::Query,
    condition: shuku_index::ReadCondition,
) {
    let unread_matches = condition.comparison.matches(0);
    let ids: Vec<i64> = snapshot
        .progress()
        .iter()
        .filter(|(_, progress)| condition.comparison.matches(progress.page + 1) != unread_matches)
        .map(|(id, _)| *id)
        .collect();
    let term = shuku_index::Term::Archives(ids);
    // 没读过的书满足条件时，`ids` 是不满足条件的有进度归档。
    if unread_matches == condition.excluded {
        query.include.push(term);
    } else {
        query.exclude.push(term);
    }
}

fn restrict_by_filter(
    snapshot: &crate::snapshot::Snapshot,
    query: &mut shuku_index::Query,
    filter: Filter,
) {
    use shuku_index::Term;
    let read_ids = |finished: Option<bool>| -> Vec<i64> {
        snapshot
            .progress()
            .iter()
            .filter(|(id, progress)| {
                finished.is_none_or(|finished| {
                    snapshot.item(**id).is_some_and(|item| {
                        (progress.page + 1 >= item.archive.page_count) == finished
                    })
                })
            })
            .map(|(id, _)| *id)
            .collect()
    };
    match filter {
        Filter::New => {
            query.include.push(Term::New);
            query.exclude.push(Term::Archives(read_ids(None)));
        }
        Filter::Untagged => query.include.push(Term::Untagged),
        Filter::Unread => query.exclude.push(Term::Archives(read_ids(None))),
        Filter::Reading => query.include.push(Term::Archives(read_ids(Some(false)))),
        Filter::Finished => query.include.push(Term::Archives(read_ids(Some(true)))),
    }
}

/// 书库页的分类按钮。`count` 是分类里在快照中的本数。
pub(crate) async fn categories(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let snapshot = app.snapshot();
        let mut categories = Vec::new();
        for category in snapshot.categories() {
            let mut query = shuku_index::Query {
                limit: 1,
                ..Default::default()
            };
            restrict_to_category(&snapshot, &mut query, category.id)?;
            let count = snapshot
                .index
                .search(&query)
                .map_err(Error::bad_request)?
                .total;
            categories.push(json!({
                "id": category.id, "name": category.name, "pinned": category.pinned,
                "dynamic": !category.search.is_empty(), "count": count,
            }));
        }
        Ok(Json(json!({ "categories": categories })))
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
        "series": item.series.zip(item.archive.series).map(|(series, (_, volume))| json!({
            "id": series.id, "name": series.name, "volume": volume,
            "volumes": series.volumes.len(),
        })),
    })
}

/// 一个系列和它的各卷，按卷号排列。只有一卷的系列不算系列，返回 404。
async fn series(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let snapshot = app.snapshot();
        let series = snapshot.series(id).ok_or_else(|| Error::Http {
            status: StatusCode::NOT_FOUND,
            message: format!("系列 {id} 不存在"),
        })?;
        let items: Vec<_> = series
            .volumes
            .iter()
            .filter_map(|id| snapshot.item(*id))
            .map(|item| list_item(&item))
            .collect();
        Ok(Json(
            json!({"id": series.id, "name": series.name, "items": items}),
        ))
    })
    .await
}

#[derive(Default, Deserialize)]
struct FacetParams {
    #[serde(default)]
    q: String,
    category: Option<i64>,
    #[serde(default)]
    filter: String,
    limit: Option<usize>,
}

async fn facets(
    State(app): State<App>,
    params: std::result::Result<Query<FacetParams>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(params) = params.map_err(Error::bad_request)?;
    let limit = tag_limit(params.limit, 20)?;
    let filters = parse_filters(&params.filter)?;
    app.blocking(move |app| {
        let snapshot = app.snapshot();
        let (query, _) = build_query(&snapshot, &params.q, params.category, &filters)?;
        Ok(Json(tag_counts(snapshot.index.facets(&query, limit))))
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

pub(crate) async fn detail(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<Json<Value>> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| detail_json(&app, id).map(Json))
        .await
}

/// 详情直接读库。`missing` 表示文件已经不在内容目录里，网页端据此提供删除记录；
/// 无法确认时（例如目录没有读权限）按还在处理。
pub(crate) fn detail_json(app: &App, id: i64) -> Result<Value> {
    let reader = app.reader()?;
    let archive = require_archive(&reader, id)?;
    let categories = reader.archive_categories(id)?;
    // 阅读器读完一卷时接着打开下一卷。
    let series = reader.archive_series(id)?.map(|series| {
        let position = series.volumes.iter().position(|&volume| volume == id);
        let neighbour = |offset: isize| {
            position
                .and_then(|position| position.checked_add_signed(offset))
                .and_then(|position| series.volumes.get(position))
        };
        json!({
            "id": series.id, "name": series.name,
            "previous": neighbour(-1), "next": neighbour(1),
        })
    });
    let tags = tag_groups(&archive.tags);
    let missing = matches!(
        app.0
            .config
            .content_dir
            .join(&archive.metadata.path)
            .try_exists(),
        Ok(false)
    );
    Ok(json!({
        "id": archive.id, "title": archive.metadata.title,
        "summary": archive.metadata.summary, "page_count": archive.page_count,
        "is_new": archive.metadata.is_new, "date_added": archive.metadata.date_added,
        "source_url": archive.metadata.source_url, "file_size": archive.metadata.file_size,
        "path": archive.metadata.path, "missing": missing,
        "progress": progress(archive.progress.as_ref()), "tags": tags,
        "categories": categories, "cover_url": cover_url(id, &archive.metadata.quick_hash),
        "series": series,
    }))
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
        let hash = &archive.metadata.quick_hash;
        let webtoon = app.settings().server.detect_webtoons
            && crate::webtoon::looks_like_webtoon(
                pages.iter().map(|page| (page.width, page.height)),
            );
        // 坏页的宽和高记为 0，客户端拿它排版会塌成一条线，按常见的竖版页面给一个占位尺寸。
        let pages: Vec<_> = pages
            .into_iter()
            .enumerate()
            .map(|(n, page)| {
                let broken = page.width == 0;
                let (width, height) = if broken {
                    BROKEN_PAGE_SIZE
                } else {
                    (page.width, page.height)
                };
                json!({
                    "width": width, "height": height, "broken": broken,
                    "url": format!("/api/archives/{id}/pages/{n}?v={hash}"),
                    "thumbnail_url": format!("/api/archives/{id}/pages/{n}/thumbnail?v={hash}"),
                })
            })
            .collect();
        Ok(Json(json!({"pages": pages, "webtoon": webtoon})))
    })
    .await
}

/// 坏页在页面清单接口里的占位尺寸，约等于 A4 的竖版比例。
const BROKEN_PAGE_SIZE: (u32, u32) = (1000, 1414);

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
