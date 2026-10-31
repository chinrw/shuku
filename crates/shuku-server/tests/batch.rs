mod common;

use axum::{Router, http::StatusCode};
use common::{Fixture, json, request};
use serde_json::{Value, json as value};

async fn post(router: &Router, body: Value) -> (StatusCode, Value) {
    let (status, _, bytes) = request(router, "POST", "/api/batch", Some(body), &[]).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn titles(router: &Router, uri: &str) -> Vec<String> {
    json(router, uri).await["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap().to_owned())
        .collect()
}

async fn favorites(router: &Router) -> i64 {
    let (status, _, body) = request(
        router,
        "POST",
        "/api/categories",
        Some(value!({"name": "收藏"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn tags_and_categories_change_for_ids_or_a_search_in_one_go() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz", "c.cbz"] {
        fixture.archive_with_pages(name, 1);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let ids: Vec<i64> = json(&router, "/api/search").await["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    let favorites = favorites(&router).await;

    let (status, body) = post(
        &router,
        value!({"ids": [ids[0], ids[1], ids[0]], "add_tags": ["artist:x", "todo"],
                "add_to_category": favorites}),
    )
    .await;
    assert_eq!((status, body), (StatusCode::OK, value!({"archives": 2})));
    // 分类马上生效，标签等快照重建。
    assert_eq!(
        titles(&router, &format!("/api/search?category={favorites}")).await,
        ["a", "b"]
    );
    app.refresh().await.unwrap();
    assert_eq!(titles(&router, "/api/search?q=artist:x$").await, ["a", "b"]);

    // 按搜索条件选：在收藏里且带 todo 的，去掉 todo、移出收藏。
    let (status, body) = post(
        &router,
        value!({"search": {"q": "todo$", "category": favorites},
                "remove_tags": ["todo"], "remove_from_category": favorites}),
    )
    .await;
    assert_eq!((status, body), (StatusCode::OK, value!({"archives": 2})));
    app.refresh().await.unwrap();
    assert!(titles(&router, "/api/search?q=todo$").await.is_empty());
    assert!(
        titles(&router, &format!("/api/search?category={favorites}"))
            .await
            .is_empty()
    );
    assert_eq!(titles(&router, "/api/search?q=artist:x$").await, ["a", "b"]);
}

#[tokio::test]
async fn a_bad_batch_changes_nothing() {
    let fixture = Fixture::new();
    fixture.archive_with_pages("a.cbz", 1);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();

    let (status, _) = post(&router, value!({"ids": [id, id + 100], "add_tags": ["x"]})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // 一本都没选中时，分类不存在照样报错。
    for body in [
        value!({"ids": [], "add_to_category": 999_999}),
        value!({"search": {"q": "nothing matches"}, "remove_from_category": 999_999}),
    ] {
        let (status, _) = post(&router, body.clone()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    }
    let (status, _, created) = request(
        &router,
        "POST",
        "/api/categories",
        Some(value!({"name": "动态", "search": "artist:a"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let dynamic = serde_json::from_slice::<Value>(&created).unwrap()["id"].clone();
    let (status, _) = post(&router, value!({"ids": [], "add_to_category": dynamic})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &router,
        value!({"ids": [id], "add_tags": ["x"], "add_to_category": 999_999}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    app.refresh().await.unwrap();
    assert!(titles(&router, "/api/search?q=x$").await.is_empty());

    for body in [
        value!({"add_tags": ["x"]}),
        value!({"ids": [id], "search": {"q": ""}, "add_tags": ["x"]}),
        value!({"ids": [id]}),
        value!({"ids": [id], "add_tags": ["a,b"]}),
        value!({"ids": [id], "add_tags": ["x"], "extra": 1}),
        value!({"search": {"filter": "bogus"}, "add_tags": ["x"]}),
    ] {
        let (status, _) = post(&router, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, _, _) = request(
        &router,
        "POST",
        "/api/batch",
        Some(value!({"ids": [id], "add_tags": ["x"]})),
        &[("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
