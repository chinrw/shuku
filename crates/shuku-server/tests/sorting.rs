mod common;

use axum::{Router, http::StatusCode};
use common::{Fixture, json, request};
use shuku_store::{Store, Tag};

/// a：2 页、作者 beta；b：4 页、没有作者；c：1 页、作者 alpha。
async fn library() -> (Fixture, Router, i64) {
    let fixture = Fixture::new();
    for (name, pages) in [("a.cbz", 2), ("b.cbz", 4), ("c.cbz", 1)] {
        fixture.archive_with_pages(name, pages);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    let artist = |value: &str| {
        [Tag {
            namespace: "artist".into(),
            value: value.into(),
        }]
    };
    let id = |name: &str| store.find_by_path(name).unwrap().unwrap();
    let (a, c) = (id("a.cbz"), id("c.cbz"));
    store.replace_tags(a, &artist("beta")).unwrap();
    store.replace_tags(c, &artist("alpha")).unwrap();
    app.refresh().await.unwrap();
    (fixture, app.router(), c)
}

async fn titles(router: &Router, uri: &str) -> Vec<String> {
    let mut titles = Vec::new();
    let mut after: Option<i64> = None;
    // 每页一本，跟着游标翻完，顺带检查倒序时的游标。
    loop {
        let page_uri = match after {
            Some(after) => format!("{uri}&limit=1&after={after}"),
            None => format!("{uri}&limit=1"),
        };
        let page = json(router, &page_uri).await;
        for item in page["items"].as_array().unwrap() {
            titles.push(item["title"].as_str().unwrap().to_owned());
        }
        match page["next_after"].as_i64() {
            Some(next) => after = Some(next),
            None => return titles,
        }
    }
}

#[tokio::test]
async fn sorts_by_pages_and_namespace_in_either_direction() {
    let (_fixture, router, _) = library().await;
    let cases = [
        ("sort=title", ["a", "b", "c"]),
        ("sort=title&order=desc", ["c", "b", "a"]),
        ("sort=pages", ["c", "a", "b"]),
        ("sort=pages&order=desc", ["b", "a", "c"]),
        ("sort=ns:artist", ["c", "a", "b"]),
        ("sort=ns:Artist&order=desc", ["b", "a", "c"]),
        ("sort=ns:missing", ["a", "b", "c"]),
    ];
    for (params, expected) in cases {
        assert_eq!(
            titles(&router, &format!("/api/search?{params}")).await,
            expected,
            "{params}"
        );
    }
    let newest = titles(&router, "/api/search?sort=date").await;
    let mut oldest = titles(&router, "/api/search?sort=date&order=asc").await;
    oldest.reverse();
    assert_eq!(newest, oldest);
    assert_eq!(
        titles(&router, "/api/search?sort=date&order=desc").await,
        newest
    );

    for params in ["sort=size", "sort=ns:", "sort=ns:%20", "order=up"] {
        let (status, _, _) =
            request(&router, "GET", &format!("/api/search?{params}"), None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{params}");
    }
}

#[tokio::test]
async fn random_returns_a_match_or_404() {
    let (_fixture, router, alpha) = library().await;
    for _ in 0..5 {
        assert_eq!(
            json(&router, "/api/random?q=artist:alpha").await["id"],
            alpha
        );
    }
    let any = json(&router, "/api/random").await["id"].as_i64().unwrap();
    assert!(json(&router, &format!("/api/archives/{any}")).await["id"] == any);
    let (status, _, _) = request(&router, "GET", "/api/random?q=artist:nobody", None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = request(&router, "GET", "/api/random?filter=bogus", None, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
