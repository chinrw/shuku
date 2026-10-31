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
    // 搜索读内存快照，进度要等快照刷新后才出现。
    assert_eq!(
        json(&router, "/api/search").await["items"][0]["progress"],
        Value::Null
    );
    app.refresh().await.unwrap();
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

// 暂停的时钟在运行时空闲时自动快进，10 秒的刷新延迟不用真的等。
#[tokio::test(start_paused = true)]
async fn search_shows_new_progress_once_the_delayed_refresh_runs() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let started = tokio::time::Instant::now();
    let (status, _, _) = request(
        &router,
        "PUT",
        &format!("/api/archives/{id}/progress"),
        Some(value!({"page": 1, "client_time": 1000})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    loop {
        let progress = json(&router, "/api/search").await["items"][0]["progress"].clone();
        if progress != Value::Null {
            assert_eq!(progress, value!({"page": 1, "client_time": 1000}));
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(60));
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    assert!(started.elapsed() >= std::time::Duration::from_secs(10));
}

#[tokio::test]
async fn concurrent_progress_reports_commit_together_and_fail_one_by_one() {
    let fixture = Fixture::new();
    let names: Vec<_> = (0..8).map(|number| format!("{number}.cbz")).collect();
    for name in &names {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let store = Store::open(fixture.database()).unwrap();
    let ids: Vec<_> = names
        .iter()
        .map(|name| store.find_by_path(name).unwrap().unwrap())
        .collect();
    // 第一条立即提交，其余在同一秒里到达，合并进下一次提交。越界的那条单独失败。
    let reports = ids.clone().into_iter().enumerate().map(|(number, id)| {
        let router = router.clone();
        let page = if number == 3 { 5 } else { 1 };
        async move {
            request(
                &router,
                "PUT",
                &format!("/api/archives/{id}/progress"),
                Some(value!({"page": page, "client_time": 1000})),
                &[],
            )
            .await
            .0
        }
    });
    let statuses = futures_join_all(reports).await;
    for (number, status) in statuses.into_iter().enumerate() {
        let expected = if number == 3 {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::OK
        };
        assert_eq!(status, expected, "{number}");
    }
    for (number, id) in ids.iter().enumerate() {
        let progress = store.archive(*id).unwrap().unwrap().progress;
        assert_eq!(
            progress.map(|progress| progress.page),
            (number != 3).then_some(1)
        );
    }
}

async fn futures_join_all<F: std::future::Future<Output = StatusCode> + Send + 'static>(
    futures: impl Iterator<Item = F>,
) -> Vec<StatusCode> {
    let tasks: Vec<_> = futures.map(tokio::spawn).collect();
    let mut statuses = Vec::new();
    for task in tasks {
        statuses.push(task.await.unwrap());
    }
    statuses
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
    assert_eq!(first["items"][0]["date_added"], 1);
    assert_eq!(first["items"][0]["source_url"], "https://example.org/book");
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
        "/api/search?q=pages:!%3D20",
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

#[tokio::test]
async fn facets_and_tag_suggestions_count_archives_per_tag() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz", "c.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    let tag = |namespace: &str, value: &str| Tag {
        namespace: namespace.into(),
        value: value.into(),
    };
    for (path, tags) in [
        (
            "a.cbz",
            vec![tag("artist", "Alpha"), tag("language", "chinese")],
        ),
        (
            "b.cbz",
            vec![tag("artist", "Alpha"), tag("language", "japanese")],
        ),
        (
            "c.cbz",
            vec![tag("artist", "albert"), tag("language", "chinese")],
        ),
    ] {
        let id = store.find_by_path(path).unwrap().unwrap();
        store.replace_tags(id, &tags).unwrap();
    }
    app.scan().await.unwrap();
    let router = app.router();

    assert_eq!(
        json(&router, "/api/facets?q=language:chinese$").await,
        value!({"tags": [
            {"tag": "language:chinese", "count": 2},
            {"tag": "artist:albert", "count": 1},
            {"tag": "artist:alpha", "count": 1},
        ]})
    );
    assert_eq!(
        json(&router, "/api/facets?limit=1").await,
        value!({"tags": [{"tag": "artist:alpha", "count": 2}]})
    );
    assert_eq!(
        json(&router, "/api/tags?prefix=Al").await,
        value!({"tags": [
            {"tag": "artist:alpha", "count": 2},
            {"tag": "artist:albert", "count": 1},
        ]})
    );
    assert_eq!(
        json(&router, "/api/tags?prefix=").await,
        value!({"tags": []})
    );
    for uri in [
        "/api/facets?limit=0",
        "/api/facets?limit=101",
        "/api/facets?q=pages:!%3D20",
        "/api/tags",
        "/api/tags?prefix=a&limit=0",
    ] {
        let (status, _, bytes) = request(&router, "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"].is_string());
    }
}

#[tokio::test]
async fn escaped_wildcards_match_tags_literally_in_search() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    for (path, value) in [("a.cbz", "foo_bar"), ("b.cbz", "fooxbar")] {
        let id = store.find_by_path(path).unwrap().unwrap();
        store
            .replace_tags(
                id,
                &[Tag {
                    namespace: "artist".into(),
                    value: value.into(),
                }],
            )
            .unwrap();
    }
    let underscore = store.find_by_path("a.cbz").unwrap().unwrap();
    app.scan().await.unwrap();
    let router = app.router();

    // 网页端点击标签时发出 `"artist:foo\_bar"`，反斜杠和引号都经过 URL 编码。
    let included = json(&router, "/api/search?q=%22artist%3Afoo%5C_bar%22").await;
    assert_eq!(included["total"], 1);
    assert_eq!(included["degraded"], false);
    assert_eq!(included["items"][0]["id"], underscore);

    let excluded = json(&router, "/api/search?q=-%22artist%3Afoo%5C_bar%22").await;
    assert_eq!(excluded["total"], 1);
    assert_ne!(excluded["items"][0]["id"], underscore);

    // 不转义时 `_` 仍是通配符，两本都会匹配并标记降级。
    let degraded = json(&router, "/api/search?q=%22artist%3Afoo_bar%22").await;
    assert_eq!(degraded["total"], 2);
    assert_eq!(degraded["degraded"], true);
}

#[tokio::test]
async fn recent_lists_read_archives_newest_first_and_validates_limit() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz", "c.cbz", "unread.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    assert_eq!(json(&router, "/api/recent").await, value!({"items": []}));

    let mut store = Store::open(fixture.database()).unwrap();
    let id = |name: &str| store.find_by_path(name).unwrap().unwrap();
    let (a, b, c) = (id("a.cbz"), id("b.cbz"), id("c.cbz"));
    // b 读得最近，a 其次，c 最早；没读过的不出现。
    for (archive, page, time) in [(a, 1, 2000), (b, 0, 3000), (c, 1, 1000)] {
        store.write_progress(archive, page, time).unwrap();
    }
    app.refresh().await.unwrap();

    let ids = |body: Value| -> Vec<i64> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_i64().unwrap())
            .collect()
    };
    let all = json(&router, "/api/recent").await;
    assert_eq!(ids(all.clone()), [b, a, c]);
    assert_eq!(
        all["items"][0]["progress"],
        value!({"page": 0, "client_time": 3000})
    );
    assert!(all["items"][0]["cover_url"].is_string());
    assert_eq!(ids(json(&router, "/api/recent?limit=2").await), [b, a]);
    for uri in [
        "/api/recent?limit=0",
        "/api/recent?limit=101",
        "/api/recent?limit=x",
    ] {
        assert_eq!(
            request(&router, "GET", uri, None, &[]).await.0,
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn categories_are_listed_pinned_first_and_filter_search() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz", "c.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    let id = |store: &Store, name: &str| store.find_by_path(name).unwrap().unwrap();
    let (a, b, c) = (
        id(&store, "a.cbz"),
        id(&store, "b.cbz"),
        id(&store, "c.cbz"),
    );
    store
        .replace_tags(
            c,
            &[Tag {
                namespace: "artist".into(),
                value: "alpha".into(),
            }],
        )
        .unwrap();
    for (lrr_id, name, search, pinned, archives) in [
        ("SET_1", "收藏", "", false, vec![a, b]),
        ("SET_2", "动态", "artist:alpha", true, vec![]),
        ("SET_3", "空的", "", true, vec![]),
    ] {
        store
            .import_lrr_category(&shuku_store::LrrCategory {
                lrr_id: lrr_id.into(),
                name: name.into(),
                search: search.into(),
                pinned,
                archives,
            })
            .unwrap();
    }
    app.refresh().await.unwrap();
    let router = app.router();

    let listed = json(&router, "/api/categories").await;
    let summary: Vec<(String, bool, bool, u64)> = listed["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| {
            (
                category["name"].as_str().unwrap().to_owned(),
                category["pinned"].as_bool().unwrap(),
                category["dynamic"].as_bool().unwrap(),
                category["count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("动态".to_owned(), true, true, 1),
            ("空的".to_owned(), true, false, 0),
            ("收藏".to_owned(), false, false, 2),
        ]
    );
    let category_id = |name: &str| {
        listed["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["name"] == name)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    let ids = |body: Value| -> Vec<i64> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_i64().unwrap())
            .collect()
    };
    let favorites = category_id("收藏");
    assert_eq!(
        ids(json(&router, &format!("/api/search?category={favorites}")).await),
        [a, b]
    );
    // 分类和搜索语句同时生效。
    assert_eq!(
        ids(json(
            &router,
            &format!("/api/search?category={favorites}&q=artist:alpha")
        )
        .await),
        Vec::<i64>::new()
    );
    let dynamic = category_id("动态");
    assert_eq!(
        ids(json(&router, &format!("/api/search?category={dynamic}")).await),
        [c]
    );
    assert_eq!(
        json(&router, &format!("/api/facets?category={dynamic}")).await["tags"],
        value!([{"tag": "artist:alpha", "count": 1}])
    );
    for uri in ["/api/search?category=999999", "/api/facets?category=999999"] {
        assert_eq!(
            request(&router, "GET", uri, None, &[]).await.0,
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }
}
