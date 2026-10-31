mod common;

use axum::{Router, http::StatusCode};
use common::{Fixture, request};
use serde_json::{Value, json as value};

async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let (status, headers, bytes) = request(router, method, uri, body, headers).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    )
}

async fn create_token(router: &Router, name: &str, headers: &[(&str, &str)]) -> (i64, String) {
    let (status, body, _) = call(
        router,
        "POST",
        "/api/admin/tokens",
        Some(value!({ "name": name })),
        headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["id"].as_i64().unwrap(),
        body["token"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn tokens_identify_devices_and_bad_ones_are_rejected() {
    let fixture = Fixture::new();
    let router = fixture.app().await.router();

    // 默认网页端和 API 不需要令牌，MCP 需要。
    assert_eq!(
        call(&router, "GET", "/api/search", None, &[]).await.0,
        StatusCode::OK
    );
    let (status, _, headers) = call(&router, "POST", "/mcp", Some(value!({})), &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers["www-authenticate"], "Bearer");

    let (id, token) = create_token(&router, "  手机  ", &[]).await;
    assert!(token.starts_with("shuku_") && token.len() == 6 + 64);
    let bearer = format!("Bearer {token}");
    let (status, body, _) = call(
        &router,
        "GET",
        "/api/auth",
        None,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        value!({"require_token": false, "device": {"id": id, "name": "手机"}})
    );
    // 列表里有最近使用时间，没有令牌本身。
    let (_, body, _) = call(&router, "GET", "/api/admin/tokens", None, &[]).await;
    assert_eq!(body["tokens"][0]["name"], "手机");
    assert!(body["tokens"][0]["last_used_at"].is_i64());
    assert!(body["tokens"][0].get("token").is_none());

    // 带了错的令牌，即使不需要令牌也拒绝，好让程序发现令牌被吊销了；格式不对的也一样。
    for header in ["Bearer shuku_nope", "Basic abc", "Bearer", "shuku_nope"] {
        let (status, _, _) = call(
            &router,
            "GET",
            "/api/search",
            None,
            &[("authorization", header)],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{header}");
    }
    // 认证方案不分大小写。
    let lower = format!("bearer {token}");
    let (status, _, _) = call(
        &router,
        "GET",
        "/api/auth",
        None,
        &[("authorization", &lower)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    for name in ["", "   ", &"名".repeat(101)] {
        let (status, _, _) = call(
            &router,
            "POST",
            "/api/admin/tokens",
            Some(value!({ "name": name })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?}");
    }

    let uri = format!("/api/admin/tokens/{id}");
    assert_eq!(
        call(&router, "DELETE", &uri, None, &[]).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&router, "DELETE", &uri, None, &[]).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _, _) = call(
        &router,
        "GET",
        "/api/auth",
        None,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn requiring_tokens_locks_the_api_but_not_login_or_the_web_app() {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let (_, search, _) = call(&router, "GET", "/api/search", None, &[]).await;
    let cover = search["items"][0]["cover_url"].as_str().unwrap().to_owned();
    let (_, _, headers) = call(&router, "GET", &cover, None, &[]).await;
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .starts_with("public")
    );
    let (_, token) = create_token(&router, "电脑", &[]).await;

    // 没登录就打开，会把自己挡在外面，拒绝。
    let enable = value!({"require_token": true});
    let (status, _, _) = call(&router, "PUT", "/api/auth", Some(enable.clone()), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _, _) = call(
        &router,
        "POST",
        "/api/auth/session",
        Some(value!({"token": "shuku_wrong"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body, headers) = call(
        &router,
        "POST",
        "/api/auth/session",
        Some(value!({ "token": token })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["device"]["name"], "电脑");
    let set_cookie = headers["set-cookie"].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    // cookie 名带监听端口，同一台机器上别的实例的登录不算。
    assert!(cookie.starts_with("shuku_token_0="), "{cookie}");
    let other_instance = cookie.replacen("shuku_token_0=", "shuku_token_3002=", 1);

    let with_cookie = [("cookie", cookie.as_str())];
    let (status, _, _) = call(&router, "PUT", "/api/auth", Some(enable), &with_cookie).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        call(&router, "GET", "/api/search", None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&router, "GET", "/api/search", None, &with_cookie)
            .await
            .0,
        StatusCode::OK
    );
    // 需要令牌时图片只让浏览器自己缓存，共享缓存不能转给别的设备。
    let (status, _, headers) = call(&router, "GET", &cover, None, &with_cookie).await;
    assert_eq!(status, StatusCode::OK);
    let cache = headers["cache-control"].to_str().unwrap();
    assert!(
        cache.starts_with("private") && !cache.contains("public"),
        "{cache}"
    );
    assert_eq!(
        call(&router, "GET", &cover, None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &router,
            "GET",
            "/api/search",
            None,
            &[("cookie", &other_instance)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // 登录状态、登录本身和网页不需要令牌。
    let (status, body, _) = call(&router, "GET", "/api/auth", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, value!({"require_token": true, "device": null}));
    assert_eq!(call(&router, "GET", "/", None, &[]).await.0, StatusCode::OK);
    assert_eq!(
        call(&router, "GET", "/api/admin/tokens", None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );

    let (status, _, headers) =
        call(&router, "DELETE", "/api/auth/session", None, &with_cookie).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        headers["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );

    // 关掉以后又不需要了。
    let (status, _, _) = call(
        &router,
        "PUT",
        "/api/auth",
        Some(value!({"require_token": false})),
        &with_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(&router, "GET", "/api/search", None, &[]).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_setting_survives_a_restart() {
    let fixture = Fixture::new();
    let app = fixture.app().await;
    let (_, token) = app.create_device_token("命令行").unwrap();
    let router = app.router();
    let bearer = format!("Bearer {token}");
    let (status, _, _) = call(
        &router,
        "PUT",
        "/api/auth",
        Some(value!({"require_token": true})),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    drop(router);
    drop(app);
    let router = fixture.app().await.router();
    assert_eq!(
        call(&router, "GET", "/api/search", None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
}
