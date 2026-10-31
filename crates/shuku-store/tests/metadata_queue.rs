mod common;

use common::{metadata, pages};
use shuku_store::{Error, Store};

#[test]
fn queued_archives_come_out_in_id_order_once_each() {
    let mut store = Store::open(":memory:").unwrap();
    let one = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let two = store
        .save_archive(None, &metadata("two.cbz"), &pages())
        .unwrap();
    store
        .write(|writer| {
            writer.queue_metadata(two)?;
            writer.queue_metadata(one)?;
            writer.queue_metadata(two)
        })
        .unwrap();
    assert_eq!(store.metadata_queue_len().unwrap(), 2);
    assert_eq!(store.next_queued_metadata().unwrap(), Some(one));

    let removed = store
        .write(|writer| {
            Ok::<_, Error>([writer.dequeue_metadata(one)?, writer.dequeue_metadata(one)?])
        })
        .unwrap();
    assert_eq!(removed, [true, false]);
    assert_eq!(store.next_queued_metadata().unwrap(), Some(two));

    assert_eq!(
        store.write(|writer| writer.clear_metadata_queue()).unwrap(),
        1
    );
    assert_eq!(store.next_queued_metadata().unwrap(), None);
}

#[test]
fn an_unknown_archive_cannot_be_queued() {
    let mut store = Store::open(":memory:").unwrap();
    assert!(store.write(|writer| writer.queue_metadata(42)).is_err());
}
