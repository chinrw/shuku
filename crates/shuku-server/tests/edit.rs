mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::{Value, json as value};

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, _, bytes) = request(router, method, uri, body, &[]).await;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

fn ids(body: &Value) -> Vec<i64> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect()
}

async fn scanned(names: &[&str]) -> (Fixture, shuku_server::App, axum::Router, Vec<i64>) {
    let fixture = Fixture::new();
    for name in names {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let all = ids(&json(&router, "/api/search").await);
    (fixture, app, router, all)
}

#[tokio::test]
async fn editing_replaces_title_summary_source_and_tags() {
    let (_fixture, app, router, all) = scanned(&["a.cbz"]).await;
    let uri = format!("/api/archives/{}", all[0]);

    let (status, detail) = send(
        &router,
        "PATCH",
        &uri,
        Some(value!({
            "title": "  新标题 ", "summary": "第一行\n第二行",
            "source_url": "exhentai.org/g/1/abc",
            "tags": ["artist: 某某 ", "language:chinese", "单独", "Artist:某某"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["title"], "新标题");
    assert_eq!(detail["summary"], "第一行\n第二行");
    assert_eq!(detail["source_url"], "exhentai.org/g/1/abc");
    assert_eq!(
        detail["tags"],
        value!([
            {"namespace": "", "values": ["单独"]},
            {"namespace": "artist", "values": ["某某"]},
            {"namespace": "language", "values": ["chinese"]},
        ])
    );
    assert_eq!(json(&router, &uri).await, detail);

    // 只给出的字段变；加和删不碰其余的标签。空字符串清除来源。
    let (status, detail) = send(
        &router,
        "PATCH",
        &uri,
        Some(value!({"source_url": "", "add_tags": ["female:glasses"], "remove_tags": ["单独"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["title"], "新标题");
    assert_eq!(detail["source_url"], Value::Null);
    assert_eq!(
        detail["tags"],
        value!([
            {"namespace": "artist", "values": ["某某"]},
            {"namespace": "female", "values": ["glasses"]},
            {"namespace": "language", "values": ["chinese"]},
        ])
    );

    // 搜索走快照，重建后才看得到；去掉的标签不再出现在补全里。
    app.refresh().await.unwrap();
    let found = json(&router, "/api/search?q=female:glasses$").await;
    assert_eq!(ids(&found), all);
    assert_eq!(found["items"][0]["title"], "新标题");
    assert_eq!(
        json(&router, "/api/tags?prefix=单").await["tags"],
        value!([])
    );
}

#[tokio::test]
async fn invalid_edits_are_rejected_without_writing_anything() {
    let (_fixture, _app, router, all) = scanned(&["a.cbz"]).await;
    let uri = format!("/api/archives/{}", all[0]);
    let before = json(&router, &uri).await;
    for body in [
        value!({"title": "  "}),
        value!({"title": "x".repeat(1001)}),
        value!({"tags": ["a,b"]}),
        value!({"tags": ["artist:a\nb"]}),
        value!({"tags": ["artist:"]}),
        value!({"tags": ["ok"], "add_tags": ["x"]}),
        value!({"source_url": "has space"}),
        value!({"title": "ok", "tags": ["ok", "  "]}),
        value!({"unknown": 1}),
    ] {
        let (status, error) = send(&router, "PATCH", &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {error}");
    }
    assert_eq!(json(&router, &uri).await, before);

    let (status, _) = send(
        &router,
        "PATCH",
        &format!("/api/archives/{}", all[0] + 100),
        Some(value!({"title": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = request(
        &router,
        "PATCH",
        &uri,
        Some(value!({"title": "x"})),
        &[("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn only_records_whose_files_are_gone_can_be_deleted() {
    let (fixture, app, router, all) = scanned(&["a.cbz", "b.cbz"]).await;
    let (a, b) = (all[0], all[1]);
    let detail = json(&router, &format!("/api/archives/{a}")).await;
    assert_eq!(detail["path"], "a.cbz");
    assert_eq!(detail["missing"], false);

    let (status, error) = send(&router, "DELETE", &format!("/api/archives/{a}"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    std::fs::remove_file(fixture.config.content_dir.join("a.cbz")).unwrap();
    assert_eq!(
        json(&router, &format!("/api/archives/{a}")).await["missing"],
        true
    );
    let (status, _) = send(&router, "DELETE", &format!("/api/archives/{a}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&router, "GET", &format!("/api/archives/{a}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&router, "DELETE", &format!("/api/archives/{a}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    app.refresh().await.unwrap();
    assert_eq!(ids(&json(&router, "/api/search").await), [b]);
}

#[tokio::test]
async fn missing_records_from_the_last_scan_are_deleted_in_bulk() {
    let (fixture, app, router, all) = scanned(&["a.cbz", "b.cbz", "c.cbz"]).await;
    for name in ["a.cbz", "b.cbz"] {
        std::fs::remove_file(fixture.config.content_dir.join(name)).unwrap();
    }
    app.scan().await.unwrap();
    let issues = json(&router, "/api/admin/issues").await;
    assert_eq!(issues["missing"]["total"], 2);

    let (status, result) = send(
        &router,
        "POST",
        "/api/admin/missing/delete",
        Some(value!({"ids": [all[0], all[1], all[0], all[2], all[2] + 100]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        result,
        value!({"deleted": [all[0], all[1], all[0]], "present": [all[2]], "not_found": [all[2] + 100]})
    );
    let issues = json(&router, "/api/admin/issues").await;
    assert_eq!(issues["missing"], value!({"total": 0, "items": []}));
    assert_eq!(
        json(&router, "/api/admin/status").await["scan"]["issues"]["missing"],
        0
    );
    app.refresh().await.unwrap();
    assert_eq!(ids(&json(&router, "/api/search").await), [all[2]]);
}

#[tokio::test]
async fn nothing_is_deleted_while_the_content_directory_looks_unmounted() {
    let (fixture, _app, router, all) = scanned(&["a.cbz"]).await;
    std::fs::remove_file(fixture.config.content_dir.join("a.cbz")).unwrap();
    let (status, error) = send(
        &router,
        "DELETE",
        &format!("/api/archives/{}", all[0]),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(
        json(&router, &format!("/api/archives/{}", all[0])).await["missing"],
        true
    );
}

#[tokio::test]
async fn deleting_with_the_file_removes_both_from_a_writable_library() {
    let (fixture, app, router, all) = scanned(&["a.cbz", "b.cbz"]).await;
    let (a, b) = (all[0], all[1]);
    let file = fixture.config.content_dir.join("a.cbz");
    assert_eq!(
        json(&router, &format!("/api/archives/{a}")).await["deletable"],
        true
    );

    let uri = format!("/api/archives/{a}?file=true");
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{error}");
    assert!(!file.exists());
    let (status, _) = send(&router, "GET", &format!("/api/archives/{a}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 下次扫描不会把它当作失踪记录，也不会重新入库。
    app.scan().await.unwrap();
    assert_eq!(ids(&json(&router, "/api/search").await), [b]);
    let issues = json(&router, "/api/admin/issues").await;
    assert_eq!(issues["missing"]["total"], 0);
}

#[tokio::test]
async fn files_changed_since_the_scan_or_an_unmounted_library_are_not_deleted() {
    let (fixture, _app, router, all) = scanned(&["a.cbz", "b.cbz"]).await;
    let content = &fixture.config.content_dir;

    // 同一路径上换成了另一个文件。
    let file = content.join("a.cbz");
    let original = std::fs::read(&file).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    fixture.archive_with_pages("a.cbz", 5);
    let uri = format!("/api/archives/{}?file=true", all[0]);
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(content.join("a.cbz").exists());

    // 等长的另一个文件，修改时间也改回原来的，只有快速指纹能认出来。
    let mut bytes = original;
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&file, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(file.exists());

    // 网络盘没挂载时内容目录是空的，文件“不在”不等于可以只删记录。
    std::fs::remove_file(content.join("a.cbz")).unwrap();
    std::fs::remove_file(content.join("b.cbz")).unwrap();
    let uri = format!("/api/archives/{}?file=true", all[1]);
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(
        json(&router, &format!("/api/archives/{}", all[1])).await["id"],
        all[1]
    );
}

#[tokio::test]
async fn deleting_a_folder_book_removes_only_its_pages() {
    let fixture = Fixture::new();
    let png = fixture.archive("zipped.cbz", 30, 40);
    let content = &fixture.config.content_dir;
    std::fs::remove_file(content.join("zipped.cbz")).unwrap();
    let book = content.join("book");
    std::fs::create_dir_all(book.join("inner")).unwrap();
    std::fs::write(book.join("1.png"), &png).unwrap();
    std::fs::write(book.join("2.png"), &png).unwrap();
    std::fs::write(book.join("notes.txt"), "keep").unwrap();
    std::fs::write(book.join("inner/1.png"), &png).unwrap();
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let mut outer = None;
    for id in ids(&json(&router, "/api/search").await) {
        if json(&router, &format!("/api/archives/{id}")).await["path"] == "book" {
            outer = Some(id);
        }
    }
    let outer = outer.unwrap();

    // 上次中断留下的暂存目录要人来处理，这次什么都不删。
    let staging = book.join(".shuku-deleting");
    std::fs::create_dir(&staging).unwrap();
    let uri = format!("/api/archives/{outer}?file=true");
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(book.join("1.png").exists() && book.join("2.png").exists());
    std::fs::remove_dir(&staging).unwrap();
    // 文件夹的修改时间变了，要重新扫描才能删（服务运行时文件监视会自动扫描）。
    app.scan().await.unwrap();

    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{error}");
    assert!(!staging.exists());
    assert!(!book.join("1.png").exists() && !book.join("2.png").exists());
    assert!(book.join("notes.txt").exists());
    assert!(book.join("inner/1.png").exists());

    app.scan().await.unwrap();
    let left = ids(&json(&router, "/api/search").await);
    assert_eq!(left.len(), 1);
    assert_eq!(
        json(&router, &format!("/api/archives/{}", left[0])).await["path"],
        "book/inner"
    );
}

#[tokio::test]
async fn a_folder_whose_pages_are_already_gone_only_loses_its_record() {
    let fixture = Fixture::new();
    let png = fixture.archive("book/1.png", 30, 40);
    let book = fixture.config.content_dir.join("book");
    std::fs::write(book.join("1.png"), &png).unwrap();
    std::fs::write(book.join("notes.txt"), "keep").unwrap();
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = ids(&json(&router, "/api/search").await)[0];

    // 例如上次删完页面后写库失败：文件夹因为还有别的文件留着，身份已经变了。
    std::fs::remove_file(book.join("1.png")).unwrap();
    let uri = format!("/api/archives/{id}?file=true");
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{error}");
    assert!(book.join("notes.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn files_reached_through_a_symlinked_directory_are_not_deleted() {
    let (fixture, _app, router, all) = scanned(&["sub/a.cbz", "b.cbz"]).await;
    let sub = fixture.config.content_dir.join("sub");
    let outside = fixture.root.path().join("outside");
    std::fs::rename(&sub, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &sub).unwrap();
    let mut target = None;
    for &id in &all {
        if json(&router, &format!("/api/archives/{id}")).await["path"] == "sub/a.cbz" {
            target = Some(id);
        }
    }
    let uri = format!("/api/archives/{}?file=true", target.unwrap());
    let (status, error) = send(&router, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(outside.join("a.cbz").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn files_in_a_read_only_library_are_not_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let (fixture, _app, router, all) = scanned(&["sub/a.cbz"]).await;
    let sub = fixture.config.content_dir.join("sub");
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    // root 不受权限位限制。
    if std::fs::write(sub.join("probe"), b"").is_ok() {
        return;
    }
    let a = all[0];
    let detail = json(&router, &format!("/api/archives/{a}")).await;
    assert_eq!(detail["deletable"], false);
    let (status, error) = send(
        &router,
        "DELETE",
        &format!("/api/archives/{a}?file=true"),
        None,
    )
    .await;
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error["error"].as_str().unwrap().contains("只读"), "{error}");
    assert!(sub.join("a.cbz").exists());
    assert_eq!(json(&router, &format!("/api/archives/{a}")).await["id"], a);
}

#[tokio::test]
async fn renaming_a_tag_merges_it_and_updates_search_immediately() {
    let (_fixture, _app, router, all) = scanned(&["a.cbz", "b.cbz"]).await;
    for (id, tag) in [(all[0], "artist:alfa"), (all[1], "artist:alpha")] {
        let (status, _) = send(
            &router,
            "PATCH",
            &format!("/api/archives/{id}"),
            Some(value!({"tags": [tag]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, result) = send(
        &router,
        "POST",
        "/api/admin/tags/rename",
        Some(value!({"from": "artist:Alfa", "to": "artist:alpha"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result, value!({"archives": 1}));
    assert_eq!(
        ids(&json(&router, "/api/search?q=artist:alpha$").await),
        all
    );
    assert_eq!(
        json(&router, "/api/tags?prefix=artist:").await["tags"],
        value!([{"tag": "artist:alpha", "count": 2}])
    );

    let (status, _) = send(
        &router,
        "POST",
        "/api/admin/tags/rename",
        Some(value!({"from": "artist:alfa", "to": "artist:x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
