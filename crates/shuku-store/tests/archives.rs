mod common;

use common::{metadata, pages};
use shuku_store::{Error, Store, Tag, open_connection};

fn tag(namespace: &str, value: &str) -> Tag {
    Tag {
        namespace: namespace.into(),
        value: value.into(),
    }
}

#[test]
fn lists_file_identity_for_scanning_without_loading_pages_or_tags() {
    let mut store = Store::open(":memory:").unwrap();
    assert!(store.scan_entries().unwrap().is_empty());
    let first = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let second = store.save_archive(None, &metadata("two.cbz"), &[]).unwrap();
    let entries = store.scan_entries().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, first);
    assert_eq!(entries[0].path, "one.cbz");
    assert_eq!(entries[0].file_size, 1000);
    assert_eq!(entries[0].mtime, 1_700_000_001);
    assert_eq!(entries[0].quick_hash, "quick");
    assert_eq!(entries[1].id, second);
}

#[test]
fn reads_library_listing_with_sorted_tag_positions_and_progress() {
    let mut store = Store::open(":memory:").unwrap();
    let first = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let mut second_metadata = metadata("two.cbz");
    second_metadata.title = "第二本".into();
    second_metadata.is_new = false;
    second_metadata.source_url = Some("https://example.org/2".into());
    let second = store.save_archive(None, &second_metadata, &[]).unwrap();
    store.write_progress(first, 1, 1234).unwrap();
    store
        .replace_tags(first, &[tag("language", "中文"), tag("Artist", "Alpha")])
        .unwrap();
    store
        .replace_tags(second, &[tag("artist", "beta")])
        .unwrap();

    let library = store.library().unwrap();
    assert_eq!(
        library
            .tags
            .iter()
            .map(|entry| (entry.tag.clone(), entry.full.as_str()))
            .collect::<Vec<_>>(),
        [
            (tag("Artist", "Alpha"), "artist:alpha"),
            (tag("artist", "beta"), "artist:beta"),
            (tag("language", "中文"), "language:中文"),
        ]
    );
    let ids: Vec<_> = library.archives.iter().map(|archive| archive.id).collect();
    assert_eq!(ids, [first, second]);
    let (one, two) = (&library.archives[0], &library.archives[1]);
    assert_eq!(one.title, "漫画 10");
    assert_eq!(one.page_count, 2);
    assert!(one.is_new);
    assert_eq!(one.quick_hash, "quick");
    assert_eq!(one.date_added, 1_700_000_000);
    assert_eq!(one.source_url, None);
    // 与详情一致，按小写完整名排序。
    assert_eq!(one.tags, [0, 2]);
    assert_eq!(two.title, "第二本");
    assert_eq!(two.page_count, 0);
    assert!(!two.is_new);
    assert_eq!(two.source_url.as_deref(), Some("https://example.org/2"));
    assert_eq!(two.tags, [1]);
    assert_eq!(
        library.progress.get(&first),
        store.archive(first).unwrap().unwrap().progress.as_ref()
    );
    assert_eq!(library.progress.len(), 1);
    assert_eq!(store.progress().unwrap(), library.progress);
}

#[test]
fn accepts_only_strictly_newer_progress_and_keeps_it_on_archive_updates() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    let id = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    assert!(store.archive(id).unwrap().unwrap().progress.is_none());
    let before = now();
    assert!(store.write_progress(id, 1, 2_000).unwrap());
    let original = store.archive(id).unwrap().unwrap().progress.unwrap();
    assert_eq!(original.page, 1);
    assert_eq!(original.client_time, 2_000);
    assert!((before..=now()).contains(&original.updated_at));
    let mut other_connection = Store::open(&path).unwrap();
    assert!(!other_connection.write_progress(id, 0, 1_999).unwrap());
    assert!(!other_connection.write_progress(id, 0, 2_000).unwrap());
    assert_eq!(store.archive(id).unwrap().unwrap().progress, Some(original));
    assert!(other_connection.write_progress(id, 0, 2_001).unwrap());
    let latest = store.archive(id).unwrap().unwrap().progress.unwrap();
    assert_eq!(latest.page, 0);
    assert_eq!(latest.client_time, 2_001);
    assert!(!store.write_progress(id, 1, 2_000).unwrap());
    store
        .save_archive(Some(id), &metadata("moved.cbz"), &pages())
        .unwrap();
    store.replace_tags(id, &[tag("", "read")]).unwrap();
    assert_eq!(
        store.archive(id).unwrap().unwrap().progress,
        Some(latest.clone())
    );
    assert!(matches!(
        store.write_progress(id + 1, 0, 3_000),
        Err(Error::ArchiveNotFound(_))
    ));
    drop(store);
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .archive(id)
            .unwrap()
            .unwrap()
            .progress,
        Some(latest)
    );
}

#[test]
fn marks_finished_by_hand_without_moving_existing_progress() {
    let mut store = Store::open(":memory:").unwrap();
    let unread = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let reading = store
        .save_archive(None, &metadata("two.cbz"), &pages())
        .unwrap();
    let progress = |store: &Store, id| store.archive(id).unwrap().unwrap().progress.unwrap();

    // 没读过的书记在最后一页。
    store.set_finished(unread, true).unwrap();
    let marked = progress(&store, unread);
    assert_eq!((marked.page, marked.finished), (1, true));

    // 读到一半的书只改标记，页码和翻页时间不变，之后的上报照常比较时间。
    store.write_progress(reading, 0, 5_000).unwrap();
    store.set_finished(reading, true).unwrap();
    let marked = progress(&store, reading);
    assert_eq!(
        (marked.page, marked.client_time, marked.finished),
        (0, 5_000, true)
    );
    assert!(!store.write_progress(reading, 1, 4_000).unwrap());

    store.set_finished(reading, false).unwrap();
    let cleared = progress(&store, reading);
    assert_eq!(
        (cleared.page, cleared.client_time, cleared.finished),
        (0, 5_000, false)
    );

    assert!(matches!(
        store.set_finished(reading + 100, true),
        Err(Error::ArchiveNotFound(_))
    ));
}

#[test]
fn replaces_normalized_tags_atomically_without_affecting_other_archives() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    let id = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let other = store.save_archive(None, &metadata("two.cbz"), &[]).unwrap();
    store
        .replace_tags(
            id,
            &[
                tag("Artist", "ÄLPHA"),
                tag("artist", "älpha"),
                tag("", "独立"),
            ],
        )
        .unwrap();
    let original = vec![tag("Artist", "ÄLPHA"), tag("", "独立")];
    assert_eq!(store.archive(id).unwrap().unwrap().tags, original);
    store
        .replace_tags(other, &[tag("ARTIST", "ÄLPHA")])
        .unwrap();
    assert_eq!(
        store.archive(other).unwrap().unwrap().tags,
        vec![tag("Artist", "ÄLPHA")]
    );

    let connection = open_connection(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_tag BEFORE INSERT ON tag WHEN NEW.full = 'reject' BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    assert!(
        store
            .replace_tags(id, &[tag("", "temporary"), tag("", "reject")])
            .is_err()
    );
    assert_eq!(store.archive(id).unwrap().unwrap().tags, original);
    let inserted: i64 = connection
        .query_row(
            "SELECT count(*) FROM tag WHERE full = 'temporary'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(inserted, 0);

    store
        .save_archive(Some(id), &metadata("moved.cbz"), &[])
        .unwrap();
    assert_eq!(store.archive(id).unwrap().unwrap().tags, original);
    store.replace_tags(id, &[tag("language", "中文")]).unwrap();
    assert_eq!(
        store.archive(id).unwrap().unwrap().tags,
        vec![tag("language", "中文")]
    );
    assert_eq!(
        store.archive(other).unwrap().unwrap().tags,
        vec![tag("Artist", "ÄLPHA")]
    );
    store.replace_tags(id, &[]).unwrap();
    assert!(store.archive(id).unwrap().unwrap().tags.is_empty());
}

#[test]
fn failed_writes_preserve_archive_and_pages_and_never_reuse_deleted_ids() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    let original = metadata("original.cbz");
    let original_pages = pages();
    let id = store
        .save_archive(None, &original, &original_pages)
        .unwrap();
    let changed = metadata("changed.cbz");
    // 页面清单写在归档行之后。让它失败，检查已经写入的归档行也一起回滚。
    open_connection(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_insert BEFORE INSERT ON page_list WHEN length(NEW.pages) > 200
                BEGIN SELECT RAISE(ABORT, 'test failure'); END;
             CREATE TRIGGER reject_update BEFORE UPDATE ON page_list WHEN length(NEW.pages) > 200
                BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        )
        .unwrap();
    let mut broken_pages = pages();
    broken_pages[1].entry_name = "长".repeat(100);
    assert!(
        store
            .save_archive(Some(id), &changed, &broken_pages)
            .is_err()
    );
    assert_eq!(store.archive(id).unwrap().unwrap().metadata, original);
    assert_eq!(store.pages(id).unwrap(), original_pages);
    assert_eq!(store.find_by_path(&changed.path).unwrap(), None);
    assert!(store.save_archive(None, &changed, &broken_pages).is_err());
    assert_eq!(store.find_by_path(&changed.path).unwrap(), None);

    assert!(store.save_archive(None, &original, &[]).is_err());
    let second = store.save_archive(None, &changed, &[]).unwrap();
    assert!(store.save_archive(Some(id), &changed, &[]).is_err());
    assert_eq!(store.pages(id).unwrap(), original_pages);
    assert!(
        matches!(store.save_archive(Some(second + 10), &metadata("missing.cbz"), &[]), Err(Error::ArchiveNotFound(missing)) if missing == second + 10)
    );
    let connection = open_connection(&path).unwrap();
    connection
        .execute("DELETE FROM archive WHERE id = ?1", [second])
        .unwrap();
    assert!(
        matches!(store.save_archive(Some(second), &changed, &[]), Err(Error::ArchiveNotFound(missing)) if missing == second)
    );
    let next = store.save_archive(None, &changed, &[]).unwrap();
    assert!(next > second);
}

#[test]
fn saves_reads_and_updates_archive_with_its_ordered_pages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    let original = metadata("作者/原名.cbz");
    let original_pages = pages();
    let id = store
        .save_archive(None, &original, &original_pages)
        .unwrap();
    assert!(id > 0);
    let archive = store.archive(id).unwrap().unwrap();
    assert_eq!(archive.id, id);
    assert_eq!(archive.metadata, original);
    assert_eq!(archive.page_count, 2);
    assert!(archive.tags.is_empty());
    assert!(archive.progress.is_none());
    assert_eq!(store.pages(id).unwrap(), original_pages);
    assert_eq!(store.find_by_path(&original.path).unwrap(), Some(id));
    assert!(store.archive(id + 1).unwrap().is_none());
    assert!(store.pages(id + 1).unwrap().is_empty());
    assert_eq!(store.find_by_path("missing.cbz").unwrap(), None);
    drop(store);

    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.archive(id).unwrap().unwrap(), archive);
    let changed = shuku_store::ArchiveMetadata {
        path: "改名.zip".into(),
        title: "新标题".into(),
        summary: String::new(),
        file_size: 4321,
        mtime: 1_700_000_100,
        quick_hash: "new-quick".into(),
        content_hash: Some("full".into()),
        date_added: 123,
        source_url: Some("https://example.com/book".into()),
        is_new: false,
        lrr_id: Some("legacy-id".into()),
    };
    let replacement_pages = vec![original_pages[1].clone()];
    assert_eq!(
        store
            .save_archive(Some(id), &changed, &replacement_pages)
            .unwrap(),
        id
    );
    assert_eq!(store.find_by_path(&original.path).unwrap(), None);
    assert_eq!(store.find_by_path(&changed.path).unwrap(), Some(id));
    let updated = store.archive(id).unwrap().unwrap();
    assert_eq!(updated.metadata, changed);
    assert_eq!(updated.page_count, 1);
    assert_eq!(store.pages(id).unwrap(), replacement_pages);

    store.save_archive(Some(id), &original, &[]).unwrap();
    assert_eq!(store.archive(id).unwrap().unwrap().metadata, original);
    assert_eq!(store.archive(id).unwrap().unwrap().page_count, 0);
    assert!(store.pages(id).unwrap().is_empty());
}

#[test]
fn content_hashes_are_listed_in_id_order_and_written_only_for_unchanged_files() {
    let mut store = Store::open(":memory:").unwrap();
    let mut hashed = metadata("hashed.cbz");
    hashed.content_hash = Some("done".into());
    store.save_archive(None, &hashed, &[]).unwrap();
    let ids: Vec<i64> = ["a.cbz", "b.cbz", "c.cbz"]
        .iter()
        .map(|path| store.save_archive(None, &metadata(path), &[]).unwrap())
        .collect();

    let first = store.missing_content_hashes(0, 2).unwrap();
    assert_eq!(
        first.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        ids[..2]
    );
    let rest = store.missing_content_hashes(first[1].id, 2).unwrap();
    assert_eq!(
        rest.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        ids[2..]
    );

    // 计算期间 b.cbz 被扫描更新过，旧的文件身份不再匹配，不能写入。
    let mut changed = metadata("b.cbz");
    changed.mtime += 1;
    store.save_archive(Some(ids[1]), &changed, &[]).unwrap();
    assert!(store.set_content_hash(&first[0], "hash-a").unwrap());
    assert!(!store.set_content_hash(&first[1], "hash-b").unwrap());
    // 已经有指纹的归档不会被覆盖。
    assert!(!store.set_content_hash(&first[0], "again").unwrap());

    assert_eq!(
        store
            .archive(ids[0])
            .unwrap()
            .unwrap()
            .metadata
            .content_hash,
        Some("hash-a".into())
    );
    assert_eq!(
        store
            .archive(ids[1])
            .unwrap()
            .unwrap()
            .metadata
            .content_hash,
        None
    );
    assert_eq!(
        store
            .missing_content_hashes(0, 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        ids[1..]
    );
}
