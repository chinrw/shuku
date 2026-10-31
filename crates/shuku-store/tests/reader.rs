mod common;

use common::{metadata, pages};
use shuku_store::{Error, Reader, ReaderPool, Store, open_connection};

#[test]
fn refuses_databases_that_are_not_migrated_to_the_current_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (x)")
        .unwrap();
    assert!(matches!(
        Reader::open(&path),
        Err(Error::VersionMismatch { found: 0, .. })
    ));
    Store::open(&path).unwrap();
    Reader::open(&path).unwrap();
}

#[test]
fn pooled_readers_see_committed_data_and_are_not_blocked_by_an_open_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    let pool = ReaderPool::new(&path);
    let first = store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    {
        let (one, two) = (pool.get().unwrap(), pool.get().unwrap());
        assert_eq!(one.pages(first).unwrap(), pages());
        assert_eq!(two.find_by_path("one.cbz").unwrap(), Some(first));
    }

    let writer = open_connection(&path).unwrap();
    writer
        .execute_batch("BEGIN IMMEDIATE; UPDATE archive SET title = '未提交'")
        .unwrap();
    let reader = pool.get().unwrap();
    assert_eq!(
        reader.archive(first).unwrap().unwrap().metadata.title,
        "漫画 10"
    );
    writer.execute_batch("COMMIT").unwrap();
    assert_eq!(
        reader.archive(first).unwrap().unwrap().metadata.title,
        "未提交"
    );
}
