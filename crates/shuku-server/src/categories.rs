//! 分类的新建、修改、删除和成员增删。分类很少，每次写入提交后立即刷新快照里的分类，
//! 加进分类后按分类筛选马上就能看到。M1 没有认证，这些写操作和进度上报一样只在局域网里用。

use crate::{App, Error, Result};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
    routing::{get, patch, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// 分类名的长度上限（字符数）。
const NAME_LIMIT: usize = 100;

impl App {
    pub(crate) fn categories_router(&self) -> Router<App> {
        Router::new()
            .route("/api/categories", get(crate::api::categories).post(create))
            .route("/api/categories/{id}", patch(update).delete(delete))
            .route(
                "/api/categories/{id}/archives/{archive}",
                put(add_member).delete(remove_member),
            )
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }

    /// 在写连接上提交 `work`，再刷新快照里的分类。刷新失败时写入已经提交，
    /// 照常返回，改由后台的延迟刷新补上。
    pub(crate) fn write_categories<T>(
        &self,
        work: impl FnOnce(&mut shuku_store::Writer<'_>) -> Result<T>,
    ) -> Result<T> {
        let value = self.write(work)?;
        if let Err(error) = self.refresh_categories() {
            tracing::error!(%error, "刷新快照里的分类失败，稍后整体重建");
            self.listing_changed();
        }
        Ok(value)
    }
}

#[derive(Deserialize)]
struct NewCategory {
    name: String,
    #[serde(default)]
    search: String,
    #[serde(default)]
    pinned: bool,
}

#[derive(Deserialize)]
struct CategoryChange {
    name: Option<String>,
    search: Option<String>,
    pinned: Option<bool>,
}

async fn create(
    State(app): State<App>,
    body: std::result::Result<Json<NewCategory>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>)> {
    let Json(body) = body.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let name = valid_name(&body.name)?;
        let search = valid_search(&body.search)?;
        let id = app
            .write_categories(|writer| Ok(writer.create_category(name, search, body.pinned)?))?;
        Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
    })
    .await
}

async fn update(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
    body: std::result::Result<Json<CategoryChange>, JsonRejection>,
) -> Result<StatusCode> {
    let Path(id) = id.map_err(Error::bad_request)?;
    let Json(body) = body.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let name = body.name.as_deref().map(valid_name).transpose()?;
        let search = body.search.as_deref().map(valid_search).transpose()?;
        app.write_categories(|writer| {
            Ok(writer.update_category(id, name, search, body.pinned)?)
        })?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

async fn delete(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<StatusCode> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        app.write_categories(|writer| Ok(writer.delete_category(id)?))?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

async fn add_member(
    State(app): State<App>,
    ids: std::result::Result<Path<(i64, i64)>, PathRejection>,
) -> Result<StatusCode> {
    set_member(app, ids, true).await
}

async fn remove_member(
    State(app): State<App>,
    ids: std::result::Result<Path<(i64, i64)>, PathRejection>,
) -> Result<StatusCode> {
    set_member(app, ids, false).await
}

async fn set_member(
    app: App,
    ids: std::result::Result<Path<(i64, i64)>, PathRejection>,
    member: bool,
) -> Result<StatusCode> {
    let Path((category, archive)) = ids.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        app.write_categories(|writer| Ok(writer.set_category_member(category, archive, member)?))?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

fn valid_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::bad_request("分类名不能为空"));
    }
    if name.chars().count() > NAME_LIMIT {
        return Err(Error::bad_request(format!(
            "分类名不能超过 {NAME_LIMIT} 个字"
        )));
    }
    Ok(name)
}

/// 动态分类的语句先按搜索的规则解析一遍，写不进 Shuku 解析不了的语句。
fn valid_search(search: &str) -> Result<&str> {
    let search = search.trim();
    if !search.is_empty() {
        shuku_index::parse_query(search).map_err(Error::bad_request)?;
    }
    Ok(search)
}
