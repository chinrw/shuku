//! 不需要令牌的服务器信息。客户端添加服务器时先读它，确认地址对、看要不要令牌。

use crate::App;
use axum::{
    Json,
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

/// 书库名没设置时的名字，和网页端左上角一致。
const DEFAULT_NAME: &str = "书库";

pub(crate) async fn info(State(app): State<App>) -> Response {
    let name = app.settings().server.name;
    let mut response = Json(json!({
        "name": if name.is_empty() { DEFAULT_NAME } else { &name },
        "version": env!("CARGO_PKG_VERSION"),
        "archives": app.snapshot().archive_count(),
        "require_token": app.auth_settings().require_token,
    }))
    .into_response();
    // 桌面和 Android 客户端的服务器列表页不在服务器的源上，要跨源读这个接口。内容不需要
    // 令牌就能看到，不带凭据，所以允许任何源。
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}
