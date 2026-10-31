//! 给 agent 用的 MCP 服务（Streamable HTTP），挂在 `/mcp`。
//!
//! 每个工具对应 `/api` 下的一个接口：调用工具时在进程里向同一个路由发一个 HTTP 请求，
//! 带上调用方的 `Authorization` 头，所以校验、认证和写入的规则和网页端完全一样，
//! 这里只负责把工具参数翻成请求、把响应翻成工具结果。只做无状态的请求-响应：
//! POST 一条 JSON-RPC 消息，回一条 JSON；不开 SSE 流，也不发会话 ID。

use crate::{App, Error, Result};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Map, Value, json};
use tower::ServiceExt;

/// 支持的协议版本，新的在前。客户端要的版本在里面就用它，否则回最新的。
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// 工具结果里接口响应的长度上限（字节），太长的截断，免得塞满 agent 的上下文。
const RESULT_LIMIT: usize = 200_000;

const INSTRUCTIONS: &str = "Shuku 是自用的漫画书库。用 search 找书（语法同 LRR：逗号分隔词条，\
`artist:某某` 匹配标签前缀，末尾加 $ 精确匹配，前面加 - 排除，`pages:>20`、`read:0` 比较页数和已读页），\
get_archive 看一本书的详情。改元数据用 edit_archive，或 batch_edit 一次改很多本。\
帮忙翻译或整理在线来源查到的元数据时：metadata_candidates 列出待确认的候选，\
把 found 里的标签、简介译好后用 update_metadata_candidate 写回，再 approve_metadata_candidate 批准，\
不合适的用 reject_metadata_candidate 拒绝；queue_metadata 把书放进某个来源的查找队列。\
标签写成 `命名空间:值`，不能含英文逗号。";

/// 一个工具调用要发的请求。
struct Call {
    method: Method,
    path: String,
    query: Vec<(&'static str, String)>,
    body: Option<Value>,
}

impl Call {
    fn get(path: impl Into<String>) -> Self {
        Self {
            method: Method::GET,
            path: path.into(),
            query: Vec::new(),
            body: None,
        }
    }

    fn with_body(method: Method, path: impl Into<String>, body: Value) -> Self {
        Self {
            method,
            path: path.into(),
            query: Vec::new(),
            body: Some(body),
        }
    }

    /// 把参数里有的几项原样放进查询串。数组、对象这类放不进查询串的值报错，不能悄悄丢掉：
    /// 丢掉一个筛选条件会让查询范围变大。
    fn query_from(
        mut self,
        args: &Map<String, Value>,
        keys: &[&'static str],
    ) -> std::result::Result<Self, String> {
        for &key in keys {
            match args.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::String(text)) => self.query.push((key, text.clone())),
                Some(Value::Number(number)) => self.query.push((key, number.to_string())),
                Some(Value::Bool(flag)) => self.query.push((key, flag.to_string())),
                Some(_) => return Err(format!("参数 {key} 的类型不对")),
            }
        }
        Ok(self)
    }
}

type Builder = fn(&Map<String, Value>) -> std::result::Result<Call, String>;

struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    build: Builder,
    /// 只读工具，告诉客户端可以放心调用。
    read_only: bool,
}

fn int(args: &Map<String, Value>, key: &str) -> std::result::Result<i64, String> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("缺少整数参数 {key}"))
}

fn text(args: &Map<String, Value>, key: &str) -> std::result::Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("缺少字符串参数 {key}"))
}

/// 只留下给出的几项，原样作为请求体。
fn pick(args: &Map<String, Value>, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|&key| Some((key.to_owned(), args.get(key)?.clone())))
            .collect(),
    )
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn search_properties() -> Value {
    json!({
        "q": { "type": "string", "description": "LRR 风格的搜索语句，空串是全部" },
        "category": { "type": "integer", "description": "只在这个分类里找" },
        "filter": { "type": "string", "description": "逗号分隔的 new、unread、reading、finished、unfinished、untagged" },
    })
}

fn merged(base: Value, extra: Value) -> Value {
    let mut base = base;
    if let (Some(base), Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

fn found_properties() -> Value {
    json!({
        "title": { "type": ["string", "null"] },
        "summary": { "type": ["string", "null"] },
        "source_url": { "type": ["string", "null"] },
        "tags": { "type": "array", "items": { "type": "object", "properties": {
            "namespace": { "type": "string" }, "value": { "type": "string" } },
            "required": ["namespace", "value"] } },
    })
}

fn candidate_path(args: &Map<String, Value>) -> std::result::Result<String, String> {
    let provider = text(args, "provider")?;
    if !provider.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("provider 无效".into());
    }
    Ok(format!(
        "/api/metadata/candidates/{}/{provider}",
        int(args, "id")?
    ))
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "search",
        description: "搜索书库，返回一页结果（标题、页数、标签、进度）。next_after 不为空时把它作为 after 取下一页。",
        schema: || {
            object(
                merged(
                    search_properties(),
                    json!({
                        "sort": { "type": "string", "description": "title、date、pages 或 ns:<命名空间>" },
                        "order": { "type": "string", "enum": ["asc", "desc"] },
                        "group": { "type": "string", "enum": ["series"], "description": "每个系列只列一卷" },
                        "after": { "type": "integer" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    }),
                ),
                &[],
            )
        },
        build: |args| {
            Call::get("/api/search").query_from(
                args,
                &[
                    "q", "category", "filter", "sort", "order", "group", "after", "limit",
                ],
            )
        },
        read_only: true,
    },
    Tool {
        name: "get_archive",
        description: "一本书的详情：标题、简介、标签、来源、所在分类、阅读进度。",
        schema: || object(json!({ "id": { "type": "integer" } }), &["id"]),
        build: |args| Ok(Call::get(format!("/api/archives/{}", int(args, "id")?))),
        read_only: true,
    },
    Tool {
        name: "get_series",
        description: "一个系列的各卷，按卷号排列。",
        schema: || object(json!({ "id": { "type": "integer" } }), &["id"]),
        build: |args| Ok(Call::get(format!("/api/series/{}", int(args, "id")?))),
        read_only: true,
    },
    Tool {
        name: "random_archive",
        description: "在搜索条件范围里随机挑一本，返回它的 ID。",
        schema: || object(search_properties(), &[]),
        build: |args| Call::get("/api/random").query_from(args, &["q", "category", "filter"]),
        read_only: true,
    },
    Tool {
        name: "recent",
        description: "最近读过的书，新的在前。",
        schema: || {
            object(
                json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 100 } }),
                &[],
            )
        },
        build: |args| Call::get("/api/recent").query_from(args, &["limit"]),
        read_only: true,
    },
    Tool {
        name: "suggest_tags",
        description: "以给定前缀开头的标签和用到它们的本数，用来查一个标签在书库里的写法。",
        schema: || {
            object(
                json!({ "prefix": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 100 } }),
                &["prefix"],
            )
        },
        build: |args| Call::get("/api/tags").query_from(args, &["prefix", "limit"]),
        read_only: true,
    },
    Tool {
        name: "tag_facets",
        description: "搜索结果里出现最多的标签。",
        schema: || {
            object(
                merged(
                    search_properties(),
                    json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 100 } }),
                ),
                &[],
            )
        },
        build: |args| {
            Call::get("/api/facets").query_from(args, &["q", "category", "filter", "limit"])
        },
        read_only: true,
    },
    Tool {
        name: "list_categories",
        description: "全部分类和各自的本数。dynamic 为真的分类由搜索语句决定成员，不能手动加减。",
        schema: || object(json!({}), &[]),
        build: |_| Ok(Call::get("/api/categories")),
        read_only: true,
    },
    Tool {
        name: "edit_archive",
        description: "改一本书的标题、简介、来源或标签。tags 整体替换；add_tags、remove_tags 只加或只删，不能和 tags 同时给。",
        schema: || {
            object(
                json!({
                    "id": { "type": "integer" },
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "source_url": { "type": "string", "description": "空串清除来源" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "add_tags": { "type": "array", "items": { "type": "string" } },
                    "remove_tags": { "type": "array", "items": { "type": "string" } },
                }),
                &["id"],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::PATCH,
                format!("/api/archives/{}", int(args, "id")?),
                pick(
                    args,
                    &[
                        "title",
                        "summary",
                        "source_url",
                        "tags",
                        "add_tags",
                        "remove_tags",
                    ],
                ),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "batch_edit",
        description: "一次给很多本加减标签、加进或移出分类，一个事务，最多 10000 本。ids 和 search 二选一；search 是和搜索相同的条件，改全部结果。",
        schema: || {
            object(
                json!({
                    "ids": { "type": "array", "items": { "type": "integer" } },
                    "search": { "type": "object", "properties": search_properties() },
                    "add_tags": { "type": "array", "items": { "type": "string" } },
                    "remove_tags": { "type": "array", "items": { "type": "string" } },
                    "add_to_category": { "type": "integer" },
                    "remove_from_category": { "type": "integer" },
                }),
                &[],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::POST,
                "/api/batch",
                pick(
                    args,
                    &[
                        "ids",
                        "search",
                        "add_tags",
                        "remove_tags",
                        "add_to_category",
                        "remove_from_category",
                    ],
                ),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "set_category_member",
        description: "把一本书加进静态分类（member 为真）或移出。",
        schema: || {
            object(
                json!({ "category": { "type": "integer" }, "archive": { "type": "integer" }, "member": { "type": "boolean" } }),
                &["category", "archive", "member"],
            )
        },
        build: |args| {
            let member = args
                .get("member")
                .and_then(Value::as_bool)
                .ok_or("缺少布尔参数 member")?;
            Ok(Call {
                method: if member { Method::PUT } else { Method::DELETE },
                path: format!(
                    "/api/categories/{}/archives/{}",
                    int(args, "category")?,
                    int(args, "archive")?
                ),
                query: Vec::new(),
                body: None,
            })
        },
        read_only: false,
    },
    Tool {
        name: "create_category",
        description: "新建分类。search 非空是动态分类。",
        schema: || {
            object(
                json!({ "name": { "type": "string" }, "search": { "type": "string" }, "pinned": { "type": "boolean" } }),
                &["name"],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::POST,
                "/api/categories",
                pick(args, &["name", "search", "pinned"]),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "rename_tag",
        description: "把标签 from 改名为 to；to 已经存在时两个合并。",
        schema: || {
            object(
                json!({ "from": { "type": "string" }, "to": { "type": "string" } }),
                &["from", "to"],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::POST,
                "/api/admin/tags/rename",
                pick(args, &["from", "to"]),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "fetch_metadata",
        description: "对一本书运行一个元数据来源（filename、comicinfo、ehentai、bangumi、anilist）。apply 为假时只预览、不写入。",
        schema: || {
            object(
                json!({
                    "id": { "type": "integer" },
                    "provider": { "type": "string", "enum": ["filename", "comicinfo", "ehentai", "bangumi", "anilist"] },
                    "url": { "type": "string", "description": "直接用这个画廊或条目，不搜索" },
                    "apply": { "type": "boolean" },
                }),
                &["id", "provider"],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::POST,
                format!("/api/archives/{}/metadata", int(args, "id")?),
                pick(args, &["provider", "url", "apply"]),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "metadata_status",
        description: "每个在线来源的队列长度、待确认的候选数、是否暂停，以及翻译数据库的状态。",
        schema: || object(json!({}), &[]),
        build: |_| Ok(Call::get("/api/admin/metadata")),
        read_only: true,
    },
    Tool {
        name: "metadata_queue",
        description: "在线来源查找队列里的书。翻页时把上一页最后一条的 id、provider 作为 after、after_provider。",
        schema: || {
            object(
                json!({
                    "provider": { "type": "string" }, "after": { "type": "integer" },
                    "after_provider": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 },
                }),
                &[],
            )
        },
        build: |args| {
            Call::get("/api/metadata/queue")
                .query_from(args, &["provider", "after", "after_provider", "limit"])
        },
        read_only: true,
    },
    Tool {
        name: "queue_metadata",
        description: "把书放进一个在线来源（ehentai、bangumi、anilist）的后台查找队列，一次 1 到 1000 本。",
        schema: || {
            object(
                json!({ "provider": { "type": "string" }, "ids": { "type": "array", "items": { "type": "integer" } } }),
                &["provider", "ids"],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::POST,
                "/api/metadata/queue",
                pick(args, &["provider", "ids"]),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "metadata_candidates",
        description: "在线来源查到、等确认的候选：archive 是书现在的元数据，found 是将要写入的内容。翻页同 metadata_queue。",
        schema: || {
            object(
                json!({
                    "provider": { "type": "string" }, "after": { "type": "integer" },
                    "after_provider": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 },
                }),
                &[],
            )
        },
        build: |args| {
            Call::get("/api/metadata/candidates")
                .query_from(args, &["provider", "after", "after_provider", "limit"])
        },
        read_only: true,
    },
    Tool {
        name: "get_metadata_candidate",
        description: "一条候选。",
        schema: || {
            object(
                json!({ "id": { "type": "integer" }, "provider": { "type": "string" } }),
                &["id", "provider"],
            )
        },
        build: |args| Ok(Call::get(candidate_path(args)?)),
        read_only: true,
    },
    Tool {
        name: "update_metadata_candidate",
        description: "整个替换一条候选将要写入的内容，例如把标签、简介译成中文后写回。",
        schema: || {
            object(
                merged(
                    json!({ "id": { "type": "integer" }, "provider": { "type": "string" } }),
                    found_properties(),
                ),
                &["id", "provider"],
            )
        },
        build: |args| {
            Ok(Call::with_body(
                Method::PUT,
                candidate_path(args)?,
                pick(args, &["title", "summary", "source_url", "tags"]),
            ))
        },
        read_only: false,
    },
    Tool {
        name: "approve_metadata_candidate",
        description: "批准一条候选并写入。给了 found 时写它，不管存着的内容。",
        schema: || {
            object(
                json!({
                    "id": { "type": "integer" }, "provider": { "type": "string" },
                    "found": { "type": "object", "properties": found_properties() },
                }),
                &["id", "provider"],
            )
        },
        build: |args| {
            let path = format!("{}/approve", candidate_path(args)?);
            Ok(match args.get("found") {
                Some(found) => Call::with_body(Method::POST, path, found.clone()),
                None => Call {
                    method: Method::POST,
                    path,
                    query: Vec::new(),
                    body: None,
                },
            })
        },
        read_only: false,
    },
    Tool {
        name: "reject_metadata_candidate",
        description: "拒绝一条候选，删掉它，不写入。",
        schema: || {
            object(
                json!({ "id": { "type": "integer" }, "provider": { "type": "string" } }),
                &["id", "provider"],
            )
        },
        build: |args| {
            Ok(Call {
                method: Method::DELETE,
                path: candidate_path(args)?,
                query: Vec::new(),
                body: None,
            })
        },
        read_only: false,
    },
];

impl App {
    pub(crate) fn mcp_router(&self) -> Router<App> {
        Router::new()
            .route("/mcp", post(handle))
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }

    /// 在进程里调用一个 `/api` 接口，返回状态码和响应体。
    async fn call_api(&self, call: Call, credentials: &HeaderMap) -> Result<(StatusCode, Vec<u8>)> {
        let mut uri = call.path;
        if !call.query.is_empty() {
            uri.push('?');
            uri.push_str(&serde_urlencoded::to_string(&call.query).map_err(Error::bad_request)?);
        }
        let mut request = Request::builder().method(call.method).uri(uri);
        // 外层请求凭什么通过的认证（Bearer 头或登录 cookie），内层请求就带什么。
        for name in [header::AUTHORIZATION, header::COOKIE] {
            for value in credentials.get_all(&name) {
                request = request.header(&name, value);
            }
        }
        let body = match call.body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let request = request.body(body).map_err(Error::bad_request)?;
        let response = self
            .router()
            .oneshot(request)
            .await
            .unwrap_or_else(|never| match never {});
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|error| Error::Write(error.to_string()))?;
        Ok((status, bytes.to_vec()))
    }
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

async fn handle(
    State(app): State<App>,
    headers: HeaderMap,
    body: std::result::Result<Json<Value>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return Json(rpc_error(Value::Null, -32700, "请求体不是 JSON")).into_response();
    };
    // 2025-03-26 版允许一次 POST 一个数组（批量），之后的版本去掉了；两种都接受。
    let replies = match body {
        Value::Array(messages) if !messages.is_empty() => {
            let mut replies = Vec::new();
            for message in messages {
                replies.extend(handle_message(&app, &headers, message).await);
            }
            Some(Value::Array(replies))
                .filter(|replies| replies.as_array().is_some_and(|r| !r.is_empty()))
        }
        Value::Array(_) => Some(rpc_error(Value::Null, -32600, "批量请求不能是空数组")),
        message => handle_message(&app, &headers, message).await,
    };
    match replies {
        Some(replies) => Json(replies).into_response(),
        // 只有通知或响应时不回内容。
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// 处理一条消息，请求返回它的响应，通知和客户端发来的响应返回 `None`。格式不对的消息
/// 在做任何事之前就以 -32600 拒绝。
async fn handle_message(app: &App, headers: &HeaderMap, message: Value) -> Option<Value> {
    let invalid = |message: &str| Some(rpc_error(Value::Null, -32600, message));
    let Value::Object(fields) = &message else {
        return invalid("消息必须是对象");
    };
    if fields.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid("jsonrpc 必须是 \"2.0\"");
    }
    let id = match fields.get("id") {
        None => None,
        Some(id @ (Value::String(_) | Value::Number(_))) => Some(id.clone()),
        Some(_) => return invalid("id 必须是字符串或数字"),
    };
    let method = match fields.get("method") {
        Some(Value::String(method)) => method.clone(),
        Some(_) => return invalid("method 必须是字符串"),
        // 客户端回的响应。这个服务不向客户端发请求，收到就算了。
        None if id.is_some() && (fields.contains_key("result") || fields.contains_key("error")) => {
            return None;
        }
        None => return invalid("缺少 method"),
    };
    let params = match fields.get("params") {
        None | Some(Value::Null) => Value::Null,
        Some(params @ Value::Object(_)) => params.clone(),
        Some(_) => {
            return Some(rpc_error(
                id.unwrap_or(Value::Null),
                -32600,
                "params 必须是对象",
            ));
        }
    };
    // 通知不回内容，也不做任何事：这里支持的通知只有 initialized 之类，都不需要处理。
    let id = id?;
    let method = method.as_str();
    let reply = match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|version| Some(**version) == requested)
                .unwrap_or(&PROTOCOL_VERSIONS[0]);
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "shuku", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            let tools: Vec<Value> = TOOLS
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name, "description": tool.description,
                        "inputSchema": (tool.schema)(),
                        "annotations": { "readOnlyHint": tool.read_only },
                    })
                })
                .collect();
            rpc_result(id, json!({ "tools": tools }))
        }
        "tools/call" => call_tool(app, id, &params, headers).await,
        _ => rpc_error(id, -32601, format!("不支持的方法 {method}")),
    };
    Some(reply)
}

async fn call_tool(app: &App, id: Value, params: &Value, credentials: &HeaderMap) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(id, -32602, "缺少工具名 name");
    };
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
        return rpc_error(id, -32602, format!("没有工具 {name}"));
    };
    let empty = Map::new();
    let args = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(args)) => args,
        Some(_) => return rpc_error(id, -32602, "arguments 必须是对象"),
    };
    // 工具只把认识的参数转给接口，拼错的参数会被悄悄丢掉、让一次编辑只做了一半，
    // 所以 schema 里没有的参数直接拒绝，和接口的 deny_unknown_fields 一样。
    let schema = (tool.schema)();
    if let Some(unknown) = args
        .keys()
        .find(|key| schema["properties"].get(key.as_str()).is_none())
    {
        return rpc_error(id, -32602, format!("工具 {name} 没有参数 {unknown}"));
    }
    let call = match (tool.build)(args) {
        Ok(call) => call,
        Err(message) => return rpc_error(id, -32602, message),
    };
    // 接口返回的错误（400、404、409 等）是工具执行失败，按 MCP 的约定放进结果、标上 isError，
    // 让 agent 看到原因自己改参数重试。
    let (text, is_error) = match app.call_api(call, credentials).await {
        Ok((status, bytes)) => {
            let mut text = if bytes.is_empty() {
                json!({ "status": status.as_u16() }).to_string()
            } else {
                String::from_utf8_lossy(&bytes).into_owned()
            };
            if text.len() > RESULT_LIMIT {
                let cut = (0..=RESULT_LIMIT)
                    .rev()
                    .find(|&at| text.is_char_boundary(at))
                    .unwrap_or(0);
                text.truncate(cut);
                text.push_str("\n…（结果太长，已截断，请缩小范围或减小 limit）");
            }
            (text, !status.is_success())
        }
        Err(error) => (error.to_string(), true),
    };
    rpc_result(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
    )
}
