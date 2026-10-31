mod common;

use axum::{Router, http::StatusCode};
use common::{Fixture, json, request};
use serde_json::{Value, json as value};
use std::{fs::File, io::Write};
use zip::{ZipWriter, write::SimpleFileOptions};

const COMICINFO: &str = "<?xml version=\"1.0\"?>
<ComicInfo><Title>Info Title</Title><Penciller>Info Artist</Penciller>
<Genre>romance</Genre><Web>https://example.org/book/1</Web></ComicInfo>";

/// 一页 PNG 加一个 ComicInfo.xml 的归档。
fn archive_with_comicinfo(fixture: &Fixture, name: &str, comicinfo: &str) {
    fixture.archive(name, 4, 6);
    let path = fixture.config.content_dir.join(name);
    let mut zip =
        ZipWriter::new_append(File::options().read(true).write(true).open(path).unwrap()).unwrap();
    zip.start_file("ComicInfo.xml", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(comicinfo.as_bytes()).unwrap();
    zip.finish().unwrap();
}

async fn send(router: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let (status, _, bytes) = request(router, method, uri, Some(body), &[]).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tag_names(detail: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for group in detail["tags"].as_array().unwrap() {
        for value in group["values"].as_array().unwrap() {
            names.push(format!(
                "{}:{}",
                group["namespace"].as_str().unwrap(),
                value.as_str().unwrap()
            ));
        }
    }
    names
}

#[tokio::test]
async fn settings_merge_patches_reject_unknown_keys_and_hide_cookies() {
    let fixture = Fixture::new();
    let router = fixture.app().await.router();
    let defaults = json(&router, "/api/admin/settings/metadata").await;
    assert_eq!(defaults["filename"]["auto"], false);
    assert_eq!(defaults["replace_title"], true);

    let (status, saved) = send(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        value!({"filename": {"auto": true}, "ehentai": {"cookies": {"ipb_pass_hash": "secret"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["filename"]["auto"], true);
    assert_eq!(saved["filename"]["regex"], "");
    assert_eq!(saved["ehentai"]["cookies_set"]["ipb_pass_hash"], true);
    assert!(!saved.to_string().contains("secret"));

    for bad in [
        value!({"filename": {"atuo": true}}),
        value!({"filename": {"regex": "(unclosed"}}),
        value!({"translation": {"path": "/etc/passwd"}}),
        value!({"anilist": {"min_tag_rank": 101}}),
        value!({"anilist": {"min_tag_rank": 256}}),
        value!({"bangumi": {"tag_limit": 101}}),
        value!({"replace_title": "yes"}),
    ] {
        let (status, _) = send(&router, "PUT", "/api/admin/settings/metadata", bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    // 设置在重启后还在，cookie 清除后显示未设置。
    let router = fixture.app().await.router();
    let reloaded = json(&router, "/api/admin/settings/metadata").await;
    assert_eq!(reloaded["filename"]["auto"], true);
    let (_, cleared) = send(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        value!({"ehentai": {"cookies": {"ipb_pass_hash": ""}}}),
    )
    .await;
    assert_eq!(cleared["ehentai"]["cookies_set"]["ipb_pass_hash"], false);

    let (status, _, _) = request(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        Some(value!({"filename": {"auto": false}})),
        &[("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn new_books_get_filename_and_comicinfo_metadata_and_join_the_queue() {
    let fixture = Fixture::new();
    archive_with_comicinfo(
        &fixture,
        "(C99) [Circle (Artist)] Book Title [Chinese].cbz",
        COMICINFO,
    );
    let app = fixture.app().await;
    let router = app.router();
    let (status, _) = send(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        value!({"filename": {"auto": true}, "comicinfo": {"auto": true}, "ehentai": {"auto": true}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    app.scan().await.unwrap();

    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    // ComicInfo 在文件名之后套用，标题以它为准；标签两边都加。
    assert_eq!(detail["title"], "Info Title");
    assert_eq!(detail["source_url"], "https://example.org/book/1");
    let names = tag_names(&detail);
    for expected in [
        ":romance",
        "artist:Artist",
        "artist:Info Artist",
        "event:C99",
        "group:Circle",
        "language:Chinese",
    ] {
        assert!(names.contains(&expected.to_owned()), "{names:?}");
    }
    let status = json(&router, "/api/admin/metadata").await;
    assert_eq!(status["providers"]["ehentai"]["queued"], 1);

    // 已经入库的书再扫描不会重跑。
    let (code, cleared) = send(&router, "DELETE", "/api/admin/metadata/queue", Value::Null).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(cleared["removed"], 1);
    app.scan().await.unwrap();
    assert_eq!(
        json(&router, "/api/admin/metadata").await["providers"]["ehentai"]["queued"],
        0
    );
}

#[tokio::test]
async fn books_scanned_with_everything_off_are_left_alone() {
    let fixture = Fixture::new();
    archive_with_comicinfo(&fixture, "[Artist] Title.cbz", COMICINFO);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    assert_eq!(detail["title"], "[Artist] Title");
    assert!(tag_names(&detail).is_empty());
    assert_eq!(
        json(&router, "/api/admin/metadata").await["providers"]["ehentai"]["queued"],
        0
    );
}

#[tokio::test]
async fn a_manual_run_adds_tags_and_reports_what_it_wrote() {
    let fixture = Fixture::new();
    archive_with_comicinfo(&fixture, "[Artist] Title.cbz", COMICINFO);
    fixture.archive("plain.cbz", 4, 6);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let items = json(&router, "/api/search").await["items"].clone();
    let id_of = |title: &str| {
        items
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["title"] == title)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    let (with_info, plain) = (id_of("[Artist] Title"), id_of("plain"));
    send(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        value!({"replace_title": false}),
    )
    .await;

    let uri = format!("/api/archives/{with_info}/metadata");
    let (status, found) = send(&router, "POST", &uri, value!({"provider": "filename"})).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["title"], Value::Null);
    assert_eq!(
        found["tags"],
        value!([{"namespace": "artist", "value": "Artist"}])
    );

    let (status, found) = send(&router, "POST", &uri, value!({"provider": "comicinfo"})).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["source_url"], "https://example.org/book/1");
    let detail = json(&router, &format!("/api/archives/{with_info}")).await;
    assert_eq!(detail["title"], "[Artist] Title");
    assert_eq!(
        tag_names(&detail),
        [":romance", "artist:Artist", "artist:Info Artist"]
    );

    let plain_uri = format!("/api/archives/{plain}/metadata");
    let (status, error) = send(
        &router,
        "POST",
        &plain_uri,
        value!({"provider": "comicinfo"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    let (status, _) = send(
        &router,
        "POST",
        &plain_uri,
        value!({"provider": "filename", "url": "https://e-hentai.org/g/1/a/"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(&router, "POST", &plain_uri, value!({"provider": "nhentai"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        &router,
        "POST",
        "/api/archives/999/metadata",
        value!({"provider": "filename"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_ehentai_run_without_the_translation_database_fails_before_any_request() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 4, 6);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    send(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        value!({"translation": {"enabled": true}}),
    )
    .await;
    let status = json(&router, "/api/admin/metadata").await;
    assert!(
        status["translation"]["error"]
            .as_str()
            .unwrap()
            .contains("db.text.json")
    );

    let (code, error) = send(
        &router,
        "POST",
        &format!("/api/archives/{id}/metadata"),
        value!({"provider": "ehentai", "url": "https://e-hentai.org/g/1/abc/"}),
    )
    .await;
    assert_eq!(code, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
    assert!(error["error"].as_str().unwrap().contains("翻译数据库"));

    std::fs::write(
        fixture.config.data_dir.join("db.text.json"),
        r#"{"data":[{"namespace":"female","frontMatters":{"name":"女性"},"data":{"x":{"name":"叉"}}}]}"#,
    )
    .unwrap();
    let status = json(&router, "/api/admin/metadata").await;
    assert_eq!(status["translation"]["entries"], 1);
}

#[tokio::test]
async fn books_migrated_from_lrr_keep_their_metadata_and_skip_the_queue() {
    let fixture = Fixture::new();
    archive_with_comicinfo(&fixture, "[Artist] Staged.cbz", COMICINFO);
    std::fs::create_dir_all(&fixture.config.data_dir).unwrap();
    shuku_store::Store::open(fixture.database())
        .unwrap()
        .write(|writer| {
            writer.stage_lrr_pending(
                "[Artist] Staged.cbz",
                &shuku_store::LrrPending {
                    archive: shuku_store::LrrArchive {
                        lrr_id: "a".repeat(40),
                        title: "LRR 的标题".into(),
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
            )
        })
        .unwrap();
    let app = fixture.app().await;
    let router = app.router();
    send(
        &router,
        "PUT",
        "/api/admin/settings/metadata",
        value!({"filename": {"auto": true}, "comicinfo": {"auto": true}, "ehentai": {"auto": true}}),
    )
    .await;
    app.scan().await.unwrap();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    assert_eq!(detail["title"], "LRR 的标题");
    assert!(tag_names(&detail).is_empty());
    assert_eq!(
        json(&router, "/api/admin/metadata").await["providers"]["ehentai"]["queued"],
        0
    );
}

/// 一本已入库、没有标签的书和它的 ID。
async fn one_book(fixture: &Fixture, name: &str) -> (shuku_server::App, Router, i64) {
    fixture.archive(name, 4, 6);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    (app, router, id)
}

#[tokio::test]
async fn a_preview_returns_what_would_be_written_without_writing() {
    let fixture = Fixture::new();
    let (_app, router, id) = one_book(&fixture, "[Artist] Title.cbz").await;
    let (status, found) = send(
        &router,
        "POST",
        &format!("/api/archives/{id}/metadata"),
        value!({"provider": "filename", "apply": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["title"], "Title");
    assert_eq!(
        found["tags"],
        value!([{"namespace": "artist", "value": "Artist"}])
    );
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    assert_eq!(detail["title"], "[Artist] Title");
    assert!(tag_names(&detail).is_empty());
}

#[tokio::test]
async fn a_preview_leaves_out_a_summary_and_source_the_book_already_has() {
    let fixture = Fixture::new();
    archive_with_comicinfo(&fixture, "book.cbz", COMICINFO);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let id = json(&router, "/api/search").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let uri = format!("/api/archives/{id}/metadata");
    let body = value!({"provider": "comicinfo", "apply": false});
    let (_, before) = send(&router, "POST", &uri, body.clone()).await;
    assert_eq!(before["source_url"], "https://example.org/book/1");
    let (status, _) = send(
        &router,
        "PATCH",
        &format!("/api/archives/{id}"),
        value!({"summary": "已有的简介", "source_url": "https://example.org/other"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = send(&router, "POST", &uri, body).await;
    assert_eq!(after["summary"], Value::Null);
    assert_eq!(after["source_url"], Value::Null);
    assert_eq!(after["title"], "Info Title");
}

#[tokio::test]
async fn books_can_be_queued_for_an_online_source_through_the_api() {
    let fixture = Fixture::new();
    let (_app, router, id) = one_book(&fixture, "book.cbz").await;
    let (status, queued) = send(
        &router,
        "POST",
        "/api/metadata/queue",
        value!({"provider": "bangumi", "ids": [id, 999]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{queued}");
    assert_eq!(queued, value!({"queued": [id], "not_found": [999]}));
    for bad in [
        value!({"provider": "filename", "ids": [id]}),
        value!({"provider": "bangumi", "ids": []}),
    ] {
        let (status, _) = send(&router, "POST", "/api/metadata/queue", bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    let listed = json(&router, "/api/metadata/queue").await;
    assert_eq!(listed["counts"], value!({"bangumi": 1}));
    assert_eq!(
        listed["items"],
        value!([{"id": id, "provider": "bangumi", "title": "book"}])
    );
    let page = json(&router, &format!("/api/metadata/queue?after={id}")).await;
    assert_eq!(page["items"], value!([]));
    let status = json(&router, "/api/admin/metadata").await;
    assert_eq!(status["providers"]["bangumi"]["queued"], 1);
    assert_eq!(status["providers"]["anilist"]["queued"], 0);

    let (_, cleared) = send(
        &router,
        "DELETE",
        "/api/metadata/queue?provider=bangumi",
        Value::Null,
    )
    .await;
    assert_eq!(cleared["removed"], 1);
}

#[tokio::test]
async fn an_agent_can_read_rewrite_approve_and_reject_candidates() {
    let fixture = Fixture::new();
    let (_app, router, id) = one_book(&fixture, "葬送的芙莉莲 第01卷.cbz").await;
    // 后台查到的结果；测试里不联网，直接写进候选表。
    shuku_store::Store::open(fixture.database())
        .unwrap()
        .write(|writer| {
            writer.save_metadata_candidate(
                id,
                "anilist",
                r#"{"tags": [{"namespace": "series", "value": "Sousou no Frieren"},
                             {"namespace": "", "value": "Magic"}],
                    "summary": "The adventure is over.",
                    "source_url": "https://anilist.co/manga/118586"}"#,
                1_700_000_000_000,
            )?;
            writer.save_metadata_candidate(id, "bangumi", r#"{"tags": []}"#, 1_700_000_000_001)
        })
        .unwrap();

    let listed = json(&router, "/api/metadata/candidates?provider=anilist").await;
    let item = &listed["items"][0];
    assert_eq!(item["id"], id);
    assert_eq!(item["archive"]["title"], "葬送的芙莉莲 第01卷");
    assert_eq!(item["found"]["summary"], "The adventure is over.");
    assert_eq!(listed["more"], false);

    let uri = format!("/api/metadata/candidates/{id}/anilist");
    let translated = value!({
        "tags": [{"namespace": "series", "value": "葬送的芙莉莲"}, {"namespace": "", "value": "魔法"}],
        "summary": "冒险结束了。",
        "source_url": "https://anilist.co/manga/118586",
    });
    for bad in [
        value!({"tags": [{"namespace": "", "value": "a, b"}]}),
        value!({"tagz": []}),
    ] {
        let (status, _) = send(&router, "PUT", &uri, bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = send(&router, "PUT", &uri, translated.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        json(&router, &uri).await["found"]["summary"],
        "冒险结束了。"
    );

    let (status, applied) = send(&router, "POST", &format!("{uri}/approve"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["summary"], "冒险结束了。");
    let detail = json(&router, &format!("/api/archives/{id}")).await;
    assert_eq!(detail["summary"], "冒险结束了。");
    assert_eq!(detail["source_url"], "https://anilist.co/manga/118586");
    assert_eq!(tag_names(&detail), [":魔法", "series:葬送的芙莉莲"]);
    let (status, _, _) = request(&router, "GET", &uri, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let bangumi = format!("/api/metadata/candidates/{id}/bangumi");
    let (status, _, _) = request(&router, "DELETE", &bangumi, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = request(&router, "DELETE", &bangumi, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = request(
        &router,
        "GET",
        &format!("/api/metadata/candidates/{id}/filename"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
