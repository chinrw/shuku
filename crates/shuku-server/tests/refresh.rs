//! 后台刷新快照失败后的重试。读库失败用改表名制造：快照读的表暂时不存在，
//! 改回来以后同样的查询就能成功，不需要在服务端代码里留测试钩子。
//! 时钟是暂停的，运行时空闲时自动快进，10 秒的刷新延迟不用真的等。

mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use serde_json::{Value, json as value};
use shuku_index::Query;
use std::time::Duration;
use tokio::time::Instant;

fn rename(fixture: &Fixture, from: &str, to: &str) {
    shuku_store::open_connection(fixture.database())
        .unwrap()
        .execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
        .unwrap();
}

/// 每秒检查一次，直到条件成立，最多等一分钟。
async fn eventually(mut done: impl AsyncFnMut() -> bool) {
    let started = Instant::now();
    while !done().await {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "一分钟内没有刷新"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn progress_refresh_retries_after_a_failed_read() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 10, 20);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    // 扫描提交批次时也通知了刷新器。等它那次刷新跑完，下面就只剩进度上报这一次通知，
    // 失败后能不能再刷新只取决于刷新器自己有没有重新排上。
    tokio::time::sleep(Duration::from_secs(15)).await;
    let started = Instant::now();
    let (status, _, _) = request(
        &router,
        "PUT",
        &format!("/api/archives/{id}/progress"),
        Some(value!({"page": 1, "client_time": 1000})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    rename(&fixture, "progress", "progress_hidden");

    // 第 10 秒的刷新读不到进度表，失败后自己再排一次。
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert_eq!(
        json(&router, "/api/search").await["items"][0]["progress"],
        Value::Null
    );
    rename(&fixture, "progress_hidden", "progress");

    // 之后没有新的写入，靠重试把进度刷进快照。
    eventually(async || json(&router, "/api/search").await["items"][0]["progress"] != Value::Null)
        .await;
    assert!(started.elapsed() >= Duration::from_secs(20));
}

#[tokio::test(start_paused = true)]
async fn a_scan_whose_final_rebuild_fails_is_still_published_by_the_retry() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 10, 20);
    let app = fixture.app().await;
    // 扫描新文件不读标签表，入库照常提交；结束时的整体重建要读标签表，所以失败。
    rename(&fixture, "archive_tag", "archive_tag_hidden");
    assert!(app.scan().await.is_err());
    assert_eq!(app.index().search(&Query::default()).unwrap().total, 0);

    // 第 10 秒的后台重建同样失败。它必须保留“要整体重建”的标记，
    // 否则下次重试只刷新进度，这本书永远进不了搜索结果。
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert_eq!(app.index().search(&Query::default()).unwrap().total, 0);
    rename(&fixture, "archive_tag_hidden", "archive_tag");

    eventually(async || app.index().search(&Query::default()).unwrap().total == 1).await;
    let router = app.router();
    assert_eq!(
        json(&router, "/api/search").await["items"][0]["title"],
        "book"
    );
}
