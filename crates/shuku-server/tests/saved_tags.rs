mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::{Value, json as value};

async fn change(router: &axum::Router, method: &str, tag: &str) -> (StatusCode, Value) {
    let (status, _, bytes) = request(
        router,
        method,
        "/api/saved-tags",
        Some(value!({ "tag": tag })),
        &[],
    )
    .await;
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn saved_tags_are_kept_in_order_lowercased_and_shared() {
    let fixture = Fixture::new();
    let app = fixture.app().await;
    let router = app.router();
    assert_eq!(json(&router, "/api/saved-tags").await, value!({"tags": []}));

    assert_eq!(
        change(&router, "PUT", " Artist:Alpha ").await.0,
        StatusCode::OK
    );
    assert_eq!(change(&router, "PUT", "long hair").await.0, StatusCode::OK);
    // 已经收藏过的再收藏一次不重复，没收藏过的取消也不报错。
    let (status, body) = change(&router, "PUT", "artist:alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, value!({"tags": ["artist:alpha", "long hair"]}));
    assert_eq!(change(&router, "DELETE", "missing").await.0, StatusCode::OK);

    let (_, body) = change(&router, "DELETE", "ARTIST:alpha").await;
    assert_eq!(body, value!({"tags": ["long hair"]}));
    // 存在库里，重新打开服务也还在。
    drop(router);
    drop(app);
    let router = fixture.app().await.router();
    assert_eq!(
        json(&router, "/api/saved-tags").await,
        value!({"tags": ["long hair"]})
    );
}

#[tokio::test]
async fn saved_tags_reject_empty_long_and_cross_site_writes() {
    let fixture = Fixture::new();
    let router = fixture.app().await.router();
    for tag in ["   ".to_owned(), "字".repeat(201)] {
        assert_eq!(
            change(&router, "PUT", &tag).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let (status, _, _) = request(
        &router,
        "PUT",
        "/api/saved-tags",
        Some(value!({"tag": "a"})),
        &[("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json(&router, "/api/saved-tags").await, value!({"tags": []}));
}

#[tokio::test]
async fn tag_parameters_match_whole_tags_and_combine_with_and() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz", "c.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = shuku_store::Store::open(fixture.database()).unwrap();
    let tag = |namespace: &str, value: &str| shuku_store::Tag {
        namespace: namespace.into(),
        value: value.into(),
    };
    // b 的“very long hair”含有“long hair”，搜索语句的子串匹配会把它也算进来。
    for (name, tags) in [
        ("a.cbz", vec![tag("artist", "alpha"), tag("", "long hair")]),
        (
            "b.cbz",
            vec![tag("artist", "alpha"), tag("", "very long hair")],
        ),
        ("c.cbz", vec![tag("", "long hair")]),
    ] {
        let id = store.find_by_path(name).unwrap().unwrap();
        store.replace_tags(id, &tags).unwrap();
    }
    app.refresh().await.unwrap();
    let router = app.router();
    let titles = |body: Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["title"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        titles(json(&router, "/api/search?tag=long%20hair").await),
        ["a", "c"]
    );
    assert_eq!(
        titles(json(&router, "/api/search?tag=Long%20Hair&tag=artist:alpha").await),
        ["a"]
    );
    assert_eq!(
        json(&router, "/api/random?tag=artist:alpha&tag=long%20hair").await["id"],
        json(&router, "/api/search?q=a").await["items"][0]["id"]
    );
}
