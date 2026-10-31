mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::Value;

fn titles(page: &Value) -> Vec<&str> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn volumes_are_grouped_into_series_and_collapse_in_the_library() {
    let fixture = Fixture::new();
    let png = fixture.archive("单本.cbz", 3, 4);
    // 作品一个文件夹、每卷一个只用卷号命名的散图文件夹。
    for volume in ["VOL_1", "VOL_2", "VOL_10"] {
        let folder = fixture.config.content_dir.join("漫画/朱丽叶").join(volume);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("001.png"), &png).unwrap();
    }
    // 文件名带作品名的卷和别的作品混在同一个文件夹里。
    fixture.archive_with_pages("散本/08/术士 第2卷.zip", 1);
    fixture.archive_with_pages("散本/08/术士 第1卷.zip", 1);
    // 只有一卷的不算系列。
    fixture.archive_with_pages("散本/08/[作者] 故事 Vol.2.zip", 1);
    let app = fixture.app().await;
    assert_eq!(app.scan().await.unwrap().added, 7);
    let router = app.router();

    let all = json(&router, "/api/search").await;
    assert_eq!(all["total"], 7);
    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 4);
    assert_eq!(
        titles(&grouped),
        ["[作者] 故事 Vol.2", "单本", "术士 第1卷", "朱丽叶 第1卷"]
    );
    let items = grouped["items"].as_array().unwrap();
    assert_eq!(items[0]["series"], Value::Null);
    let juliet = &items[3]["series"];
    assert_eq!(juliet["name"], "朱丽叶");
    assert_eq!(
        (juliet["volume"].as_u64(), juliet["volumes"].as_u64()),
        (Some(1), Some(3))
    );

    // 搜到其中一卷时显示的是这一卷。
    let found = json(&router, "/api/search?group=series&q=%E7%AC%AC2%E5%8D%B7").await;
    assert_eq!(titles(&found), ["术士 第2卷", "朱丽叶 第2卷"]);

    let series = json(&router, &format!("/api/series/{}", juliet["id"])).await;
    assert_eq!(series["name"], "朱丽叶");
    assert_eq!(
        titles(&series),
        ["朱丽叶 第1卷", "朱丽叶 第2卷", "朱丽叶 第10卷"]
    );

    let volumes: Vec<i64> = series["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    let middle = json(&router, &format!("/api/archives/{}", volumes[1])).await;
    assert_eq!(middle["series"]["previous"], volumes[0]);
    assert_eq!(middle["series"]["next"], volumes[2]);
    let last = json(&router, &format!("/api/archives/{}", volumes[2])).await;
    assert_eq!(last["series"]["next"], Value::Null);
    let single = items[0]["id"].as_i64().unwrap();
    let alone = json(&router, &format!("/api/archives/{single}")).await;
    assert_eq!(alone["series"], Value::Null);

    let (status, _, _) = request(&router, "GET", "/api/series/999", None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = request(&router, "GET", "/api/search?group=author", None, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn filename_parsing_still_sees_the_original_name_of_a_volume() {
    let fixture = Fixture::new();
    fixture.archive_with_pages("作品/VOL_1.zip", 1);
    fixture.archive_with_pages("作品/VOL_2.zip", 1);
    let app = fixture.app().await;
    let router = app.router();
    // 文件名里的下划线在解析前换成空格。
    let settings =
        serde_json::json!({"filename": {"auto": true, "regex": "^(?<artist>[A-Z]+) \\d+$"}});
    let (status, _, _) = request(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        Some(settings),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    app.scan().await.unwrap();

    let page = json(&router, "/api/search").await;
    assert_eq!(titles(&page), ["作品 第1卷", "作品 第2卷"]);
    let id = page["items"][0]["id"].as_i64().unwrap();
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    assert_eq!(
        detail["tags"],
        serde_json::json!([{"namespace": "artist", "values": ["VOL"]}])
    );
}

#[tokio::test]
async fn volumes_added_after_moving_a_series_folder_join_the_same_series() {
    let fixture = Fixture::new();
    let content = &fixture.config.content_dir;
    // 页数不同，各卷的快速指纹不同，移动后按指纹认回各自的记录。
    fixture.archive_with_pages("漫画/作品/01.zip", 1);
    fixture.archive_with_pages("漫画/作品/02.zip", 2);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    std::fs::create_dir(content.join("已整理")).unwrap();
    std::fs::rename(content.join("漫画/作品"), content.join("已整理/作品")).unwrap();
    assert_eq!(app.scan().await.unwrap().moved, 2);
    fixture.archive_with_pages("已整理/作品/03.zip", 3);
    assert_eq!(app.scan().await.unwrap().added, 1);

    let router = app.router();
    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 1);
    let series = &grouped["items"][0]["series"];
    assert_eq!(series["volumes"], 3);
    let volumes = json(&router, &format!("/api/series/{}", series["id"])).await;
    assert_eq!(titles(&volumes), ["作品 第1卷", "作品 第2卷", "作品 第3卷"]);
}
