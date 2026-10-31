mod common;

use common::{metadata, pages};
use rusqlite::{Connection, params};
use shuku_store::{Error, Store, open_connection};

#[test]
fn rejects_newer_schema_versions_without_changing_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    for version in [13, 15] {
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        assert!(
            matches!(open_connection(&path), Err(Error::UnsupportedVersion { found, .. }) if found == version)
        );
        let unchanged: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(unchanged, version);
    }
}

#[test]
fn failed_migration_rolls_back_schema_and_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE tag (sentinel TEXT); INSERT INTO tag VALUES ('保留');")
        .unwrap();
    assert!(open_connection(&path).is_err());
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);
    let partial: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'archive'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(partial, 0);
    let sentinel: String = connection
        .query_row("SELECT sentinel FROM tag", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sentinel, "保留");
}

#[test]
fn opens_configured_database_and_preserves_data_when_reopened() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let connection = open_connection(&path).unwrap();
    let pragma = |name: &str| {
        connection
            .pragma_query_value(None, name, |row| row.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(pragma("user_version"), 12);
    assert_eq!(pragma("synchronous"), 1);
    assert_eq!(pragma("foreign_keys"), 1);
    assert_eq!(pragma("mmap_size"), 0x7fff_0000);
    assert_eq!(pragma("cache_size"), -65536);
    assert_eq!(
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );

    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name != 'sqlite_sequence' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        tables,
        [
            "archive",
            "archive_tag",
            "category",
            "category_archive",
            "device_token",
            "lrr_pending",
            "metadata_candidate",
            "metadata_queue",
            "page_list",
            "progress",
            "series",
            "setting",
            "tag",
            "tankoubon",
            "tankoubon_archive",
            "tankoubon_tag"
        ]
    );
    let without_rowid: i64 = connection
        .query_row(
            "SELECT wr FROM pragma_table_list WHERE name = 'archive_tag'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(without_rowid, 1);
    let reverse_index: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_index_list('archive_tag') AS indexes
             WHERE (SELECT group_concat(name, ',') FROM pragma_index_info(indexes.name)) = 'archive_id,tag_id'",
            [], |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reverse_index, 1);

    connection
        .execute_batch(
            "INSERT INTO category (name, search, pinned) VALUES ('收藏', '', 1);
         INSERT INTO tankoubon (name, summary) VALUES ('系列', '简介');
         INSERT INTO tag (namespace, value, full) VALUES ('Artist', 'Alpha', 'artist:alpha');
         INSERT INTO tankoubon_tag (tankoubon_id, tag_id) VALUES (1, 1);",
        )
        .unwrap();
    assert!(
        connection
            .execute("INSERT INTO archive_tag VALUES (1, 999)", [])
            .is_err()
    );
    drop(connection);

    let reopened: Connection = open_connection(&path).unwrap();
    let name: String = reopened
        .query_row("SELECT name FROM category", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "收藏");
    assert!(reopened.execute("INSERT INTO tag (namespace, value, full) VALUES ('artist', 'alpha', 'artist:alpha')", []).is_err());
}

#[test]
fn migrates_page_rows_into_one_list_per_archive() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(concat!(
            include_str!("../src/migrations/001_initial.sql"),
            include_str!("../src/migrations/002_lrr_ids.sql"),
            "PRAGMA user_version = 2;"
        ))
        .unwrap();
    let mut ids = Vec::new();
    for (path, page_count) in [("with-pages.cbz", 2), ("empty.cbz", 0)] {
        let archive = metadata(path);
        connection
            .execute(
                "INSERT INTO archive (path, title, summary, file_size, mtime, quick_hash,
                    page_count, date_added, is_new) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7, 1)",
                params![
                    archive.path,
                    archive.title,
                    archive.file_size,
                    archive.mtime,
                    archive.quick_hash,
                    page_count,
                    archive.date_added
                ],
            )
            .unwrap();
        ids.push(connection.last_insert_rowid());
    }
    // 故意倒序插入，迁移要按 page_no 排序。
    for (page_no, page) in pages().iter().enumerate().rev() {
        connection
            .execute(
                "INSERT INTO page VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    ids[0],
                    page_no as i64,
                    page.entry_name,
                    page.data_offset,
                    page.compressed_size,
                    page.size,
                    page.method,
                    page.width,
                    page.height
                ],
            )
            .unwrap();
    }
    drop(connection);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.pages(ids[0]).unwrap(), pages());
    assert!(store.pages(ids[1]).unwrap().is_empty());
    let connection = open_connection(&path).unwrap();
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM page_list", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2);
    let old_table: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'page'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_table, 0);
}

#[test]
fn migration_keeps_categories_and_members_and_stops_reusing_category_ids() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(concat!(
            include_str!("../src/migrations/001_initial.sql"),
            include_str!("../src/migrations/002_lrr_ids.sql"),
            include_str!("../src/migrations/003_page_list.sql"),
            include_str!("../src/migrations/004_lrr_pending.sql"),
            "PRAGMA user_version = 4;
             INSERT INTO archive (path, title, summary, page_count, file_size, mtime, quick_hash,
                 content_hash, is_new, date_added, source_url)
                 VALUES ('a.cbz', 'A', '', 1, 1, 0, 'h', NULL, 0, 0, NULL);
             INSERT INTO category (id, name, search, pinned, lrr_id)
                 VALUES (1, '收藏', '', 1, 'SET_1'), (2, '动态', 'artist:a', 0, NULL);
             INSERT INTO category_archive VALUES (1, 1);"
        ))
        .unwrap();
    drop(connection);

    let mut store = Store::open(&path).unwrap();
    let categories = store.categories().unwrap();
    assert_eq!(
        categories
            .iter()
            .map(|c| (c.id, c.name.as_str(), c.archives.clone()))
            .collect::<Vec<_>>(),
        [(1, "收藏", vec![1]), (2, "动态", vec![])]
    );
    store.write(|writer| writer.delete_category(2)).unwrap();
    let created = store
        .write(|writer| writer.create_category("新的", "", false))
        .unwrap();
    assert_eq!(created, 3);
    drop(store);
    // 删除归档仍然级联删除成员关系，外键在重建后的表上照样生效。
    let connection = open_connection(&path).unwrap();
    connection
        .execute("DELETE FROM archive WHERE id = 1", [])
        .unwrap();
    let members: i64 = connection
        .query_row("SELECT count(*) FROM category_archive", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(members, 0);
}

#[test]
fn device_tokens_are_found_by_hash_touched_and_revoked() {
    let mut store = Store::open(":memory:").unwrap();
    let (first, second) = store
        .write(|writer| {
            Ok::<_, Error>((
                writer.create_device_token("手机", "aa", 1)?,
                writer.create_device_token("agent", "bb", 2)?,
            ))
        })
        .unwrap();
    // 同一个摘要不能有两个令牌。
    assert!(
        store
            .write(|writer| writer.create_device_token("重复", "aa", 3))
            .is_err()
    );
    store
        .write(|writer| writer.touch_device_token(first, 10))
        .unwrap();
    let found = store.device_token_by_hash("aa").unwrap().unwrap();
    assert_eq!(
        (found.id, found.name.as_str(), found.last_used_at),
        (first, "手机", Some(10))
    );
    assert!(
        store
            .write(|writer| writer.delete_device_token(first))
            .unwrap()
    );
    assert!(
        !store
            .write(|writer| writer.delete_device_token(first))
            .unwrap()
    );
    assert_eq!(store.device_token_by_hash("aa").unwrap(), None);
    let ids: Vec<i64> = store
        .device_tokens()
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, [second]);
}

#[test]
fn migration_marks_progress_at_the_last_page_as_finished() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    // 先建到迁移 011 为止的库，再放三本书的进度：停在最后一页、停在中间、页数为 0。
    drop(open_connection(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE progress DROP COLUMN finished;
             PRAGMA user_version = 11;
             INSERT INTO archive (id, path, title, summary, page_count, file_size, mtime,
                 quick_hash, content_hash, is_new, date_added, source_url)
                 VALUES (1, 'a.cbz', 'A', '', 3, 1, 0, 'h', NULL, 0, 0, NULL),
                        (2, 'b.cbz', 'B', '', 3, 1, 0, 'h', NULL, 0, 0, NULL);
             INSERT INTO progress (archive_id, page, client_time, updated_at)
                 VALUES (1, 2, 1, 1), (2, 1, 1, 1);",
        )
        .unwrap();
    drop(connection);

    let mut store = Store::open(&path).unwrap();
    let finished = |store: &Store, id: i64| {
        store
            .archive(id)
            .unwrap()
            .unwrap()
            .progress
            .unwrap()
            .finished
    };
    assert!(finished(&store, 1));
    assert!(!finished(&store, 2));
    // 读完的书翻回前面，仍然算读完；读到最后一页时补记。
    assert!(store.write_progress(1, 0, 2).unwrap());
    assert!(finished(&store, 1));
    assert!(store.write_progress(2, 2, 2).unwrap());
    assert!(finished(&store, 2));
    // 被拒绝的旧上报不改任何东西。
    assert!(!store.write_progress(2, 0, 1).unwrap());
    assert!(finished(&store, 2));
}
