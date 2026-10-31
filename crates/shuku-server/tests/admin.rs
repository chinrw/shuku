mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::Value;
use shuku_store::{Store, Tag};
use std::time::{Duration, Instant};
use tracing_subscriber::layer::SubscriberExt;

/// 每个测试文件是单独的进程，全局订阅者只在这里装一次，扫描线程里的日志也能收到。
fn capture_logs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(shuku_server::log_layer());
        tracing::subscriber::set_global_default(subscriber).unwrap();
    });
}

async fn wait_until_idle(router: &axum::Router) -> Value {
    let started = Instant::now();
    loop {
        let status = json(router, "/api/admin/status").await;
        if status["scan"]["running"] == false && status["hash"]["running"] == false {
            return status;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "任务没有结束");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn status_reports_library_counts_snapshot_and_the_last_scan() {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    fixture.archive("b.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let mut store = Store::open(fixture.database()).unwrap();
    let a = store.find_by_path("a.cbz").unwrap().unwrap();
    store.write_progress(a, 0, 1).unwrap();
    app.refresh().await.unwrap();

    let status = json(&router, "/api/admin/status").await;
    assert_eq!(status["server"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(status["server"]["started_at"].as_i64().unwrap() > 0);
    let library = &status["library"];
    assert_eq!(library["archives"], 2);
    assert_eq!(library["pages"], 4);
    assert_eq!(library["with_progress"], 1);
    // 进度的翻页时间是 1970 年，不算最近 7 天。
    assert_eq!(library["read_last_week"], 0);
    assert_eq!(library["lrr_pending"], 0);
    assert!(status["storage"]["database_bytes"].as_u64().unwrap() > 0);
    assert_eq!(status["snapshot"]["archives"], 2);
    let scan = &status["scan"];
    assert_eq!(scan["running"], false);
    assert_eq!(scan["total"], 2);
    assert_eq!(scan["processed"], 2);
    assert_eq!(scan["report"]["added"], 2);
    assert_eq!(scan["error"], Value::Null);
}

#[tokio::test]
async fn triggered_scan_runs_in_the_background_then_hashes() {
    let fixture = Fixture::new();
    let app = fixture.app().await;
    let router = app.router();
    fixture.archive("new.cbz", 10, 20);
    let (status, _, body) = request(&router, "POST", "/api/admin/scan", None, &[]).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&body)
    );

    // 扫描结束到补算开始之间有一小段两者都不在运行，所以等补算结束的时间出现。
    let started = Instant::now();
    let idle = loop {
        let status = json(&router, "/api/admin/status").await;
        if status["hash"]["finished_at"].is_i64() {
            break status;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "补算没有结束");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(idle["scan"]["report"]["added"], 1);
    assert_eq!(idle["hash"]["report"]["hashed"], 1);
    assert_eq!(idle["library"]["hashed"], 1);
    // 扫描结束时重建了快照，搜索立即能看到新文件。
    assert_eq!(json(&router, "/api/search").await["total"], 1);

    let (status, _, _) = request(&router, "POST", "/api/admin/hash", None, &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let idle = wait_until_idle(&router).await;
    assert_eq!(idle["hash"]["total"], 0);
}

#[tokio::test]
async fn issues_list_failed_skipped_and_missing_files() {
    capture_logs();
    let fixture = Fixture::new();
    fixture.archive("kept.cbz", 10, 20);
    fixture.archive("gone.cbz", 10, 20);
    let content = &fixture.config.content_dir;
    std::fs::write(content.join("notes.txt"), "不是归档").unwrap();
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let gone = Store::open(fixture.database())
        .unwrap()
        .find_by_path("gone.cbz")
        .unwrap()
        .unwrap();
    std::fs::remove_file(content.join("gone.cbz")).unwrap();
    // zip 开头但内容是坏的，扫描失败而不是跳过。
    std::fs::write(content.join("broken.cbz"), b"PK\x03\x04 not really a zip").unwrap();
    let report = app.scan().await.unwrap();
    assert_eq!((report.failed, report.skipped), (1, 1));

    let router = app.router();
    let issues = json(&router, "/api/admin/issues").await;
    assert_eq!(issues["scanning"], false);
    assert_eq!(issues["failed"]["total"], 1);
    assert_eq!(issues["failed"]["items"][0]["path"], "broken.cbz");
    assert!(issues["failed"]["items"][0]["error"].is_string());
    assert_eq!(
        issues["skipped"]["items"],
        serde_json::json!([{"path": "notes.txt"}])
    );
    assert_eq!(issues["unreadable"]["total"], 0);
    assert_eq!(
        issues["missing"]["items"],
        serde_json::json!([{"id": gone, "path": "gone.cbz", "title": "gone"}])
    );
    let status = json(&router, "/api/admin/status").await;
    assert_eq!(status["scan"]["issues"]["missing"], 1);

    let logs = json(&router, "/api/admin/logs?level=warn").await;
    assert_eq!(logs["started_at"], status["server"]["started_at"]);
    let entries = logs["entries"].as_array().unwrap();
    let failure = entries
        .iter()
        .find(|entry| entry["message"].as_str().unwrap().contains("broken.cbz"))
        .expect("日志里有扫描失败的记录");
    assert_eq!(failure["level"], "warn");
    assert!(
        failure["message"]
            .as_str()
            .unwrap()
            .starts_with("扫描文件失败")
    );
    // 只取比上次看到的更新的日志。
    let last = entries.last().unwrap()["seq"].as_u64().unwrap();
    let newer = json(&router, &format!("/api/admin/logs?level=warn&after={last}")).await;
    assert!(
        newer["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["seq"].as_u64().unwrap() > last)
    );
}

#[tokio::test]
async fn duplicates_group_files_with_the_same_full_hash() {
    let fixture = Fixture::new();
    fixture.archive("one.cbz", 10, 20);
    std::fs::copy(
        fixture.config.content_dir.join("one.cbz"),
        fixture.config.content_dir.join("copy.cbz"),
    )
    .unwrap();
    fixture.archive("other.cbz", 30, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    let router = app.router();

    let duplicates = json(&router, "/api/admin/duplicates").await;
    assert_eq!(duplicates["total"], 1);
    let paths: Vec<&str> = duplicates["groups"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["copy.cbz", "one.cbz"]);
}

#[tokio::test]
async fn namespaces_count_distinct_tags_and_uses_case_insensitively() {
    let fixture = Fixture::new();
    fixture.archive("a.cbz", 10, 20);
    fixture.archive("b.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    let tag = |namespace: &str, value: &str| Tag {
        namespace: namespace.into(),
        value: value.into(),
    };
    let a = store.find_by_path("a.cbz").unwrap().unwrap();
    let b = store.find_by_path("b.cbz").unwrap().unwrap();
    store
        .replace_tags(
            a,
            &[tag("artist", "x"), tag("Artist", "y"), tag("", "solo")],
        )
        .unwrap();
    store.replace_tags(b, &[tag("artist", "x")]).unwrap();
    app.refresh().await.unwrap();
    let router = app.router();

    let body = json(&router, "/api/admin/namespaces").await;
    assert_eq!(
        body["namespaces"],
        serde_json::json!([
            {"namespace": "artist", "tags": 2, "uses": 3},
            {"namespace": "", "tags": 1, "uses": 1},
        ])
    );
    let status = json(&router, "/api/admin/status").await;
    assert_eq!(
        (
            status["library"]["tags"].clone(),
            status["library"]["namespaces"].clone()
        ),
        (3.into(), 2.into())
    );
}

#[tokio::test]
async fn clearing_the_cache_removes_resized_images() {
    let fixture = Fixture::new();
    let app = fixture.app().await;
    let router = app.router();
    let cache = fixture.config.data_dir.join("cache/images");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("a.jpg"), [0_u8; 100]).unwrap();
    std::fs::write(cache.join("b.jpg"), [0_u8; 50]).unwrap();
    // 正在生成的图片的临时文件，清空时不能删，也不算进缓存大小。
    std::fs::write(cache.join(".tmpAbC123"), [0_u8; 10]).unwrap();

    let status = json(&router, "/api/admin/status").await;
    assert_eq!(status["storage"]["cache_files"], 2);
    assert_eq!(status["storage"]["cache_bytes"], 150);
    let (code, _, body) = request(&router, "DELETE", "/api/admin/cache", None, &[]).await;
    assert_eq!(code, StatusCode::OK);
    let removed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        removed,
        serde_json::json!({"removed_files": 2, "removed_bytes": 150})
    );
    assert!(cache.is_dir());
    assert!(cache.join(".tmpAbC123").is_file());
    assert_eq!(
        json(&router, "/api/admin/status").await["storage"]["cache_files"],
        0
    );
}

#[tokio::test]
async fn rejects_invalid_parameters() {
    let fixture = Fixture::new();
    let router = fixture.app().await.router();
    for uri in [
        "/api/admin/duplicates?limit=0",
        "/api/admin/duplicates?limit=501",
        "/api/admin/logs?level=debug",
        "/api/admin/logs?limit=0",
        "/api/admin/logs?after=-1",
    ] {
        assert_eq!(
            request(&router, "GET", uri, None, &[]).await.0,
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }
    let refreshed = request(&router, "POST", "/api/admin/refresh", None, &[]).await;
    assert_eq!(refreshed.0, StatusCode::OK);
}

#[tokio::test]
async fn write_requests_from_other_sites_are_rejected() {
    let fixture = Fixture::new();
    let router = fixture.app().await.router();
    for headers in [
        &[("sec-fetch-site", "cross-site")][..],
        &[("sec-fetch-site", "same-site")][..],
        &[("origin", "http://evil.example"), ("host", "library:3002")][..],
    ] {
        let (status, _, _) = request(&router, "POST", "/api/admin/refresh", None, headers).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{headers:?}");
        let (status, _, _) = request(&router, "DELETE", "/api/admin/cache", None, headers).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{headers:?}");
    }
    for headers in [
        &[("sec-fetch-site", "same-origin")][..],
        &[("origin", "http://library:3002"), ("host", "library:3002")][..],
        &[][..],
    ] {
        let (status, _, _) = request(&router, "POST", "/api/admin/refresh", None, headers).await;
        assert_eq!(status, StatusCode::OK, "{headers:?}");
    }
    // 读接口不受影响。
    let (status, _, _) = request(
        &router,
        "GET",
        "/api/admin/status",
        None,
        &[("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn books_with_broken_pages_are_imported_and_listed() {
    let fixture = Fixture::new();
    let good = fixture.archive("good.cbz", 10, 20);
    // 第一页全是 0（真实书库里见过），第二页正常。
    let path = fixture.config.content_dir.join("bad-page.cbz");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, bytes) in [("01.jpg", vec![0_u8; 4096]), ("02.png", good)] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, &bytes).unwrap();
    }
    zip.finish().unwrap();
    let app = fixture.app().await;
    let report = app.scan().await.unwrap();
    assert_eq!((report.added, report.failed), (2, 0));
    let router = app.router();

    let issues = json(&router, "/api/admin/issues").await;
    assert_eq!(issues["broken"]["total"], 1);
    let item = &issues["broken"]["items"][0];
    assert_eq!(item["path"], "bad-page.cbz");
    assert_eq!(item["pages"], serde_json::json!(["01.jpg"]));
    assert_eq!(
        json(&router, "/api/admin/status").await["scan"]["issues"]["broken"],
        1
    );

    let id = item["id"].as_i64().unwrap();
    let pages = json(&router, &format!("/api/archives/{id}/pages")).await;
    let first = &pages["pages"][0];
    assert_eq!(first["broken"], true);
    assert!(first["width"].as_u64().unwrap() > 0 && first["height"].as_u64().unwrap() > 0);
    assert_eq!(pages["pages"][1]["broken"], false);
    let (status, _, _) = request(&router, "GET", first["url"].as_str().unwrap(), None, &[]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // 封面坏了时缩略图用第二页。
    let cover = json(&router, &format!("/api/archives/{id}")).await["cover_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, headers, _) = request(&router, "GET", &cover, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/jpeg");
}

#[tokio::test]
async fn page_thumbnails_are_per_page_and_folder_books_have_no_file_to_download() {
    let fixture = Fixture::new();
    let png = fixture.archive("unused.cbz", 30, 40);
    let book = fixture.config.content_dir.join("pictures");
    std::fs::create_dir(&book).unwrap();
    std::fs::write(book.join("1.png"), b"").unwrap();
    std::fs::write(book.join("2.png"), &png).unwrap();
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = Store::open(fixture.database())
        .unwrap()
        .find_by_path("pictures")
        .unwrap()
        .unwrap();
    let pages = json(&router, &format!("/api/archives/{id}/pages")).await;
    // 坏掉的第一页的缩略图是 422，不会悄悄换成别的页；封面才挑第一张好的。
    let (status, _, _) = request(
        &router,
        "GET",
        pages["pages"][0]["thumbnail_url"].as_str().unwrap(),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _, _) = request(
        &router,
        "GET",
        pages["pages"][1]["thumbnail_url"].as_str().unwrap(),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &router,
        "GET",
        &format!("/api/archives/{id}/file"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
