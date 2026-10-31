mod common;

use axum::{Router, http::StatusCode};
use common::{Fixture, json, request};
use serde_json::json as value;
use shuku_store::{Store, Tag};

/// 测试用的查询参数编码，只处理这里用到的字符。
fn encode(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            ':' | '>' | '<' | '=' | ',' | ' ' | '!' => format!("%{:02X}", ch as u32),
            _ => ch.to_string(),
        })
        .collect()
}

async fn ids(router: &Router, uri: &str) -> Vec<String> {
    json(router, uri).await["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap().to_owned())
        .collect()
}

/// a：2 页、读完、有标签、新；b：4 页、读到第 1 页、不新；c：1 页、没读过、新。
async fn library() -> (Fixture, Router) {
    let fixture = Fixture::new();
    for (name, pages) in [("a.cbz", 2), ("b.cbz", 4), ("c.cbz", 1)] {
        fixture.archive_with_pages(name, pages);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    let id = |name: &str| store.find_by_path(name).unwrap().unwrap();
    let (a, b) = (id("a.cbz"), id("b.cbz"));
    store
        .replace_tags(
            a,
            &[Tag {
                namespace: "artist".into(),
                value: "alpha".into(),
            }],
        )
        .unwrap();
    store.write_progress(a, 1, 1_000).unwrap();
    store.write_progress(b, 0, 1_000).unwrap();
    shuku_store::open_connection(fixture.database())
        .unwrap()
        .execute("UPDATE archive SET is_new = 0 WHERE id = ?1", [b])
        .unwrap();
    app.refresh().await.unwrap();
    let router = app.router();
    (fixture, router)
}

#[tokio::test]
async fn pages_and_read_conditions_filter_search_results() {
    let (_fixture, router) = library().await;
    let cases = [
        ("pages:>2", vec!["b"]),
        ("pages:<=2", vec!["a", "c"]),
        ("-pages:2", vec!["b", "c"]),
        ("read:0", vec!["c"]),
        ("read:>0", vec!["a", "b"]),
        ("read:1", vec!["b"]),
        ("-read:1", vec!["a", "c"]),
        ("read:>=2", vec!["a"]),
        ("read:<2", vec!["b", "c"]),
        ("-read:<2", vec!["a"]),
        ("artist:alpha, read:>0", vec!["a"]),
    ];
    for (q, expected) in cases {
        let query = encode(q);
        assert_eq!(
            ids(&router, &format!("/api/search?q={query}")).await,
            expected,
            "{q}"
        );
    }
    let (status, _, _) = request(&router, "GET", "/api/search?q=pages:!%3D2", None, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn quick_filters_combine_with_each_other_and_the_query() {
    let (_fixture, router) = library().await;
    let cases = [
        ("new", vec!["c"]),
        ("unread", vec!["c"]),
        ("reading", vec!["b"]),
        ("finished", vec!["a"]),
        ("untagged", vec!["b", "c"]),
        ("new,untagged", vec!["c"]),
        ("reading, untagged", vec!["b"]),
        ("finished,untagged", vec![]),
        ("unfinished", vec!["b", "c"]),
        ("new,unfinished", vec!["c"]),
        ("", vec!["a", "b", "c"]),
    ];
    for (filter, expected) in cases {
        let filter = encode(filter);
        assert_eq!(
            ids(&router, &format!("/api/search?filter={filter}")).await,
            expected,
            "{filter}"
        );
    }
    assert_eq!(
        json(&router, "/api/facets?filter=finished").await["tags"],
        value!([{"tag": "artist:alpha", "count": 1}])
    );
    let (status, _, _) = request(&router, "GET", "/api/search?filter=bogus", None, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn dynamic_categories_can_use_read_conditions() {
    let (_fixture, router) = library().await;
    let (status, _, body) = request(
        &router,
        "POST",
        "/api/categories",
        Some(value!({"name": "没读过", "search": "read:0"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        json(&router, "/api/categories").await["categories"][0]["count"],
        1
    );
    assert_eq!(
        ids(&router, &format!("/api/search?category={id}")).await,
        ["c"]
    );
}
