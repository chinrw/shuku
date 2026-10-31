mod common;

use axum::Router;
use common::{Fixture, json};
use serde_json::Value;
use std::{
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};

const QUIET: Duration = Duration::from_millis(200);

async fn status(router: &Router) -> Value {
    json(router, "/api/admin/status").await["scan"].clone()
}

/// 等 `check` 成立，最多 10 秒。
async fn eventually(router: &Router, what: &str, check: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let scan = status(router).await;
        if check(&scan) {
            return scan;
        }
        assert!(Instant::now() < deadline, "{what}: {scan}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn new_and_deleted_files_trigger_a_scan_but_scanning_does_not() {
    let fixture = Fixture::new();
    fixture.archive_with_pages("a.cbz", 2);
    // 内容目录里有一个链接到外面的目录，监视开始前就在。
    let outside = fixture.root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.config.content_dir.join("link")).unwrap();
    // 下载工具的临时目录，只有它自己能读。不能让整个监视失败。
    let locked = fixture.config.content_dir.join(".locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let watcher = app.watch_content(QUIET, Duration::from_secs(2)).unwrap();
    let first = status(&router).await["started_at"].clone();

    fixture.archive_with_pages("sub/b.cbz", 3);
    let scan = eventually(&router, "新文件入库", |scan| {
        scan["started_at"] != first && scan["running"] == false
    })
    .await;
    assert_eq!(scan["report"]["added"], 1);
    assert_eq!(json(&router, "/api/search").await["total"], 2);

    // 新建的子目录在扫描前加上了监视，里面后来的文件也会触发扫描。
    let previous = scan["started_at"].clone();
    fixture.archive_with_pages("sub/c.cbz", 1);
    let scan = eventually(&router, "新子目录里的文件入库", |scan| {
        scan["started_at"] != previous && scan["running"] == false
    })
    .await;
    assert_eq!(scan["report"]["added"], 1);

    // 扫描要打开每个文件。这些事件不能触发下一次扫描。
    tokio::time::sleep(QUIET * 4).await;
    assert_eq!(status(&router).await["started_at"], scan["started_at"]);

    // 隐藏目录里的变化不算。
    fixture.archive_with_pages(".partial/x.cbz", 1);
    tokio::time::sleep(QUIET * 4).await;
    assert_eq!(status(&router).await["started_at"], scan["started_at"]);

    std::fs::remove_file(fixture.config.content_dir.join("a.cbz")).unwrap();
    let previous = scan["started_at"].clone();
    eventually(&router, "删除的文件列为失踪", |scan| {
        scan["started_at"] != previous && scan["running"] == false && scan["issues"]["missing"] == 1
    })
    .await;

    // 和扫描一样不跟随符号链接：链接到内容目录以外的目录里的变化不算。
    let linked = status(&router).await["started_at"].clone();
    std::fs::write(outside.join("x.cbz"), b"not a zip").unwrap();
    tokio::time::sleep(QUIET * 4).await;
    assert_eq!(status(&router).await["started_at"], linked);

    // 句柄丢掉以后不再扫描。
    drop(watcher);
    fixture.archive_with_pages("d.cbz", 1);
    tokio::time::sleep(QUIET * 4).await;
    assert_eq!(status(&router).await["started_at"], linked);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
}
