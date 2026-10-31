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
    assert_eq!(titles(&volumes), ["作品 01", "作品 02", "作品 03"]);
}

#[tokio::test]
async fn books_scanned_before_series_existed_are_grouped_on_the_next_scan() {
    let fixture = Fixture::new();
    fixture.archive_with_pages("作品/01.zip", 1);
    fixture.archive_with_pages("作品/02.zip", 2);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    // 模拟系列功能之前入库的书：没有系列，标题是文件名；其中一本的标题改过。
    let connection = shuku_store::open_connection(fixture.database()).unwrap();
    connection
        .execute_batch(
            "UPDATE archive SET series_id = NULL, volume = NULL, title = '01' WHERE path = '作品/01.zip';
             UPDATE archive SET series_id = NULL, volume = NULL, title = '改过的标题' WHERE path = '作品/02.zip';
             DELETE FROM series;",
        )
        .unwrap();
    app.refresh().await.unwrap();
    let router = app.router();
    assert_eq!(json(&router, "/api/search?group=series").await["total"], 2);

    assert_eq!(app.scan().await.unwrap().unchanged, 2);
    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 1);
    let series = json(
        &router,
        &format!("/api/series/{}", grouped["items"][0]["series"]["id"]),
    )
    .await;
    assert_eq!(titles(&series), ["作品 01", "改过的标题"]);
}

#[tokio::test]
async fn regrouping_old_books_skips_hidden_folders_and_keeps_lrr_titles() {
    let fixture = Fixture::new();
    for path in [
        ".hidden/作品/01.zip",
        ".hidden/作品/02.zip",
        "公开/01.zip",
        "公开/02.zip",
    ] {
        fixture.archive_with_pages(path, 1 + path.len() % 3);
    }
    let app = fixture.app().await;
    let router = app.router();
    let scan_hidden = |enabled: bool| {
        let router = router.clone();
        async move {
            let (status, _, _) = request(
                &router,
                "PUT",
                "/api/admin/settings/scan",
                Some(serde_json::json!({"scan_hidden": enabled})),
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
    };
    scan_hidden(true).await;
    app.scan().await.unwrap();
    // 模拟系列功能之前入库的书；公开/01 是从 LRR 导入的，标题恰好就是文件名。
    let connection = shuku_store::open_connection(fixture.database()).unwrap();
    connection
        .execute_batch(
            "UPDATE archive SET series_id = NULL, volume = NULL,
                title = replace(replace(path, rtrim(path, replace(path, '/', '')), ''), '.zip', '');
             UPDATE archive SET lrr_id = 'lrr' WHERE path = '公开/01.zip';
             DELETE FROM series;",
        )
        .unwrap();
    scan_hidden(false).await;
    app.scan().await.unwrap();

    let titles_by_path: Vec<(String, String, bool)> = connection
        .prepare("SELECT path, title, series_id IS NOT NULL FROM archive ORDER BY path")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        titles_by_path,
        [
            (".hidden/作品/01.zip".into(), "01".into(), false),
            (".hidden/作品/02.zip".into(), "02".into(), false),
            ("公开/01.zip".into(), "01".into(), true),
            ("公开/02.zip".into(), "公开 02".into(), true),
        ]
    );
}

/// 给 `path` 的归档加一个写明系列和序号的 ComicInfo.xml。
fn add_comicinfo(fixture: &Fixture, path: &str, series: &str, number: u32) {
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(fixture.config.content_dir.join(path))
        .unwrap();
    let mut zip = zip::ZipWriter::new_append(file).unwrap();
    zip.start_file("ComicInfo.xml", zip::write::SimpleFileOptions::default())
        .unwrap();
    let xml = format!("<ComicInfo><Series>{series}</Series><Number>{number}</Number></ComicInfo>");
    std::io::Write::write_all(&mut zip, xml.as_bytes()).unwrap();
    zip.finish().unwrap();
}

async fn enable_comicinfo(router: &axum::Router) {
    let (status, _, _) = request(
        router,
        "PUT",
        "/api/admin/settings/metadata",
        Some(serde_json::json!({"comicinfo": {"auto": true}})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn comicinfo_series_and_number_group_volumes_across_folders() {
    let fixture = Fixture::new();
    for (path, number, pages) in [("a/one.zip", 2, 1), ("b/two.zip", 1, 2)] {
        fixture.archive_with_pages(path, pages);
        add_comicinfo(&fixture, path, "Some Work", number);
    }
    let app = fixture.app().await;
    let router = app.router();
    enable_comicinfo(&router).await;
    app.scan().await.unwrap();

    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 1);
    let series = &grouped["items"][0]["series"];
    assert_eq!(series["name"], "Some Work");
    let volumes = json(&router, &format!("/api/series/{}", series["id"])).await;
    assert_eq!(titles(&volumes), ["two", "one"]);
}

#[tokio::test]
async fn moving_a_comicinfo_series_keeps_grouping_by_its_name() {
    let fixture = Fixture::new();
    let content = &fixture.config.content_dir;
    for (path, number) in [("旧/01.zip", 1), ("旧/02.zip", 2)] {
        fixture.archive_with_pages(path, number as usize);
        add_comicinfo(&fixture, path, "Some Work", number);
    }
    let app = fixture.app().await;
    let router = app.router();
    enable_comicinfo(&router).await;
    app.scan().await.unwrap();
    std::fs::rename(content.join("旧"), content.join("新")).unwrap();
    assert_eq!(app.scan().await.unwrap().moved, 2);
    fixture.archive_with_pages("别处/03.zip", 3);
    add_comicinfo(&fixture, "别处/03.zip", "Some Work", 3);
    app.scan().await.unwrap();

    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 1);
    assert_eq!(grouped["items"][0]["series"]["volumes"], 3);
}

#[tokio::test]
async fn comicinfo_follows_a_folder_already_grouped_by_path_and_its_numbers_keep_names() {
    let fixture = Fixture::new();
    fixture.archive_with_pages("作品/01.zip", 1);
    fixture.archive_with_pages("作品/02.zip", 2);
    let app = fixture.app().await;
    let router = app.router();
    app.scan().await.unwrap();
    // 按路径归组之后才开 ComicInfo，新卷跟着原来的系列。
    enable_comicinfo(&router).await;
    fixture.archive_with_pages("作品/03.zip", 3);
    add_comicinfo(&fixture, "作品/03.zip", "作品", 3);
    // 另一部作品第一次入库就有 ComicInfo，序号和名字里的卷号不一样时标题不写卷号。
    fixture.archive_with_pages("另一部/第01卷.zip", 4);
    add_comicinfo(&fixture, "另一部/第01卷.zip", "另一部", 5);
    fixture.archive_with_pages("另一部/第02卷.zip", 5);
    add_comicinfo(&fixture, "另一部/第02卷.zip", "另一部", 6);
    app.scan().await.unwrap();

    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 2);
    let volumes: Vec<_> = grouped["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["series"]["name"].clone(),
                item["series"]["volumes"].clone(),
            )
        })
        .collect();
    assert_eq!(
        volumes,
        [
            (Value::from("作品"), Value::from(3)),
            (Value::from("另一部"), Value::from(2))
        ]
    );
    let other = json(
        &router,
        &format!("/api/series/{}", grouped["items"][1]["series"]["id"]),
    )
    .await;
    assert_eq!(titles(&other), ["第01卷", "第02卷"]);
}

#[tokio::test]
async fn books_imported_from_lrr_ignore_comicinfo_but_still_group_by_path() {
    let fixture = Fixture::new();
    for (path, number) in [("作品/01.zip", 1), ("作品/02.zip", 2)] {
        fixture.archive_with_pages(path, number as usize);
        add_comicinfo(&fixture, path, "Some Work", number);
    }
    std::fs::create_dir_all(&fixture.config.data_dir).unwrap();
    let mut store = shuku_store::Store::open(fixture.database()).unwrap();
    store
        .write(|writer| {
            for path in ["作品/01.zip", "作品/02.zip"] {
                writer.stage_lrr_pending(
                    path,
                    &shuku_store::LrrPending {
                        archive: shuku_store::LrrArchive {
                            lrr_id: path.repeat(4),
                            title: format!("LRR {path}"),
                            summary: String::new(),
                            date_added: None,
                            source_url: None,
                            is_new: false,
                            tags: Vec::new(),
                        },
                        progress: None,
                        categories: Vec::new(),
                        tankoubons: Vec::new(),
                    },
                )?;
            }
            Ok::<_, shuku_store::Error>(())
        })
        .unwrap();
    drop(store);
    let app = fixture.app().await;
    let router = app.router();
    enable_comicinfo(&router).await;
    app.scan().await.unwrap();

    let grouped = json(&router, "/api/search?group=series").await;
    assert_eq!(grouped["total"], 1);
    assert_eq!(grouped["items"][0]["series"]["name"], "作品");
    assert_eq!(grouped["items"][0]["title"], "LRR 作品/01.zip");
}
