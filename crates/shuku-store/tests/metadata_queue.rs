mod common;

use common::{metadata, pages};
use shuku_store::{Error, Store};

fn store_with_two_archives() -> (Store, i64, i64) {
    let mut store = Store::open(":memory:").unwrap();
    let one = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let two = store
        .save_archive(None, &metadata("two.cbz"), &pages())
        .unwrap();
    (store, one, two)
}

#[test]
fn each_provider_has_its_own_queue_in_id_order() {
    let (mut store, one, two) = store_with_two_archives();
    store
        .write(|writer| {
            writer.queue_metadata(two, "ehentai")?;
            writer.queue_metadata(one, "ehentai")?;
            writer.queue_metadata(two, "ehentai")?;
            writer.queue_metadata(two, "bangumi")
        })
        .unwrap();
    assert_eq!(
        store.metadata_queue_counts().unwrap(),
        [("bangumi".into(), 1), ("ehentai".into(), 2)]
    );
    assert_eq!(store.next_queued_metadata("ehentai").unwrap(), Some(one));
    assert_eq!(store.next_queued_metadata("bangumi").unwrap(), Some(two));
    assert_eq!(store.next_queued_metadata("anilist").unwrap(), None);
    assert_eq!(
        store
            .metadata_queue(None, Some((two, "bangumi")), 10)
            .unwrap(),
        [(two, "ehentai".into())]
    );
    assert_eq!(
        store.metadata_queue(Some("ehentai"), None, 1).unwrap(),
        [(one, "ehentai".into())]
    );

    let removed = store
        .write(|writer| {
            Ok::<_, Error>([
                writer.dequeue_metadata(one, "ehentai")?,
                writer.dequeue_metadata(one, "ehentai")?,
            ])
        })
        .unwrap();
    assert_eq!(removed, [true, false]);
    assert_eq!(
        store
            .write(|writer| writer.clear_metadata_queue(Some("ehentai")))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .write(|writer| writer.clear_metadata_queue(None))
            .unwrap(),
        1
    );
    assert!(store.metadata_queue_counts().unwrap().is_empty());
}

#[test]
fn an_unknown_archive_cannot_be_queued() {
    let mut store = Store::open(":memory:").unwrap();
    assert!(
        store
            .write(|writer| writer.queue_metadata(42, "ehentai"))
            .is_err()
    );
}

#[test]
fn candidates_are_saved_updated_and_taken_once() {
    let (mut store, one, two) = store_with_two_archives();
    store
        .write(|writer| {
            writer.save_metadata_candidate(one, "bangumi", r#"{"tags":[]}"#, 10)?;
            writer.save_metadata_candidate(two, "anilist", "{}", 20)?;
            // 再查一次时替换。
            writer.save_metadata_candidate(one, "bangumi", r#"{"title":"a"}"#, 30)
        })
        .unwrap();
    let saved = store.metadata_candidate(one, "bangumi").unwrap().unwrap();
    assert_eq!(
        (saved.found.as_str(), saved.fetched_at),
        (r#"{"title":"a"}"#, 30)
    );
    assert_eq!(
        store.metadata_candidate_counts().unwrap(),
        [("anilist".into(), 1), ("bangumi".into(), 1)]
    );
    let listed = store
        .metadata_candidates(None, Some((one, "bangumi")), 10)
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].archive_id, two);

    let updated = store
        .write(|writer| {
            Ok::<_, Error>([
                writer.update_metadata_candidate(one, "bangumi", r#"{"title":"b"}"#)?,
                writer.update_metadata_candidate(one, "anilist", "{}")?,
            ])
        })
        .unwrap();
    assert_eq!(updated, [true, false]);
    let taken = store
        .write(|writer| writer.take_metadata_candidate(one, "bangumi"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (taken.found.as_str(), taken.fetched_at),
        (r#"{"title":"b"}"#, 30)
    );
    assert_eq!(
        store
            .write(|writer| writer.take_metadata_candidate(one, "bangumi"))
            .unwrap(),
        None
    );
}
