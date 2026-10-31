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

#[cfg(unix)]
#[tokio::test]
async fn skips_unreadable_subdirectories_without_treating_their_records_as_missing() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 80, 120);
    fixture.archive("private/hidden.cbz", 90, 130);
    let app = fixture.app().await;
    assert_eq!(app.scan().await.unwrap().added, 2);
    let private = fixture.config.content_dir.join("private");
    let copy = fixture.root.path().join("copy.cbz");
    std::fs::copy(private.join("hidden.cbz"), &copy).unwrap();
    // 目录内改名后旧路径确实不存在，但目录只能进不能列，扫描看不到新路径。
    std::fs::rename(private.join("hidden.cbz"), private.join("renamed.cbz")).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o111)).unwrap();
    // root 不受权限位限制，这时测不出跳过的行为。
    if std::fs::read_dir(&private).is_ok() {
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    // 同内容的副本不能顶替读不到的那条记录。
    std::fs::rename(&copy, fixture.config.content_dir.join("copy.cbz")).unwrap();
    let report = app.scan().await;
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    let report = report.unwrap();
    assert_eq!(
        (report.added, report.moved, report.unchanged, report.failed),
        (1, 0, 1, 1)
    );
    assert_eq!(app.index().search(&Query::default()).unwrap().total, 3);
}

#[cfg(unix)]
#[tokio::test]
async fn reports_unchecked_records_and_fails_on_unreadable_content_root() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 80, 120);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let book = fixture.config.content_dir.join("book.cbz");
    std::fs::remove_file(&book).unwrap();
    std::os::unix::fs::symlink(&book, &book).unwrap();
    assert_eq!(app.scan().await.unwrap().failed, 1);
    let content = &fixture.config.content_dir;
    std::fs::set_permissions(content, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(content).is_ok() {
        std::fs::set_permissions(content, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let result = app.scan().await;
    std::fs::set_permissions(content, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn new_archives_get_lrr_metadata_staged_before_the_scan() {
    let fixture = Fixture::new();
    fixture.archive("sub/staged.cbz", 80, 120);
    fixture.archive("plain.cbz", 80, 120);
    std::fs::create_dir_all(&fixture.config.data_dir).unwrap();
    let mut store = Store::open(fixture.database()).unwrap();
    store
        .write(|writer| {
            writer.stage_lrr_pending(
                "sub/staged.cbz",
                &shuku_store::LrrPending {
                    archive: shuku_store::LrrArchive {
                        lrr_id: "a".repeat(40),
                        title: "LRR 的标题".into(),
                        summary: String::new(),
                        date_added: Some(1_600_000_000),
                        source_url: None,
                        is_new: false,
                        tags: vec![Tag {
                            namespace: "artist".into(),
                            value: "Alpha".into(),
                        }],
                    },
                    progress: Some((1, 1_700_000_000_000)),
                    categories: Vec::new(),
                    tankoubons: Vec::new(),
                },
            )
        })
        .unwrap();
    let app = fixture.app().await;
    assert_eq!(app.scan().await.unwrap().added, 2);

    let staged = store.find_by_path("sub/staged.cbz").unwrap().unwrap();
    let archive = store.archive(staged).unwrap().unwrap();
    assert_eq!(archive.metadata.title, "LRR 的标题");
    assert_eq!(archive.metadata.date_added, 1_600_000_000);
    assert!(!archive.metadata.is_new);
    assert_eq!(archive.progress.unwrap().page, 1);
    assert_eq!(store.lrr_pending_count().unwrap(), 0);
    let plain = store.find_by_path("plain.cbz").unwrap().unwrap();
    assert_eq!(
        store.archive(plain).unwrap().unwrap().metadata.title,
        "plain"
    );
    // 扫描结束时重建的快照里已经是 LRR 的标签。
    let found = app
        .index()
        .search(&shuku_index::parse_query("artist:alpha").unwrap().query)
        .unwrap();
    assert_eq!(found.ids, [staged]);
}

#[tokio::test]
async fn hidden_entries_are_skipped_unless_enabled() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 80, 120);
    fixture.archive(".partial/download.cbz", 80, 120);
    let png = fixture.archive("sub/.hidden.cbz", 80, 120);
    let pictures = fixture.config.content_dir.join(".partial/pictures");
    std::fs::create_dir_all(&pictures).unwrap();
    std::fs::write(pictures.join("1.png"), &png).unwrap();
    let app = fixture.app().await;
    let report = app.scan().await.unwrap();
    // 隐藏的目录和文件直接跳过，不算失败也不算不支持的格式。
    assert_eq!((report.added, report.failed, report.skipped), (1, 0, 0));

    // 在控制台里打开后，下一次扫描就带上隐藏的条目。
    let router = app.router();
    let (status, _, body) = common::request(
        &router,
        "PUT",
        "/api/admin/settings/scan",
        Some(serde_json::json!({"scan_hidden": true})),
        &[],
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"scan_hidden": true, "scan_interval_hours": 0})
    );
    let report = app.scan().await.unwrap();
    assert_eq!((report.added, report.unchanged), (3, 1));

    // 再关掉：隐藏目录里的书留在库里，不算失踪，同内容的可见副本作为新书入库而不是顶替它。
    common::request(
        &router,
        "PUT",
        "/api/admin/settings/scan",
        Some(serde_json::json!({"scan_hidden": false, "scan_interval_hours": 24})),
        &[],
    )
    .await;
    std::fs::copy(
        fixture.config.content_dir.join(".partial/download.cbz"),
        fixture.config.content_dir.join("copy.cbz"),
    )
    .unwrap();
    let visible = fixture.config.content_dir.join("pictures");
    std::fs::create_dir(&visible).unwrap();
    std::fs::write(visible.join("1.png"), &png).unwrap();
    let report = app.scan().await.unwrap();
    assert_eq!((report.added, report.moved), (2, 0));
    let store = Store::open(fixture.database()).unwrap();
    assert!(
        store
            .find_by_path(".partial/download.cbz")
            .unwrap()
            .is_some()
    );
    assert!(store.find_by_path(".partial/pictures").unwrap().is_some());
    drop(router);
    drop(app);
    // 设置存在库里，重启后还在。
    let reopened = fixture.app().await;
    let settings = common::json(&reopened.router(), "/api/admin/settings/scan").await;
    assert_eq!(
        settings,
        serde_json::json!({"scan_hidden": false, "scan_interval_hours": 24})
    );
}

#[tokio::test]
async fn new_archives_are_dated_by_their_modification_time() {
    let fixture = Fixture::new();
    fixture.archive("old.cbz", 80, 120);
    fixture.archive("future.cbz", 80, 120);
    let set_mtime = |name: &str, seconds: u64| {
        std::fs::File::options()
            .write(true)
            .open(fixture.config.content_dir.join(name))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    };
    set_mtime("old.cbz", 1_500_000_000);
    // 修改时间在未来时用扫描时的时间。
    set_mtime("future.cbz", 4_000_000_000);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let store = Store::open(fixture.database()).unwrap();
    let date = |name: &str| {
        let id = store.find_by_path(name).unwrap().unwrap();
        store.archive(id).unwrap().unwrap().metadata.date_added
    };
    assert_eq!(date("old.cbz"), 1_500_000_000);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!((now - 60..=now).contains(&date("future.cbz")));
}

#[tokio::test]
async fn folders_of_loose_images_are_read_as_books() {
    let fixture = Fixture::new();
    let png = fixture.archive("zipped.cbz", 30, 40);
    let content = &fixture.config.content_dir;
    let book = content.join("series/Vol.1");
    std::fs::create_dir_all(&book).unwrap();
    std::fs::write(book.join("10.png"), &png).unwrap();
    std::fs::write(book.join("2.png"), &png).unwrap();
    std::fs::write(book.join("notes.txt"), "不是页面").unwrap();
    // 内容目录本身直接放着的图片不成书。
    std::fs::write(content.join("loose.png"), &png).unwrap();
    let app = fixture.app().await;
    let report = app.scan().await.unwrap();
    assert_eq!((report.added, report.skipped, report.failed), (2, 2, 0));

    let store = Store::open(fixture.database()).unwrap();
    let id = store.find_by_path("series/Vol.1").unwrap().unwrap();
    let (archive, pages) = store.archive_with_pages(id).unwrap().unwrap();
    // 文件夹名里的点不是扩展名。
    assert_eq!(archive.metadata.title, "Vol.1");
    let names: Vec<_> = pages.iter().map(|page| page.entry_name.as_str()).collect();
    assert_eq!(names, ["2.png", "10.png"]);
    assert_eq!((pages[0].width, pages[0].height), (30, 40));

    let router = app.router();
    let listed = common::json(&router, &format!("/api/archives/{id}/pages")).await;
    let (status, _, body) = common::request(
        &router,
        "GET",
        listed["pages"][1]["url"].as_str().unwrap(),
        None,
        &[],
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(body, png);

    // 没变就不重新读；补算得出完整指纹。
    assert_eq!(app.scan().await.unwrap().unchanged, 2);
    assert_eq!(app.hash_contents().await.unwrap().hashed, 2);

    // 页面改名时总大小和图片的修改时间都不变，靠文件夹本身的修改时间发现，重新读清单。
    std::fs::rename(book.join("10.png"), book.join("20.png")).unwrap();
    let dir = std::fs::File::open(&book).unwrap();
    dir.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(app.scan().await.unwrap().updated, 1);
    let (_, renamed) = store.archive_with_pages(id).unwrap().unwrap();
    let names: Vec<_> = renamed
        .iter()
        .map(|page| page.entry_name.as_str())
        .collect();
    assert_eq!(names, ["2.png", "20.png"]);

    // 加一页算更新；整个文件夹移走保留原来的记录。
    std::fs::write(book.join("11.png"), &png).unwrap();
    dir.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(app.scan().await.unwrap().updated, 1);
    std::fs::rename(&book, content.join("series/Vol.01")).unwrap();
    assert_eq!(app.scan().await.unwrap().moved, 1);
    assert_eq!(store.find_by_path("series/Vol.01").unwrap(), Some(id));

    // 图片都删了，只剩空目录：算失踪，不再当书。
    for name in ["2.png", "20.png", "11.png"] {
        std::fs::remove_file(content.join("series/Vol.01").join(name)).unwrap();
    }
    app.scan().await.unwrap();
    let issues = common::json(&router, "/api/admin/issues").await;
    assert_eq!(issues["missing"]["items"][0]["id"], id);
}
