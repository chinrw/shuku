//! 收藏的标签：钉在书库筛选行上、点一下就按它筛选的标签。所有设备共用，按收藏的先后
//! 存在设置表里。认证打开前和分类一样只在局域网里用，写请求要求同源。

use crate::{App, Error, Result};
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};

const KEY: &str = "saved_tags";
/// 筛选行放不下太多，也免得设置表里的这一项无限增长。
const LIMIT: usize = 100;
/// 单个标签的长度上限（字符数）。
const TAG_LIMIT: usize = 200;

impl App {
    pub(crate) fn saved_tags_router(&self) -> Router<App> {
        Router::new()
            .route("/api/saved-tags", get(list).put(add).delete(remove))
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }
}

#[derive(Deserialize)]
struct TagBody {
    tag: String,
}

async fn list(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let tags: Vec<String> = app.reader()?.setting(KEY)?.unwrap_or_default();
        Ok(Json(json!({ "tags": tags })))
    })
    .await
}

async fn add(
    State(app): State<App>,
    body: std::result::Result<Json<TagBody>, JsonRejection>,
) -> Result<Json<Value>> {
    change(app, body, true).await
}

async fn remove(
    State(app): State<App>,
    body: std::result::Result<Json<TagBody>, JsonRejection>,
) -> Result<Json<Value>> {
    change(app, body, false).await
}

/// 在同一个写事务里读出、改、写回整个列表，两台设备同时改不会丢掉一边。
async fn change(
    app: App,
    body: std::result::Result<Json<TagBody>, JsonRejection>,
    saved: bool,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    let tag = valid_tag(&body.tag)?;
    app.blocking(move |app| {
        let tags = app.write(|writer| {
            let mut tags: Vec<String> = writer.setting(KEY)?.unwrap_or_default();
            let present = tags.contains(&tag);
            if saved && !present {
                if tags.len() >= LIMIT {
                    return Err(Error::bad_request(format!("最多收藏 {LIMIT} 个标签")));
                }
                tags.push(tag);
            } else if !saved && present {
                tags.retain(|other| *other != tag);
            } else {
                return Ok(tags);
            }
            writer.set_setting(KEY, &tags)?;
            Ok(tags)
        })?;
        Ok(Json(json!({ "tags": tags })))
    })
    .await
}

/// 搜索按小写匹配标签，收藏也存小写的完整标签名，同一个标签只收藏一次。
fn valid_tag(tag: &str) -> Result<String> {
    let tag = tag.trim().to_lowercase();
    if tag.is_empty() {
        return Err(Error::bad_request("标签不能为空"));
    }
    if tag.chars().count() > TAG_LIMIT {
        return Err(Error::bad_request(format!("标签最多 {TAG_LIMIT} 个字")));
    }
    Ok(tag)
}
