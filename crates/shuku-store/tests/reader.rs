mod common;

use common::{metadata, pages};
use shuku_store::{Error, Reader, ReaderPool, Store, open_connection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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

/// FULL checkpoint 的忙等处理：第一次被调用时它已经拿到 checkpoint 锁、在等旧读事务，
/// 置位 `CHECKPOINTER_WAITING`；之后一直等到测试置位 `CHECKPOINTER_RELEASED`。
static CHECKPOINTER_WAITING: AtomicBool = AtomicBool::new(false);
static CHECKPOINTER_RELEASED: AtomicBool = AtomicBool::new(false);

fn hold_checkpoint_lock(calls: i32) -> bool {
    CHECKPOINTER_WAITING.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(5));
    // 测试卡住时最多等约 10 秒，不让测试进程挂死。
    !CHECKPOINTER_RELEASED.load(Ordering::SeqCst) && calls < 2000
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
    // 旧快照上的读事务让 FULL checkpoint 进入忙等，忙等期间它占着 checkpoint 锁。
    let old_read = open_connection(&path).unwrap();
    old_read
        .execute_batch("BEGIN; SELECT count(*) FROM archive;")
        .unwrap();
    store
        .save_archive(None, &metadata("two.cbz"), &pages())
        .unwrap();
    let checkpointer = std::thread::spawn({
        let path = path.clone();
        move || {
            let connection = open_connection(&path).unwrap();
            connection.busy_handler(Some(hold_checkpoint_lock)).unwrap();
            connection
                .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !CHECKPOINTER_WAITING.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "FULL checkpoint 没有进入忙等");
        std::thread::sleep(Duration::from_millis(1));
    }
    // 这时 PASSIVE checkpoint 拿不到锁，SQLite 返回 (1, -1, -1)。
    assert!(!store.checkpoint().unwrap());
    CHECKPOINTER_RELEASED.store(true, Ordering::SeqCst);
    checkpointer.join().unwrap();
    old_read.execute_batch("COMMIT").unwrap();
    assert!(store.checkpoint().unwrap());
}

#[test]
fn stats_count_archives_hashes_progress_and_staged_records() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path().join("shuku.sqlite")).unwrap();
    let empty = store.stats().unwrap();
    assert_eq!((empty.archives, empty.pages, empty.bytes), (0, 0, 0));

    let mut hashed = metadata("a.cbz");
    hashed.content_hash = Some("same".into());
    hashed.lrr_id = Some("a".repeat(40));
    let a = store.save_archive(None, &hashed, &pages()).unwrap();
    store
        .save_archive(None, &metadata("b.cbz"), &pages()[..1])
        .unwrap();
    store.write_progress(a, 1, 1000).unwrap();
    store
        .write(|writer| {
            writer.stage_lrr_pending(
                "later.cbz",
                &shuku_store::LrrPending {
                    archive: shuku_store::LrrArchive {
                        lrr_id: "b".repeat(40),
                        title: String::new(),
                        summary: String::new(),
                        date_added: None,
                        source_url: None,
                        is_new: true,
                        tags: Vec::new(),
                    },
                    progress: None,
                    categories: Vec::new(),
                    tankoubons: Vec::new(),
                },
            )
        })
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.archives, 2);
    assert_eq!(stats.pages, 3);
    assert_eq!(stats.bytes, 2000);
    assert_eq!(stats.hashed, 1);
    assert_eq!(stats.lrr_imported, 1);
    assert_eq!(stats.with_progress, 1);
    assert_eq!(stats.lrr_pending, 1);
    assert_eq!((stats.categories, stats.tankoubons), (0, 0));
}

#[test]
fn duplicates_group_archives_by_full_content_hash() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path().join("shuku.sqlite")).unwrap();
    let save = |store: &mut Store, path: &str, hash: Option<&str>| {
        let mut archive = metadata(path);
        archive.content_hash = hash.map(Into::into);
        store.save_archive(None, &archive, &pages()).unwrap()
    };
    let b2 = save(&mut store, "z/b.cbz", Some("bbb"));
    let a1 = save(&mut store, "y/a.cbz", Some("aaa"));
    let a2 = save(&mut store, "x/a.cbz", Some("aaa"));
    let b1 = save(&mut store, "b.cbz", Some("bbb"));
    save(&mut store, "unique.cbz", Some("ccc"));
    // 还没算出完整指纹的不算重复，即使快速指纹相同。
    save(&mut store, "p.cbz", None);
    save(&mut store, "q.cbz", None);

    let ids = |groups: &[Vec<shuku_store::DuplicateFile>]| -> Vec<Vec<i64>> {
        groups
            .iter()
            .map(|group| group.iter().map(|file| file.id).collect())
            .collect()
    };
    let (total, groups) = store.duplicates(10).unwrap();
    assert_eq!(total, 2);
    assert_eq!(ids(&groups), [vec![a2, a1], vec![b1, b2]]);
    assert_eq!(groups[0][0].path, "x/a.cbz");
    assert_eq!(groups[0][0].file_size, 1000);

    let (total, groups) = store.duplicates(1).unwrap();
    assert_eq!(total, 2);
    assert_eq!(ids(&groups), [vec![a2, a1]]);
}

#[test]
fn archives_with_zero_sized_pages_are_listed_as_broken() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path().join("shuku.sqlite")).unwrap();
    let mut with_broken = pages();
    with_broken[1].width = 0;
    with_broken[1].height = 0;
    let broken = store
        .save_archive(None, &metadata("b/broken.cbz"), &with_broken)
        .unwrap();
    store
        .save_archive(None, &metadata("a/fine.cbz"), &pages())
        .unwrap();

    let (total, archives) = store.broken_archives(10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(archives[0].id, broken);
    assert_eq!(archives[0].path, "b/broken.cbz");
    assert_eq!(archives[0].pages, ["10.jpg"]);
    assert_eq!(store.stats().unwrap().with_broken_pages, 1);

    // 重新扫描后页面都好了，就不再列出。
    store
        .save_archive(Some(broken), &metadata("b/broken.cbz"), &pages())
        .unwrap();
    assert_eq!(store.broken_archives(10).unwrap().0, 0);
}

#[test]
fn settings_are_stored_as_json_per_group() {
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Scan {
        hidden: bool,
        hours: u32,
    }
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path().join("shuku.sqlite")).unwrap();
    assert_eq!(store.setting::<Scan>("scan").unwrap(), None);
    let scan = Scan {
        hidden: true,
        hours: 24,
    };
    store
        .write(|writer| {
            writer.set_setting(
                "scan",
                &Scan {
                    hidden: false,
                    hours: 1,
                },
            )?;
            writer.set_setting("scan", &scan)?;
            writer.set_setting("other", &"x")
        })
        .unwrap();
    assert_eq!(store.setting::<Scan>("scan").unwrap(), Some(scan));
    assert!(matches!(
        store.setting::<u32>("other"),
        Err(Error::Setting { key, .. }) if key == "other"
    ));
    store
        .write(|writer| writer.delete_setting("other"))
        .unwrap();
    assert_eq!(store.setting::<String>("other").unwrap(), None);
}
