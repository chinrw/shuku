mod common;

use axum::{Router, http::StatusCode};
use common::{Fixture, request};
use serde_json::{Value, json as value};

struct Client {
    router: Router,
    bearer: String,
}

impl Client {
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let body = value!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
        let (status, _, bytes) = request(
            &self.router,
            "POST",
            "/mcp",
            Some(body),
            &[("authorization", &self.bearer)],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            (&reply["jsonrpc"], &reply["id"]),
            (&value!("2.0"), &value!(7))
        );
        reply
    }

    /// 调用工具，返回 (是否出错, 结果文本解析成的 JSON)。
    async fn call(&self, name: &str, arguments: Value) -> (bool, Value) {
        let reply = self
            .rpc("tools/call", value!({"name": name, "arguments": arguments}))
            .await;
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        (
            result["isError"].as_bool().unwrap(),
            serde_json::from_str(text).unwrap(),
        )
    }
}

async fn client() -> (Fixture, Client, i64) {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let (_, token) = app.create_device_token("agent").unwrap();
    let router = app.router();
    let client = Client {
        router,
        bearer: format!("Bearer {token}"),
    };
    let (_, search) = client.call("search", value!({})).await;
    let id = search["items"][0]["id"].as_i64().unwrap();
    (fixture, client, id)
}

#[tokio::test]
async fn agents_need_a_token_and_get_the_protocol_basics() {
    let (_fixture, client, _) = client().await;
    let body = value!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    let (status, _, _) = request(&client.router, "POST", "/mcp", Some(body), &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = request(
        &client.router,
        "GET",
        "/mcp",
        None,
        &[("authorization", &client.bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    let reply = client
        .rpc(
            "initialize",
            value!({"protocolVersion": "2025-03-26", "capabilities": {}}),
        )
        .await;
    assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(reply["result"]["serverInfo"]["name"], "shuku");
    assert!(reply["result"]["capabilities"]["tools"].is_object());
    let reply = client
        .rpc("initialize", value!({"protocolVersion": "1999-01-01"}))
        .await;
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");

    let note = value!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let (status, _, _) = request(
        &client.router,
        "POST",
        "/mcp",
        Some(note),
        &[("authorization", &client.bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let tools = client.rpc("tools/list", value!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            assert_eq!(tool["inputSchema"]["type"], "object");
            tool["name"].as_str().unwrap()
        })
        .collect();
    for name in [
        "search",
        "edit_archive",
        "batch_edit",
        "metadata_candidates",
        "approve_metadata_candidate",
    ] {
        assert!(names.contains(&name), "{name}");
    }
    assert_eq!(
        client.rpc("nope", value!({})).await["error"]["code"],
        -32601
    );
}

#[tokio::test]
async fn tools_read_and_write_through_the_api() {
    let (_fixture, client, id) = client().await;

    let (error, body) = client
        .call(
            "edit_archive",
            value!({"id": id, "add_tags": ["artist:某某"], "title": "新标题"}),
        )
        .await;
    assert!(!error, "{body}");
    assert_eq!(body["title"], "新标题");
    let (_, detail) = client.call("get_archive", value!({"id": id})).await;
    assert_eq!(
        detail["tags"][0],
        value!({"namespace": "artist", "values": ["某某"]})
    );

    let (_, category) = client
        .call("create_category", value!({"name": "收藏"}))
        .await;
    let category = category["id"].as_i64().unwrap();
    let (error, _) = client
        .call(
            "set_category_member",
            value!({"category": category, "archive": id, "member": true}),
        )
        .await;
    assert!(!error);
    let (_, categories) = client.call("list_categories", value!({})).await;
    assert_eq!(categories["categories"][0]["count"], 1);
    let (_, found) = client
        .call("search", value!({"category": category, "limit": 10}))
        .await;
    assert_eq!(found["total"], 1);

    // 接口的错误是工具失败，带着原因放进结果。
    let (error, body) = client.call("get_archive", value!({"id": id + 100})).await;
    assert!(error);
    assert!(body["error"].is_string());
    let (error, body) = client
        .call("edit_archive", value!({"id": id, "add_tags": ["a,b"]}))
        .await;
    assert!(error, "{body}");

    let (error, body) = client.call("metadata_candidates", value!({})).await;
    assert!(!error, "{body}");
    assert_eq!(body["items"], value!([]));
    let (error, _) = client
        .call(
            "reject_metadata_candidate",
            value!({"id": id, "provider": "bangumi"}),
        )
        .await;
    assert!(error);

    // 参数不对是协议层的错误。
    let reply = client
        .rpc(
            "tools/call",
            value!({"name": "get_archive", "arguments": {}}),
        )
        .await;
    assert_eq!(reply["error"]["code"], -32602);
    let reply = client
        .rpc("tools/call", value!({"name": "missing_tool"}))
        .await;
    assert_eq!(reply["error"]["code"], -32602);
    // 拼错的参数整个调用都拒绝，标题也不改。
    let reply = client
        .rpc(
            "tools/call",
            value!({"name": "edit_archive", "arguments": {"id": id, "title": "不该改", "add_tag": ["x"]}}),
        )
        .await;
    assert_eq!(reply["error"]["code"], -32602);
    let (_, detail) = client.call("get_archive", value!({"id": id})).await;
    assert_eq!(detail["title"], "新标题");
    let reply = client
        .rpc(
            "tools/call",
            value!({"name": "get_metadata_candidate", "arguments": {"id": id, "provider": "../x"}}),
        )
        .await;
    assert_eq!(reply["error"]["code"], -32602);
}

#[tokio::test]
async fn wrong_argument_types_fail_instead_of_widening_the_query() {
    let (_fixture, client, _) = client().await;
    for arguments in [
        value!({"q": ["artist:foo"]}),
        value!({"category": {}}),
        value!({"limit": [1]}),
    ] {
        let reply = client
            .rpc(
                "tools/call",
                value!({"name": "search", "arguments": arguments}),
            )
            .await;
        assert_eq!(reply["error"]["code"], -32602, "{arguments}");
    }
}

#[tokio::test]
async fn a_browser_login_cookie_also_works_for_tool_calls() {
    let (_fixture, client, _) = client().await;
    let token = client.bearer.trim_start_matches("Bearer ").to_owned();
    let (_, headers, _) = request(
        &client.router,
        "POST",
        "/api/auth/session",
        Some(value!({ "token": token })),
        &[],
    )
    .await;
    let cookie = headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let (status, _, _) = request(
        &client.router,
        "PUT",
        "/api/auth",
        Some(value!({"require_token": true})),
        &[("cookie", &cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = value!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": "search", "arguments": {}}});
    let (status, _, bytes) = request(
        &client.router,
        "POST",
        "/mcp",
        Some(body),
        &[("cookie", &cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
}

#[tokio::test]
async fn batches_get_one_reply_per_request_and_bad_envelopes_change_nothing() {
    let (_fixture, client, _) = client().await;
    let post = |body: Value| {
        let (router, bearer) = (client.router.clone(), client.bearer.clone());
        async move {
            let (status, _, bytes) = request(
                &router,
                "POST",
                "/mcp",
                Some(body),
                &[("authorization", &bearer)],
            )
            .await;
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };

    let (status, replies) = post(value!([
        {"jsonrpc": "2.0", "id": 1, "method": "ping"},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": "b", "method": "tools/call",
         "params": {"name": "list_categories", "arguments": {}}},
    ]))
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&Value> = replies
        .as_array()
        .unwrap()
        .iter()
        .map(|reply| &reply["id"])
        .collect();
    assert_eq!(ids, [&value!(1), &value!("b")]);
    let (status, _) =
        post(value!([{"jsonrpc": "2.0", "method": "notifications/initialized"}])).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    for body in [
        value!({"jsonrpc": "1.0", "id": 1, "method": "tools/call",
                "params": {"name": "create_category", "arguments": {"name": "不该有"}}}),
        value!({"jsonrpc": "2.0", "id": {}, "method": "tools/call",
                "params": {"name": "create_category", "arguments": {"name": "不该有"}}}),
        value!({"jsonrpc": "2.0", "id": 1, "method": 42}),
        value!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": ["x"]}),
        value!([]),
    ] {
        let (status, reply) = post(body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(reply["error"]["code"], -32600, "{body}");
    }
    let (_, categories) = client.call("list_categories", value!({})).await;
    assert_eq!(categories["categories"], value!([]));
}
