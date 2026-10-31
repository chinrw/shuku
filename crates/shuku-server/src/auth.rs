//! 设备令牌。每台设备（浏览器、阅读程序、agent）一个，可以单独吊销。
//!
//! `/mcp` 总是要令牌：agent 能改数据，要知道是谁。`/api` 默认不要，和以前一样只在
//! 局域网里用；控制台里打开“网页端也需要令牌”以后，除了登录和查询登录状态都要。
//! 网页和图片不带令牌也照常返回，网页端收到 401 时转到登录页。
//!
//! 令牌放在 `Authorization: Bearer` 头里，网页端登录后放在 HttpOnly 的 cookie 里。
//! 库里只存令牌的 BLAKE3 摘要。每次请求查一次库（有索引，约十微秒），不在内存里缓存，
//! 所以用命令行在服务运行时新建或吊销的令牌马上生效。

use crate::{App, Error, Result};
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Request, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shuku_store::DeviceToken;
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

/// 在 setting 表里的键。
pub(crate) const KEY: &str = "auth";
/// cookie 名的前缀，后面接监听端口。浏览器的 cookie 不分端口，同一台机器上的两个实例
/// （例如另起一个管普通漫画的）用同一个名字会互相覆盖登录。
const COOKIE_PREFIX: &str = "shuku_token_";
/// 浏览器允许的 cookie 最长期限是 400 天。
const COOKIE_MAX_AGE: u64 = 400 * 24 * 3600;
/// 最近使用时间最多隔这么久写一次库，免得每张图片都写一次。
const TOUCH_INTERVAL_MS: i64 = 10 * 60 * 1000;
const NAME_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AuthSettings {
    /// 网页端和 `/api` 也需要令牌。
    pub(crate) require_token: bool,
}

/// 认证用的状态：设置和每个令牌上次写库的时间。
#[derive(Default)]
pub(crate) struct AuthState {
    settings: Mutex<AuthSettings>,
    touched: Mutex<HashMap<i64, i64>>,
}

impl AuthState {
    pub(crate) fn load(stored: shuku_store::Result<Option<AuthSettings>>) -> Self {
        let settings = stored.unwrap_or_else(|error| {
            // 读不懂时按需要令牌处理：宁可让用户用命令行建令牌登录，也不悄悄放开。
            tracing::warn!(%error, "认证设置无法解析，按需要令牌处理");
            Some(AuthSettings {
                require_token: true,
            })
        });
        Self {
            settings: Mutex::new(settings.unwrap_or_default()),
            touched: Mutex::new(HashMap::new()),
        }
    }
}

/// 通过认证的设备，放在请求的扩展里。
#[derive(Debug, Clone)]
pub(crate) struct Device {
    pub(crate) id: i64,
    pub(crate) name: String,
}

/// 新建一个令牌：`shuku_` 加 32 个随机字节的十六进制。
fn new_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| Error::Config(format!("取随机数失败：{error}")))?;
    Ok(format!("shuku_{}", hex(&bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn valid_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_LIMIT || name.chars().any(char::is_control) {
        return Err(Error::bad_request(format!(
            "设备名不能为空、不能超过 {NAME_LIMIT} 个字"
        )));
    }
    Ok(name)
}

impl App {
    fn cookie_name(&self) -> String {
        format!("{COOKIE_PREFIX}{}", self.0.config.listen.port())
    }

    pub(crate) fn auth_settings(&self) -> AuthSettings {
        *self
            .0
            .auth
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// 新建令牌，返回它的记录和令牌本身。令牌只在这里出现一次。命令行也用它。
    pub fn create_device_token(&self, name: &str) -> Result<(DeviceToken, String)> {
        let name = valid_name(name)?;
        let token = new_token()?;
        let now = now_ms();
        let id =
            self.write(|writer| Ok(writer.create_device_token(name, &digest(&token), now)?))?;
        tracing::info!(id, name, "新建设备令牌");
        let record = DeviceToken {
            id,
            name: name.to_owned(),
            created_at: now,
            last_used_at: None,
        };
        Ok((record, token))
    }

    pub fn device_tokens(&self) -> Result<Vec<DeviceToken>> {
        Ok(self.reader()?.device_tokens()?)
    }

    /// 吊销令牌，令牌不存在时返回 404。
    pub fn revoke_device_token(&self, id: i64) -> Result<()> {
        if !self.write(|writer| Ok(writer.delete_device_token(id)?))? {
            return Err(Error::not_found());
        }
        tracing::info!(id, "吊销设备令牌");
        Ok(())
    }

    /// 令牌对应的设备。顺便记下使用时间，最多隔 [`TOUCH_INTERVAL_MS`] 写一次库。
    fn device_for(&self, token: &str) -> Result<Option<Device>> {
        let Some(record) = self.reader()?.device_token_by_hash(&digest(token))? else {
            return Ok(None);
        };
        let now = now_ms();
        let due = {
            let mut touched = self
                .0
                .auth
                .touched
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let last = touched
                .get(&record.id)
                .copied()
                .or(record.last_used_at)
                .unwrap_or(i64::MIN);
            let due = now.saturating_sub(last) >= TOUCH_INTERVAL_MS;
            if due {
                touched.insert(record.id, now);
            }
            due
        };
        if due
            && let Err(error) = self.write(|writer| Ok(writer.touch_device_token(record.id, now)?))
        {
            // 只影响控制台里显示的最近使用时间，不让请求失败。
            tracing::warn!(%error, "记录令牌使用时间失败");
        }
        Ok(Some(Device {
            id: record.id,
            name: record.name,
        }))
    }

    fn set_auth_settings(&self, settings: AuthSettings) -> Result<()> {
        let mut current = self
            .0
            .auth
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.write(|writer| Ok(writer.set_setting(KEY, &settings)?))?;
        *current = settings;
        Ok(())
    }

    pub(crate) fn auth_router(&self) -> Router<App> {
        Router::new()
            .route("/api/auth", get(status))
            .route("/api/auth", put(update))
            .route("/api/auth/session", post(login).delete(logout))
            .route("/api/admin/tokens", get(list).post(create))
            .route("/api/admin/tokens/{id}", delete(revoke))
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }
}

/// 请求带的令牌：先看 `Authorization: Bearer`，再看名为 `cookie` 的登录 cookie。第二项表示
/// 来自 `Authorization`。认证方案不分大小写；`Authorization` 头格式不对时当作一个空令牌，
/// 验证时返回 401，不当作没带令牌放过去。
fn presented(headers: &HeaderMap, cookie: &str) -> Option<(String, bool)> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        let token = value
            .to_str()
            .ok()
            .and_then(|value| value.trim().split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map_or("", |(_, token)| token.trim());
        return Some((token.to_owned(), true));
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(cookie)?.strip_prefix('='))
        .map(|token| (token.to_owned(), false))
}

fn unauthorized(message: &str) -> Response {
    let mut response = Error::Http {
        status: StatusCode::UNAUTHORIZED,
        message: message.into(),
    }
    .into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

/// 认证中间件，套在整个路由外面。
pub(crate) async fn authenticate(
    State(app): State<App>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let mcp = path == "/mcp" || path.starts_with("/mcp/");
    let api = path == "/api" || path.starts_with("/api/");
    // 登录、查询登录状态和服务器信息不能要求先登录。
    let open = (path == "/api/auth" || path == "/api/info")
        && request.method() == axum::http::Method::GET
        || path == "/api/auth/session";
    let required = mcp || (api && !open && app.auth_settings().require_token);
    let presented = presented(request.headers(), &app.cookie_name());
    // 网页和图片以外，带了令牌就验证；网页端没打开认证时不管 cookie，过期的 cookie
    // 不该让本来不需要令牌的请求失败。
    let device = match &presented {
        Some((token, bearer)) if (mcp || api) && (*bearer || required) => {
            let lookup = {
                let (app, token) = (app.clone(), token.clone());
                tokio::task::spawn_blocking(move || app.device_for(&token)).await
            };
            match lookup {
                Ok(Ok(Some(device))) => Some(device),
                Ok(Ok(None)) => return unauthorized("令牌无效或已吊销"),
                Ok(Err(error)) => return error.into_response(),
                Err(error) => return Error::from(error).into_response(),
            }
        }
        _ => None,
    };
    if required && device.is_none() {
        return unauthorized("需要令牌");
    }
    if let Some(device) = device {
        request.extensions_mut().insert(device);
    }
    let mut response = next.run(request).await;
    // 图片接口的缓存头是 public。需要令牌时改成 private，局域网里的共享缓存（代理）
    // 不能把登录设备取到的图片转给没登录的设备；浏览器自己的缓存照常用。
    if required
        && let Some(value) = response.headers().get(header::CACHE_CONTROL)
        && let Ok(text) = value.to_str()
        && text.contains("public")
        && let Ok(private) = HeaderValue::from_str(&text.replace("public", "private"))
    {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, private);
    }
    response
}

/// 网页端用它决定要不要显示登录页，以及控制台里显示“这台设备”。
async fn status(
    State(app): State<App>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    // 网页端没打开认证时中间件不查 cookie，这里自己查，控制台才知道这台设备登录没有。
    let device = match (device, presented(&headers, &app.cookie_name())) {
        (Some(Extension(device)), _) => Some(device),
        (None, Some((token, _))) => app.blocking(move |app| app.device_for(&token)).await?,
        (None, None) => None,
    };
    Ok(Json(json!({
        "require_token": app.auth_settings().require_token,
        "device": device.map(|device| json!({"id": device.id, "name": device.name})),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthPatch {
    require_token: bool,
}

/// 打开“网页端也需要令牌”时，这个请求本身必须带着有效的令牌，否则一打开就把自己挡在外面。
async fn update(
    State(app): State<App>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
    body: std::result::Result<Json<AuthPatch>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let device = match (device, presented(&headers, &app.cookie_name())) {
            (Some(Extension(device)), _) => Some(device),
            (None, Some((token, _))) => app.device_for(&token)?,
            (None, None) => None,
        };
        if body.require_token && device.is_none() {
            return Err(Error::Http {
                status: StatusCode::CONFLICT,
                message: "先在这台设备上用令牌登录，再打开，免得把自己挡在外面".into(),
            });
        }
        app.set_auth_settings(AuthSettings {
            require_token: body.require_token,
        })?;
        tracing::info!(require_token = body.require_token, "修改认证设置");
        Ok(Json(json!({ "require_token": body.require_token })))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginBody {
    token: String,
}

fn cookie(name: &str, value: &str, max_age: u64) -> HeaderValue {
    // 局域网里用 HTTP，没法加 Secure。SameSite=Strict 让别的网站发来的请求不带它。
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// 网页端登录：验证令牌，放进 cookie。
async fn login(
    State(app): State<App>,
    body: std::result::Result<Json<LoginBody>, JsonRejection>,
) -> Result<Response> {
    let Json(body) = body.map_err(Error::bad_request)?;
    let token = body.token.trim().to_owned();
    // 令牌只由十六进制和下划线组成；其他字符放进 cookie 头会出错。
    if token.is_empty()
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Ok(unauthorized("令牌无效或已吊销"));
    }
    let checked = token.clone();
    let Some(device) = app.blocking(move |app| app.device_for(&checked)).await? else {
        return Ok(unauthorized("令牌无效或已吊销"));
    };
    let mut response =
        Json(json!({"device": {"id": device.id, "name": device.name}})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie(&app.cookie_name(), &token, COOKIE_MAX_AGE),
    );
    Ok(response)
}

async fn logout(State(app): State<App>) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie(&app.cookie_name(), "", 0));
    response
}

fn token_json(token: &DeviceToken) -> Value {
    json!({
        "id": token.id, "name": token.name,
        "created_at": token.created_at, "last_used_at": token.last_used_at,
    })
}

async fn list(State(app): State<App>) -> Result<Json<Value>> {
    app.blocking(|app| {
        let tokens: Vec<Value> = app.device_tokens()?.iter().map(token_json).collect();
        Ok(Json(json!({ "tokens": tokens })))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    name: String,
}

async fn create(
    State(app): State<App>,
    body: std::result::Result<Json<CreateBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>)> {
    let Json(body) = body.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        let (record, token) = app.create_device_token(&body.name)?;
        let mut value = token_json(&record);
        value["token"] = json!(token);
        Ok((StatusCode::CREATED, Json(value)))
    })
    .await
}

async fn revoke(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
) -> Result<StatusCode> {
    let Path(id) = id.map_err(Error::bad_request)?;
    app.blocking(move |app| {
        app.revoke_device_token(id)?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}
