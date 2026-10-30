use rusqlite::Connection;
use shuku_store::{Error, open_connection};

#[test]
fn rejects_newer_schema_versions_without_changing_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    for version in [3, 7] {
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
    assert_eq!(pragma("user_version"), 2);
    assert_eq!(pragma("synchronous"), 1);
    assert_eq!(pragma("foreign_keys"), 1);
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
            "page",
            "progress",
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
