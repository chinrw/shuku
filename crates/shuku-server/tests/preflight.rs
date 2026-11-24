mod common;

use axum::http::StatusCode;
use common::{Fixture, request};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path};
use zip::{CompressionMethod, write::SimpleFileOptions};

const CHECK: &str = "/api/archives/check-duplicate";
// 用 Python 的 hashlib.sha256 独立计算，固定调用方和服务端的字节协议。
const A: &str = "6c63cc5063ac82d8bbc925f9a31adf3a87f1510c021e0fde51854d60484b5019";
const B: &str = "4495c163ec0c65f1326b768135003cc49f436e67d59c98ac8011e6d59c9f19b9";
const C: &str = "94a8b9bb4c1cac547867f3001b67be5909d733614d1b784402efccbf52906d06";

fn gif(color: [u8; 3]) -> Vec<u8> {
    [
        b"GIF89a\x01\0\x01\0\x80\0\0".as_slice(),
        &color,
        b"\xff\xff\xff,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;",
    ]
    .concat()
}

fn zip_at(path: &Path, method: CompressionMethod, files: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        zip.start_file(
            *name,
            SimpleFileOptions::default().compression_method(method),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

async fn check(http: &axum::Router, hashes: &[&str]) -> Value {
    let (status, _, body) = request(
        http,
        "POST",
        CHECK,
        Some(json!({"page_sha256":hashes})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn page_multiset_matches_repacked_renamed_archives_and_folders_but_not_editions() {
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    let (a, b) = (gif([0, 0, 0]), gif([255, 0, 0]));
    zip_at(
        &dir.join("old.cbz"),
        CompressionMethod::Stored,
        &[
            ("1.gif", &a),
            ("2.gif", &b),
            ("3.gif", &a),
            ("ComicInfo.xml", b"<old/>"),
        ],
    );
    zip_at(
        &dir.join("other.zip"),
        CompressionMethod::Deflated,
        &[
            ("new/x.gif", &b),
            ("notes.json", b"{}"),
            ("new/y.gif", &a),
            ("new/z.gif", &a),
            ("__MACOSX/._old.gif", &b),
        ],
    );
    let loose = dir.join("loose");
    fs::create_dir(&loose).unwrap();
    for (name, bytes) in [("one.gif", &b), ("two.gif", &a), ("three.gif", &a)] {
        fs::write(loose.join(name), bytes).unwrap();
    }
    let app = f.app().await;
    let http = app.router();
    assert_eq!(
        check(&http, &[A, A, B]).await["coverage"]["reason"],
        "scan_not_started"
    );
    app.scan().await.unwrap();
    assert_eq!(
        check(&http, &[A, A, B]).await,
        json!({"status":"unknown","matches":[],"pending_archives":3,"coverage":{"reason":"pending_fingerprints"}})
    );
    app.hash_contents().await.unwrap();

    let result = check(&http, &[B, &A.to_uppercase(), A]).await;
    assert_eq!(result["status"], "duplicate");
    let store = shuku_store::Store::open(f.database()).unwrap();
    for name in ["loose", "old.cbz", "other.zip"] {
        let id = store.find_by_path(name).unwrap().unwrap();
        assert!(
            result["matches"]
                .as_array()
                .unwrap()
                .contains(&json!({"id":id,"path":name}))
        );
    }
    for pages in [&[A, B][..], &[A, A, B, B], &[A, B, B], &[A, A, C]] {
        assert_eq!(
            check(&http, pages).await,
            json!({"status":"no_duplicate","matches":[],"pending_archives":0})
        );
    }
    // 查重命中不能像整文件副本任务那样把三条不同记录合并。
    assert_eq!(store.stats().unwrap().archives, 3);
    drop(app);
    let restarted = f.app().await;
    restarted.scan().await.unwrap();
    assert_eq!(check(&restarted.router(), &[A, A, B]).await, result);
}

#[tokio::test]
async fn scan_failure_and_partly_built_or_invalidated_index_never_rule_out_a_duplicate() {
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    let (a, b) = (gif([0, 0, 0]), gif([255, 0, 0]));
    zip_at(
        &dir.join("first.zip"),
        CompressionMethod::Stored,
        &[("1.gif", &a)],
    );
    let app = f.app().await;
    let http = app.router();
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    zip_at(
        &dir.join("second.zip"),
        CompressionMethod::Stored,
        &[("2.gif", &b)],
    );
    app.scan().await.unwrap();
    let partial = check(&http, &[A]).await;
    assert_eq!(partial["status"], "unknown");
    assert_eq!(partial["matches"][0]["path"], "first.zip");
    assert_eq!(partial["pending_archives"], 1);
    // 查重 POST 不补算，也不触发扫描。
    assert_eq!(
        check(&http, &[B]).await,
        json!({"status":"unknown","matches":[],"pending_archives":1,"coverage":{"reason":"pending_fingerprints"}})
    );
    app.hash_contents().await.unwrap();
    assert_eq!(check(&http, &[B]).await["status"], "duplicate");

    zip_at(
        &dir.join("first.zip"),
        CompressionMethod::Stored,
        &[("1.gif", &a), ("2.gif", &b)],
    );
    app.scan().await.unwrap();
    assert_eq!(check(&http, &[A]).await["status"], "unknown");
    app.hash_contents().await.unwrap();
    assert_eq!(check(&http, &[A]).await["status"], "no_duplicate");
    assert_eq!(check(&http, &[A, B]).await["status"], "duplicate");
    fs::write(dir.join("broken.zip"), b"PK\x03\x04").unwrap();
    fs::write(dir.join("wrong-format.zip"), b"not a zip").unwrap();
    assert_eq!(app.scan().await.unwrap().failed, 2);
    assert_eq!(
        check(&http, &[C]).await,
        json!({"status":"unknown","matches":[],"pending_archives":0,"coverage":{"reason":"archive_format_failures","failed_archives":2}})
    );
}

#[tokio::test]
async fn rejects_empty_lists_and_invalid_page_hashes() {
    let f = Fixture::new();
    let app = f.app().await;
    for body in [
        json!({"page_sha256":[]}),
        json!({"page_sha256":["short"]}),
        json!({"page_sha256":["z".repeat(64)]}),
        json!({"content_hash":A}),
    ] {
        assert_eq!(
            request(&app.router(), "POST", CHECK, Some(body), &[])
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn unreadable_page_is_not_indexed_even_when_the_archive_already_has_a_full_hash() {
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    let path = dir.join("book.zip");
    let a = gif([0, 0, 0]);
    zip_at(&path, CompressionMethod::Stored, &[("page.gif", &a)]);
    // 损坏页数据而保留中央目录和图片头。扫描仍认得这本书，只有解压校验时才会失败。
    let mut bytes = fs::read(&path).unwrap();
    let offset = bytes.windows(a.len()).position(|w| w == a).unwrap();
    bytes[offset + a.len() - 1] ^= 1;
    fs::write(&path, bytes).unwrap();
    let app = f.app().await;
    assert_eq!(app.scan().await.unwrap().failed, 0);
    let mut store = shuku_store::Store::open(f.database()).unwrap();
    let entry = store.scan_entries().unwrap().pop().unwrap();
    store
        .set_content_hash(&entry, &shuku_archive::content_hash(&path).unwrap())
        .unwrap();
    assert_eq!(app.hash_contents().await.unwrap().hashed, 0);
    assert_eq!(
        check(&app.router(), &[A]).await,
        json!({"status":"unknown","matches":[],"pending_archives":1,"coverage":{"reason":"pending_fingerprints"}})
    );

    zip_at(
        &path,
        CompressionMethod::Deflated,
        &[("renamed.gif", &a), ("ComicInfo.xml", b"<fixed/>")],
    );
    app.scan().await.unwrap();
    let current = store.scan_entries().unwrap().pop().unwrap();
    store
        .set_content_hash(&current, &shuku_archive::content_hash(&path).unwrap())
        .unwrap();
    assert_eq!(app.hash_contents().await.unwrap().hashed, 0);
    assert_eq!(check(&app.router(), &[A]).await["status"], "duplicate");
}

#[tokio::test]
async fn returns_one_live_path_per_entry_and_never_reports_a_vanished_hit_as_absent() {
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    zip_at(
        &dir.join("a.zip"),
        CompressionMethod::Stored,
        &[("page.gif", &gif([0, 0, 0]))],
    );
    fs::copy(dir.join("a.zip"), dir.join("b.zip")).unwrap();
    let app = f.app().await;
    app.scan().await.unwrap();
    app.hash_contents().await.unwrap();
    let http = app.router();
    let result = check(&http, &[A]).await;
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    let id = &result["matches"][0]["id"];
    fs::remove_file(dir.join("a.zip")).unwrap();
    assert_eq!(
        check(&http, &[A]).await,
        json!({"status":"duplicate","matches":[{"id":id,"path":"b.zip"}],"pending_archives":0})
    );
    fs::remove_file(dir.join("b.zip")).unwrap();
    let result = check(&http, &[A]).await;
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["coverage"]["reason"], "indexed_match_changed");
}

#[tokio::test]
async fn a_failed_scan_cannot_be_treated_as_archive_format_failures() {
    let f = Fixture::new();
    let dir = &f.config.content_dir;
    fs::write(dir.join("bad.zip"), b"not a zip").unwrap();
    let app = f.app().await;
    assert_eq!(app.scan().await.unwrap().failed, 1);
    fs::rename(dir, dir.with_extension("moved")).unwrap();
    assert!(app.scan().await.is_err());
    let result = check(&app.router(), &[A]).await;
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["coverage"]["reason"], "scan_failed");
}
