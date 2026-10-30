mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::{Value, json as value};
use shuku_store::{Store, Tag};

#[tokio::test]
async fn serves_static_assets_and_spa_routes_but_never_api_fallbacks() {
    let fixture = Fixture::new();
    let router = fixture.app().await.router();
    for uri in ["/", "/reader/42", "/missing.js", "/apix"] {
        let (status, headers, body) = request(&router, "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "text/html");
        assert_eq!(body, "<html>书库</html>".as_bytes());
    }
    assert_eq!(
        request(&router, "GET", "/app.js", None, &[]).await.2,
        b"const shuku = true;"
    );
    for uri in ["/api", "/api/", "/api/does-not-exist"] {
        let (status, headers, body) = request(&router, "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(headers["content-type"], "application/json");
        assert!(serde_json::from_slice::<Value>(&body).unwrap()["error"].is_string());
    }
    let (status, _, body) = request(&router, "POST", "/api/search", None, &[]).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(serde_json::from_slice::<Value>(&body).unwrap()["error"].is_string());
    std::fs::write(fixture.root.path().join("secret.txt"), "private").unwrap();
    let (_, _, body) = request(&router, "GET", "/%2e%2e/secret.txt", None, &[]).await;
    assert_ne!(body, b"private");
}

#[tokio::test]
async fn progress_rejects_stale_equal_and_out_of_range_reports() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let uri = format!("/api/archives/{id}/progress");
    for (page, time, accepted, expected_page, expected_time) in [
        (1, 2000, true, 1, 2000),
        (0, 1999, false, 1, 2000),
        (0, 2000, false, 1, 2000),
        (0, 2001, true, 0, 2001),
    ] {
        let (status, _, bytes) = request(
            &router,
            "PUT",
            &uri,
            Some(value!({"page":page,"client_time":time})),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            value!({"accepted":accepted,"progress":{"page":expected_page,"client_time":expected_time}})
        );
    }
    for body in [
        value!({"page":2,"client_time":3000}),
        value!({"page":-1,"client_time":3000}),
        value!({"page":0}),
    ] {
        let (status, _, bytes) = request(&router, "PUT", &uri, Some(body), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"].is_string());
    }
    let current = value!({"page":0,"client_time":2001});
    assert_eq!(
        json(&router, &format!("/api/archives/{id}")).await["progress"],
        current
    );
    assert_eq!(
        json(&router, "/api/search").await["items"][0]["progress"],
        current
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            "/api/archives/999999/progress",
            Some(value!({"page":0,"client_time":1})),
            &[]
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn search_paginates_and_returns_details_pages_and_query_errors() {
    let fixture = Fixture::new();
    for title in ["漫画 10", "漫画 2", "other"] {
        fixture.archive(&format!("{title}.cbz"), 80, 120);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    let id = store.find_by_path("漫画 2.cbz").unwrap().unwrap();
    store
        .replace_tags(
            id,
            &[
                Tag {
                    namespace: "artist".into(),
                    value: "乙".into(),
                },
                Tag {
                    namespace: "".into(),
                    value: "独立标签".into(),
                },
                Tag {
                    namespace: "artist".into(),
                    value: "甲".into(),
                },
            ],
        )
        .unwrap();
    let mut metadata = store.archive(id).unwrap().unwrap().metadata;
    metadata.summary = "摘要".into();
    metadata.date_added = 1;
    metadata.source_url = Some("https://example.org/book".into());
    store
        .save_archive(Some(id), &metadata, &store.pages(id).unwrap())
        .unwrap();
    app.scan().await.unwrap();
    let router = app.router();
    let first = json(&router, "/api/search?q=%E6%BC%AB%E7%94%BB&limit=1").await;
    assert_eq!(first["total"], 2);
    assert_eq!(first["degraded"], false);
    assert_eq!(first["items"][0]["title"], "漫画 2");
    assert_eq!(first["items"][0]["progress"], Value::Null);
    assert_eq!(first["items"][0]["page_count"], 2);
    assert_eq!(first["items"][0]["is_new"], true);
    assert_eq!(
        first["items"][0]["tags"],
        value!([
            {"namespace": "", "values": ["独立标签"]},
            {"namespace": "artist", "values": ["乙", "甲"]}
        ])
    );
    assert_eq!(first["next_after"], id);
    let second = json(
        &router,
        &format!("/api/search?q=%E6%BC%AB%E7%94%BB&limit=1&after={id}"),
    )
    .await;
    assert_eq!(second["items"][0]["title"], "漫画 10");
    assert_eq!(second["next_after"], Value::Null);
    assert_eq!(json(&router, "/api/search?q=artist:*").await["total"], 1);
    assert_eq!(json(&router, "/api/search?q=o*er").await["degraded"], true);
    assert_eq!(
        json(&router, "/api/search?sort=date").await["items"][2]["id"],
        id
    );
    assert_eq!(
        json(&router, "/api/search?q=absent").await["items"],
        value!([])
    );
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    assert_eq!(detail["summary"], "摘要");
    assert_eq!(detail["date_added"], 1);
    assert_eq!(detail["source_url"], "https://example.org/book");
    assert_eq!(
        detail["tags"],
        value!([
            {"namespace": "", "values": ["独立标签"]},
            {"namespace": "artist", "values": ["乙", "甲"]}
        ])
    );
    assert_eq!(detail["cover_url"], first["items"][0]["cover_url"]);
    let pages = json(&router, &format!("/api/archives/{id}/pages")).await;
    assert_eq!(pages["pages"].as_array().unwrap().len(), 2);
    assert_eq!(pages["pages"][0]["width"], 80);
    assert_eq!(pages["pages"][0]["height"], 120);
    assert_eq!(
        pages["pages"][0]["url"],
        format!("/api/archives/{id}/pages/0?v={}", metadata.quick_hash)
    );
    for uri in [
        "/api/search?limit=0",
        "/api/search?limit=201",
        "/api/search?limit=oops",
        "/api/search?after=999999",
        "/api/search?sort=bad",
        "/api/search?q=pages:%3E20",
        "/api/search?q=-a*b",
        "/api/archives/invalid",
    ] {
        let (status, _, bytes) = request(&router, "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"].is_string());
    }
    for uri in [
        "/api",
        "/api/missing",
        "/api/archives/999999",
        "/api/archives/999999/pages",
    ] {
        let (status, _, bytes) = request(&router, "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"].is_string());
    }
}
