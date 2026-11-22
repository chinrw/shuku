mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::json as value;
use shuku_store::Store;
use std::{
    fs,
    time::{Duration, UNIX_EPOCH},
};

#[tokio::test]
async fn duplicate_download_survives_losing_either_path_without_waiting_for_a_scan() {
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    let png = f.archive("first.zip", 17, 31);
    let app = f.app().await;
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    let store = Store::open(f.database()).unwrap();
    let id = store.find_by_path("first.zip").unwrap().unwrap();
    let bytes = fs::read(dir.join("first.zip")).unwrap();
    fs::copy(dir.join("first.zip"), dir.join("second.cbz")).unwrap();
    fs::File::options()
        .write(true)
        .open(dir.join("second.cbz"))
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_605_000_000))
        .unwrap();
    app.scan().await.unwrap();
    let old = store.find_by_path("second.cbz").unwrap().unwrap();
    app.hash_contents().await.unwrap();
    assert_eq!(store.find_by_path("second.cbz").unwrap(), Some(id));
    let app = f.app().await;
    let http = app.router();
    assert_eq!(json(&http, "/api/search").await["total"], 1);
    let uri = format!("/api/archives/{id}");
    let old_uri = format!("/api/archives/{old}/pages");
    let (status, headers, _) = request(&http, "GET", &old_uri, None, &[]).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(headers["location"], format!("{uri}/pages"));
    let (status, _, _) = request(&http, "DELETE", &format!("{uri}?file=true"), None, &[]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        json(&http, &uri).await["files"].as_array().unwrap().len(),
        2
    );
    fs::remove_file(dir.join("first.zip")).unwrap();
    assert_eq!(json(&http, &uri).await["path"], "second.cbz");
    let page_url = json(&http, &format!("{uri}/pages")).await["pages"][0]["url"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _, body) = request(&http, "GET", &page_url, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, png);
    assert_eq!(
        request(&http, "GET", &format!("{uri}/file"), None, &[])
            .await
            .2,
        bytes
    );
    assert_eq!(
        request(&http, "DELETE", &uri, None, &[]).await.0,
        StatusCode::CONFLICT
    );
    app.scan().await.unwrap();
    assert_eq!(
        json(&http, "/api/admin/issues").await["missing"]["total"],
        0
    );
    fs::write(dir.join("first.zip"), &bytes).unwrap();
    app.scan().await.unwrap();
    assert_eq!(json(&http, "/api/search").await["total"], 1);
    let (status, _, _) = request(
        &http,
        "DELETE",
        &format!("{uri}?file=true&path=second.cbz"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!dir.join("second.cbz").exists());
    assert_eq!(fs::read(dir.join("first.zip")).unwrap(), bytes);
    assert_eq!(json(&http, "/api/search").await["items"][0]["id"], id);
}

#[tokio::test]
async fn removing_one_hard_link_leaves_the_other_readable_before_scanning() {
    let f = Fixture::new();
    let content = &f.config.content_dir;
    f.archive("a.zip", 20, 30);
    fs::hard_link(content.join("a.zip"), content.join("b.zip")).unwrap();
    let app = f.app().await;
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    fs::remove_file(content.join("a.zip")).unwrap();
    let (status, _, bytes) = request(
        &router,
        "GET",
        &format!("/api/archives/{id}/file"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, fs::read(content.join("b.zip")).unwrap());
}

#[tokio::test]
async fn changed_primary_gets_a_new_entry_while_original_content_keeps_its_id() {
    let f = Fixture::new();
    f.archive("a.zip", 12, 34);
    fs::copy(
        f.config.content_dir.join("a.zip"),
        f.config.content_dir.join("b.zip"),
    )
    .unwrap();
    let app = f.app().await;
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    let store = Store::open(f.database()).unwrap();
    let id = store.find_by_path("a.zip").unwrap().unwrap();
    f.archive_with_pages("a.zip", 5);
    app.scan().await.unwrap();
    assert_ne!(store.find_by_path("a.zip").unwrap(), Some(id));
    assert_eq!(store.find_by_path("b.zip").unwrap(), Some(id));
    assert_eq!(store.archive(id).unwrap().unwrap().page_count, 2);
    assert_eq!(json(&app.router(), "/api/search").await["total"], 2);
}

#[tokio::test]
async fn existing_entries_need_a_review_and_only_matching_full_hashes_can_merge() {
    let f = Fixture::new();
    let root = &f.config.content_dir;
    f.archive("a.zip", 12, 34);
    fs::copy(root.join("a.zip"), root.join("b.zip")).unwrap();
    f.archive("different.zip", 20, 40);
    let app = f.app().await;
    app.scan().await.unwrap();
    // 已有库升级到迁移 014 时，所有旧记录默认为不自动合并。
    shuku_store::open_connection(f.database())
        .unwrap()
        .execute("UPDATE archive SET auto_copy=0", [])
        .unwrap();
    app.hash_contents().await.unwrap();
    let mut store = Store::open(f.database()).unwrap();
    let a = store.find_by_path("a.zip").unwrap().unwrap();
    let b = store.find_by_path("b.zip").unwrap().unwrap();
    let other = store.find_by_path("different.zip").unwrap().unwrap();
    store.write_progress(a, 1, 100).unwrap();
    store.write_progress(b, 0, 200).unwrap();
    let category = store
        .write(|w| {
            let c = w.create_category("收藏", "", false)?;
            w.set_category_member(c, b, true)?;
            Ok::<_, shuku_store::Error>(c)
        })
        .unwrap();
    let router = app.router();
    assert_eq!(json(&router, "/api/search").await["total"], 3);
    let hash = shuku_archive::content_hash(root.join("a.zip")).unwrap();
    for (expected, ids) in [("stale", vec![b]), (hash.as_str(), vec![b, other])] {
        let (status, _, _) = request(
            &router,
            "POST",
            "/api/admin/duplicates/merge",
            Some(value!({"keep":a,"others":ids,"hash":expected})),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(store.archive(b).unwrap().is_some());
    }
    let (status, _, _) = request(
        &router,
        "POST",
        "/api/admin/duplicates/merge",
        Some(value!({"keep":a,"others":[b],"hash":hash})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store.archive_categories(a).unwrap(), [category]);
    let progress = store.archive(a).unwrap().unwrap().progress.unwrap();
    assert_eq!(progress.page, 0);
    assert!(progress.finished);
    assert_eq!(json(&router, "/api/search").await["total"], 2);
}

#[tokio::test]
async fn equal_quick_hashes_do_not_merge_or_relink_different_full_hashes() {
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    let png = f.archive("kept.zip", 12, 24);
    let first = dir.join("a.zip");
    let second = dir.join("b.zip");
    let mut zip = ZipWriter::new(fs::File::create(&first).unwrap());
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let padding = vec![0; 96 * 1024];
    for (name, bytes) in [
        ("left.bin", padding.as_slice()),
        ("page.png", &png),
        ("right.bin", &padding),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    // 只改中间条目本地文件头中的 DOS 时间。两份 ZIP 的图片和 CRC 都完好，
    // 整个文件的大小和首尾各 64 KiB 也相同，但完整指纹不同。
    let original = fs::read(&first).unwrap();
    let mut bytes = original.clone();
    let pos = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x03\x04")
        .nth(1)
        .unwrap()
        .0;
    bytes[pos + 10] ^= 1;
    fs::write(&second, &bytes).unwrap();
    assert_eq!(
        shuku_archive::quick_hash(&first).unwrap(),
        shuku_archive::quick_hash(&second).unwrap()
    );
    assert_ne!(
        shuku_archive::content_hash(&first).unwrap(),
        shuku_archive::content_hash(&second).unwrap()
    );
    let app = f.app().await;
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    let store = Store::open(f.database()).unwrap();
    let a = store.find_by_path("a.zip").unwrap().unwrap();
    let b = store.find_by_path("b.zip").unwrap().unwrap();
    assert_ne!(a, b);
    // 两本都移走后只放回 b 的内容。即使 a 的 ID 更小，也不能抢走 b 的路径。
    fs::remove_file(first).unwrap();
    fs::rename(second, dir.join("returned.zip")).unwrap();
    app.scan().await.unwrap();
    assert_eq!(store.find_by_path("returned.zip").unwrap(), Some(b));
    assert!(store.archive(a).unwrap().unwrap().missing);

    let returned = dir.join("returned.zip");
    fs::copy(&returned, dir.join("copy.zip")).unwrap();
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    assert_eq!(store.find_by_path("copy.zip").unwrap(), Some(b));
    let mtime = returned.metadata().unwrap().modified().unwrap();
    fs::write(&returned, original).unwrap();
    fs::File::options()
        .write(true)
        .open(&returned)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    // 长度、mtime 和快速指纹全没变。旧书仍从未被替换的副本读。
    let (status, _, body) = request(
        &app.router(),
        "GET",
        &format!("/api/archives/{b}/file"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, bytes);
    app.scan().await.unwrap();
    assert_eq!(store.find_by_path("copy.zip").unwrap(), Some(b));
    assert_eq!(store.find_by_path("returned.zip").unwrap(), Some(a));
}
