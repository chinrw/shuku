mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::{Value, json as value};

fn ids(body: &Value) -> Vec<i64> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect()
}

async fn status(router: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> StatusCode {
    request(router, method, uri, body, &[]).await.0
}

#[tokio::test]
async fn adding_to_a_category_shows_up_in_search_and_detail_immediately() {
    let fixture = Fixture::new();
    for name in ["a.cbz", "b.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let all = ids(&json(&router, "/api/search").await);

    let (code, _, body) = request(
        &router,
        "POST",
        "/api/categories",
        Some(value!({"name": "  收藏  "})),
        &[],
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    let favorites = serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    let member = format!("/api/categories/{favorites}/archives/{}", all[1]);
    for _ in 0..2 {
        assert_eq!(
            status(&router, "PUT", &member, None).await,
            StatusCode::NO_CONTENT
        );
    }

    // 不等快照的延迟刷新：分类列表、按分类搜索和详情都已经是新的。
    let listed = json(&router, "/api/categories").await;
    assert_eq!(
        listed["categories"],
        value!([{
            "id": favorites, "name": "收藏", "pinned": false, "dynamic": false, "count": 1,
            "bookmark": true, "duplicates": false
        }])
    );
    let search = json(&router, &format!("/api/search?category={favorites}")).await;
    assert_eq!(ids(&search), [all[1]]);
    let detail = json(&router, &format!("/api/archives/{}", all[1])).await;
    assert_eq!(detail["categories"], value!([favorites]));

    assert_eq!(
        status(&router, "DELETE", &member, None).await,
        StatusCode::NO_CONTENT
    );
    let search = json(&router, &format!("/api/search?category={favorites}")).await;
    assert!(ids(&search).is_empty());
    let detail = json(&router, &format!("/api/archives/{}", all[1])).await;
    assert_eq!(detail["categories"], value!([]));
}

#[tokio::test]
async fn categories_are_renamed_pinned_and_deleted() {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let archive = ids(&json(&router, "/api/search").await)[0];

    let mut created = Vec::new();
    for body in [
        value!({"name": "作者", "search": "artist:alpha"}),
        value!({"name": "收藏"}),
    ] {
        let (code, _, bytes) = request(&router, "POST", "/api/categories", Some(body), &[]).await;
        assert_eq!(code, StatusCode::CREATED);
        created.push(
            serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
                .as_i64()
                .unwrap(),
        );
    }
    let (dynamic, favorites) = (created[0], created[1]);
    let uri = format!("/api/categories/{favorites}");
    let change = value!({"name": "最爱", "pinned": true});
    assert_eq!(
        status(&router, "PATCH", &uri, Some(change)).await,
        StatusCode::NO_CONTENT
    );
    let names: Vec<_> = json(&router, "/api/categories").await["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| (category["name"].clone(), category["pinned"].clone()))
        .collect();
    assert_eq!(
        names,
        [
            (value!("最爱"), value!(true)),
            (value!("作者"), value!(false))
        ]
    );

    assert_eq!(
        status(&router, "DELETE", &uri, None).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&router, "DELETE", &uri, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        json(&router, "/api/categories").await["categories"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let dynamic_member = format!("/api/categories/{dynamic}/archives/{archive}");
    assert_eq!(
        status(&router, "PUT", &dynamic_member, None).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn category_writes_reject_bad_input_and_cross_site_requests() {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let archive = ids(&json(&router, "/api/search").await)[0];

    for body in [
        value!({"name": "   "}),
        value!({"name": "字".repeat(101)}),
        value!({"name": "坏语句", "search": "pages:!=20"}),
        value!({"search": "artist:alpha"}),
    ] {
        let code = status(&router, "POST", "/api/categories", Some(body.clone())).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    }
    let (_, _, bytes) = request(
        &router,
        "POST",
        "/api/categories",
        Some(value!({"name": "收藏"})),
        &[],
    )
    .await;
    let favorites = serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
        .as_i64()
        .unwrap();
    let uri = format!("/api/categories/{favorites}");
    let to_dynamic = value!({"search": "artist:alpha"});
    assert_eq!(
        status(&router, "PATCH", &uri, Some(to_dynamic)).await,
        StatusCode::BAD_REQUEST
    );
    let missing = format!("/api/categories/{favorites}/archives/{}", archive + 100);
    assert_eq!(
        status(&router, "PUT", &missing, None).await,
        StatusCode::NOT_FOUND
    );
    let unknown = format!("/api/categories/{}/archives/{archive}", favorites + 100);
    assert_eq!(
        status(&router, "PUT", &unknown, None).await,
        StatusCode::NOT_FOUND
    );

    let member = format!("/api/categories/{favorites}/archives/{archive}");
    let (code, _, _) = request(
        &router,
        "PUT",
        &member,
        None,
        &[("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let search = json(&router, &format!("/api/search?category={favorites}")).await;
    assert!(ids(&search).is_empty());
}

#[tokio::test]
async fn favorites_is_the_bookmark_and_lrr_duplicates_are_flagged() {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    // 动态的 Favorites 不能当书签：书签要能加进、移出。编号小的同名静态分类优先。
    for body in [
        value!({"name": "Favorites", "search": "artist:alpha"}),
        value!({"name": "favorites"}),
        value!({"name": "收藏"}),
        value!({"name": "DuplicateArchives"}),
        value!({"name": "DuplicateArchives", "search": "artist:alpha"}),
        value!({"name": "其他"}),
    ] {
        let (code, _, _) = request(&router, "POST", "/api/categories", Some(body), &[]).await;
        assert_eq!(code, StatusCode::CREATED);
    }
    let listed = json(&router, "/api/categories").await;
    let flags: Vec<_> = listed["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| {
            (
                category["name"].as_str().unwrap().to_owned(),
                category["bookmark"].as_bool().unwrap(),
                category["duplicates"].as_bool().unwrap(),
            )
        })
        .collect();
    let flag = |name: &str| {
        flags
            .iter()
            .find(|(category, _, _)| category == name)
            .map(|(_, bookmark, duplicates)| (*bookmark, *duplicates))
            .unwrap()
    };
    assert_eq!(flag("Favorites"), (false, false));
    assert_eq!(flag("favorites"), (true, false));

    // LRR 默认的书签分类叫“🔖 Favorites”，前面的 emoji 不影响识别。
    let fixture = Fixture::new();
    let router = fixture.app().await.router();
    let (code, _, _) = request(
        &router,
        "POST",
        "/api/categories",
        Some(value!({"name": "🔖 Favorites"})),
        &[],
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    assert_eq!(
        json(&router, "/api/categories").await["categories"][0]["bookmark"],
        true
    );
    assert_eq!(flag("收藏"), (false, false));
    // 同名的只有编号最小的静态分类被认出，书库页隐藏的和控制台列出的是同一个。
    let duplicates: Vec<_> = flags
        .iter()
        .filter(|(name, _, _)| name == "DuplicateArchives")
        .map(|(_, _, duplicates)| *duplicates)
        .collect();
    assert_eq!(duplicates.iter().filter(|flag| **flag).count(), 1);
    assert_eq!(flag("DuplicateArchives"), (false, true));
    assert_eq!(flag("其他"), (false, false));
}
