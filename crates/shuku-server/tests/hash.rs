mod common;

use common::Fixture;
use shuku_store::Store;
use std::{
    fs,
    time::{Duration, UNIX_EPOCH},
};

fn content_hash(fixture: &Fixture, path: &str) -> Option<String> {
    let store = Store::open(fixture.database()).unwrap();
    let id = store.find_by_path(path).unwrap().unwrap();
    store.archive(id).unwrap().unwrap().metadata.content_hash
}

#[tokio::test]
async fn hashes_unchanged_files_and_skips_changed_or_missing_ones() {
    let fixture = Fixture::new();
    for name in ["kept.cbz", "changed.cbz", "gone.cbz"] {
        fixture.archive(name, 10, 20);
    }
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let content = &fixture.config.content_dir;
    // 扫描之后、补算之前文件变了或被删了，这时算出的指纹不属于数据库里的那份文件。
    fs::File::options()
        .write(true)
        .open(content.join("changed.cbz"))
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
    fs::remove_file(content.join("gone.cbz")).unwrap();

    let report = app.hash_contents().await.unwrap();

    assert_eq!((report.hashed, report.skipped, report.failed), (1, 2, 0));
    assert_eq!(
        content_hash(&fixture, "kept.cbz"),
        Some(shuku_archive::content_hash(content.join("kept.cbz")).unwrap())
    );
    assert_eq!(content_hash(&fixture, "changed.cbz"), None);
    assert_eq!(content_hash(&fixture, "gone.cbz"), None);

    // 重新扫描后变化的文件可以补算；已经算过的不会再读。
    app.scan().await.unwrap();
    let report = app.hash_contents().await.unwrap();
    assert_eq!((report.hashed, report.skipped, report.failed), (1, 1, 0));
    assert!(content_hash(&fixture, "changed.cbz").is_some());
}

#[tokio::test]
async fn skips_files_rewritten_with_same_size_and_mtime() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let path = fixture.config.content_dir.join("book.cbz");
    // 等长重写并把修改时间改回去，大小和秒级修改时间都和扫描时一样，只有内容不同。
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut bytes = fs::read(&path).unwrap();
    for byte in bytes.iter_mut().take(8) {
        *byte = !*byte;
    }
    fs::write(&path, &bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();

    let report = app.hash_contents().await.unwrap();

    assert_eq!((report.hashed, report.skipped, report.failed), (0, 1, 0));
    assert_eq!(content_hash(&fixture, "book.cbz"), None);
}

#[tokio::test]
async fn stops_before_reading_files_once_stopped() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();

    app.stop();
    let report = app.hash_contents().await.unwrap();

    assert_eq!((report.hashed, report.skipped, report.failed), (0, 0, 0));
    assert_eq!(content_hash(&fixture, "book.cbz"), None);
}
