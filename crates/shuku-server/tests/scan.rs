mod common;

use common::Fixture;
use shuku_index::Query;
use shuku_store::{Store, Tag};

#[tokio::test]
async fn requires_content_directory_and_detects_mtime_only_changes() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 80, 120);
    let mut config = fixture.config.clone();
    config.content_dir = config.content_dir.join("book.cbz");
    assert!(shuku_server::App::open(config).await.is_err());
    assert!(!fixture.config.data_dir.exists());
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let file = std::fs::File::options()
        .write(true)
        .open(fixture.config.content_dir.join("book.cbz"))
        .unwrap();
    file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
    let report = app.scan().await.unwrap();
    assert_eq!((report.updated, report.unchanged), (1, 0));
}

#[tokio::test]
async fn rejects_overlapping_directories_before_writing_to_content() {
    let fixture = Fixture::new();
    let mut config = fixture.config.clone();
    config.data_dir = config.content_dir.join("nested/data");
    assert!(shuku_server::App::open(config.clone()).await.is_err());
    assert!(!fixture.config.content_dir.join("nested").exists());
    config.data_dir = fixture.root.path().to_owned();
    assert!(shuku_server::App::open(config).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn ignores_symlinks_in_content_and_keeps_missing_records_for_future_moves() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 80, 120);
    std::os::unix::fs::symlink(
        fixture.config.content_dir.join("book.cbz"),
        fixture.config.content_dir.join("alias.cbz"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        &fixture.config.content_dir,
        fixture.config.content_dir.join("loop"),
    )
    .unwrap();
    let app = fixture.app().await;
    assert_eq!(app.scan().await.unwrap().added, 1);
    let original = app.index().search(&Query::default()).unwrap().ids;
    let held_snapshot = app.index();
    std::fs::rename(
        fixture.config.content_dir.join("book.cbz"),
        fixture.root.path().join("away.cbz"),
    )
    .unwrap();
    app.scan().await.unwrap();
    assert_eq!(app.index().search(&Query::default()).unwrap().ids, original);
    std::fs::rename(
        fixture.root.path().join("away.cbz"),
        fixture.config.content_dir.join("returned.cbz"),
    )
    .unwrap();
    assert_eq!(app.scan().await.unwrap().moved, 1);
    fixture.archive("second.cbz", 90, 130);
    app.scan().await.unwrap();
    assert_eq!(held_snapshot.search(&Query::default()).unwrap().total, 1);
    assert_eq!(app.index().search(&Query::default()).unwrap().total, 2);
}

#[tokio::test]
async fn scans_nested_zip_by_magic_and_preserves_identity_on_moves_and_changes() {
    let fixture = Fixture::new();
    fixture.archive("nested/漫画 2.bin", 800, 1200);
    std::fs::write(
        fixture.config.content_dir.join("unsupported.rar"),
        b"Rar!test",
    )
    .unwrap();
    std::fs::write(
        fixture.config.content_dir.join("broken.cbz"),
        b"PK\x03\x04broken",
    )
    .unwrap();
    let app = fixture.app().await;
    assert_eq!(app.index().search(&Query::default()).unwrap().total, 0);
    let report = app.scan().await.unwrap();
    assert_eq!((report.added, report.skipped, report.failed), (1, 1, 1));
    let ids = app.index().search(&Query::default()).unwrap().ids;
    let id = ids[0];
    let mut store = Store::open(fixture.database()).unwrap();
    let original = store.archive(id).unwrap().unwrap();
    assert_eq!(original.metadata.title, "漫画 2");
    assert_eq!(original.page_count, 2);
    // 扫描不读整个文件，完整指纹留给后台任务。
    assert_eq!(original.metadata.content_hash, None);
    let pages = store.pages(id).unwrap();
    assert_eq!(pages[0].entry_name, "2.png");
    assert_eq!((pages[0].width, pages[0].height), (800, 1200));
    store
        .replace_tags(
            id,
            &[Tag {
                namespace: "artist".into(),
                value: "某某".into(),
            }],
        )
        .unwrap();
    store.write_progress(id, 1, 2000).unwrap();
    let mut metadata = original.metadata.clone();
    metadata.title = "人工标题".into();
    metadata.summary = "保留摘要".into();
    store.save_archive(Some(id), &metadata, &pages).unwrap();
    let unchanged = app.scan().await.unwrap();
    assert_eq!(
        (unchanged.added, unchanged.updated, unchanged.unchanged),
        (0, 0, 1)
    );
    std::fs::rename(
        fixture.config.content_dir.join("nested/漫画 2.bin"),
        fixture.config.content_dir.join("renamed.cbz"),
    )
    .unwrap();
    let moved = app.scan().await.unwrap();
    assert_eq!((moved.added, moved.moved), (0, 1));
    assert_eq!(store.find_by_path("renamed.cbz").unwrap(), Some(id));
    fixture.archive("renamed.cbz", 900, 1300);
    let updated = app.scan().await.unwrap();
    assert_eq!(updated.updated, 1);
    let archive = store.archive(id).unwrap().unwrap();
    assert_eq!(archive.metadata.title, "人工标题");
    assert_eq!(archive.metadata.summary, "保留摘要");
    assert_eq!(archive.tags[0].value, "某某");
    assert_eq!(archive.progress.unwrap().page, 1);
    assert_ne!(archive.metadata.quick_hash, original.metadata.quick_hash);
    assert_eq!(store.pages(id).unwrap()[0].width, 900);
    // 已存在的原件不能被相同指纹的新副本抢走身份。
    std::fs::copy(
        fixture.config.content_dir.join("renamed.cbz"),
        fixture.config.content_dir.join("copy.cbz"),
    )
    .unwrap();
    assert_eq!(app.scan().await.unwrap().added, 1);
    let reopened = fixture.app().await;
    assert_eq!(reopened.index().search(&Query::default()).unwrap().total, 2);
}
