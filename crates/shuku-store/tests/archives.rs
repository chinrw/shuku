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
fn reads_list_metadata_in_requested_order() {
    let mut store = Store::open(":memory:").unwrap();
    let first = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let mut second_metadata = metadata("two.cbz");
    second_metadata.title = "第二本".into();
    second_metadata.is_new = false;
    second_metadata.content_hash = Some("full".into());
    let second = store.save_archive(None, &second_metadata, &[]).unwrap();
    store.write_progress(first, 1, 1234).unwrap();
    let list = store.list_metadata(&[second, second + 1, first]).unwrap();
    assert_eq!(
        list.iter().map(|item| item.id).collect::<Vec<_>>(),
        [second, first]
    );
    assert_eq!(list[0].title, "第二本");
    assert_eq!(list[0].page_count, 0);
    assert!(!list[0].is_new);
    assert!(list[0].progress.is_none());
    assert_eq!(list[0].quick_hash, "quick");
    assert_eq!(list[0].content_hash.as_deref(), Some("full"));
    assert_eq!(list[1].title, "漫画 10");
    assert_eq!(list[1].page_count, 2);
    assert!(list[1].is_new);
    assert_eq!(
        list[1].progress,
        store.archive(first).unwrap().unwrap().progress
    );
    assert!(list[1].content_hash.is_none());
    assert!(store.list_metadata(&[]).unwrap().is_empty());
    assert!(store.list_metadata(&[second + 1]).unwrap().is_empty());
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
    let mut broken_pages = pages();
    broken_pages[1].size = u64::MAX;
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
