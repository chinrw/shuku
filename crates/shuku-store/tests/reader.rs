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
        let (archive, listed) = one.archive_with_pages(first).unwrap().unwrap();
        assert_eq!((archive.id, listed), (first, pages()));
        assert!(one.archive_with_pages(first + 1).unwrap().is_none());
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

#[test]
fn manual_checkpoints_leave_pages_still_needed_by_an_older_read() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.disable_auto_checkpoint().unwrap();
    store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    let old_read = open_connection(&path).unwrap();
    old_read
        .execute_batch("BEGIN; SELECT count(*) FROM archive;")
        .unwrap();
    store
        .save_archive(None, &metadata("two.cbz"), &pages())
        .unwrap();
    assert!(!store.checkpoint().unwrap());
    old_read.execute_batch("COMMIT").unwrap();
    assert!(store.checkpoint().unwrap());
}

#[test]
fn savepoints_undo_one_step_and_failed_writes_undo_the_whole_transaction() {
    let mut store = Store::open(":memory:").unwrap();
    let (kept, missing) = store
        .write(|writer| {
            let kept = writer.save_archive(None, &metadata("kept.cbz"), &pages())?;
            let missing = writer.savepoint(|writer| {
                writer.save_archive(None, &metadata("undone.cbz"), &[])?;
                writer.write_progress(i64::MAX, 0, 1)
            })?;
            Ok::<_, Error>((kept, missing))
        })
        .unwrap();
    assert!(matches!(missing, Err(Error::ArchiveNotFound(i64::MAX))));
    assert_eq!(store.find_by_path("kept.cbz").unwrap(), Some(kept));
    assert_eq!(store.find_by_path("undone.cbz").unwrap(), None);

    let failed: Result<(), Error> = store.write(|writer| {
        writer.save_archive(None, &metadata("rolled-back.cbz"), &[])?;
        Err(Error::ArchiveNotFound(0))
    });
    assert!(failed.is_err());
    assert_eq!(store.find_by_path("rolled-back.cbz").unwrap(), None);
}

#[test]
fn a_checkpoint_blocked_by_another_checkpointer_is_not_complete() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.disable_auto_checkpoint().unwrap();
    store
        .save_archive(None, &metadata("one.cbz"), &pages())
        .unwrap();
    // 旧快照上的读事务让 FULL checkpoint 一直等，等待期间它占着 checkpoint 锁。
    let old_read = open_connection(&path).unwrap();
    old_read
        .execute_batch("BEGIN; SELECT count(*) FROM archive;")
        .unwrap();
    store
        .save_archive(None, &metadata("two.cbz"), &pages())
        .unwrap();
    // 探测用的连接要在 FULL checkpoint 开始前打开：打开连接时的迁移检查要写锁，
    // 而 FULL 等待期间占着写锁。
    let probe = open_connection(&path).unwrap();
    let (started, blocked) = std::sync::mpsc::channel();
    let checkpointer = std::thread::spawn({
        let path = path.clone();
        move || {
            let connection = open_connection(&path).unwrap();
            connection
                .busy_timeout(std::time::Duration::from_secs(3))
                .unwrap();
            started.send(()).unwrap();
            connection
                .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        }
    });
    blocked.recv().unwrap();
    // 等另一个连接拿到锁并进入忙等：这时 PASSIVE checkpoint 返回 (1, -1, -1)。
    // 旧读事务本身也会让 checkpoint 写不完，所以先确认处于锁竞争，再检查返回值。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let busy: i64 = probe
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| row.get(0))
            .unwrap();
        if busy == 1 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "没有等到锁竞争");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!store.checkpoint().unwrap());
    old_read.execute_batch("COMMIT").unwrap();
    checkpointer.join().unwrap();
    assert!(store.checkpoint().unwrap());
}
