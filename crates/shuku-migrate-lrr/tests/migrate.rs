mod common;

use common::{MemorySource, PREFIX, add_archive, double_encoded};
use rusqlite::Connection;
use shuku_migrate_lrr::{Error, migrate};
use shuku_store::{Store, Tag, open_connection};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const D: &str = "dddddddddddddddddddddddddddddddddddddddd";

fn tag(namespace: &str, value: &str) -> Tag {
    Tag {
        namespace: namespace.into(),
        value: value.into(),
    }
}

fn sorted(mut tags: Vec<Tag>) -> Vec<(String, String)> {
    tags.sort_by(|a, b| (&a.namespace, &a.value).cmp(&(&b.namespace, &b.value)));
    tags.into_iter()
        .map(|tag| (tag.namespace, tag.value))
        .collect()
}

fn lrr() -> MemorySource {
    let mut source = MemorySource::default();
    let file = |path: &str| format!("{PREFIX}/{path}").into_bytes();
    let title = double_encoded("漫画");
    let a_file = file("a/one.cbz");
    source.hash(
        A,
        &[
            ("file", &a_file),
            ("title", &title),
            ("summary", "简介".as_bytes()),
            (
                "tags",
                "artist:Alpha, date_added:1600000000, source:https://example.com/g/1, source:https://mirror, 独立 , ,language:japanese".as_bytes(),
            ),
            ("isnew", b"false"),
            ("progress", b"5"),
            ("lastreadtime", b"1700000000"),
        ],
    );
    let b_file = file("two.zip");
    source.hash(
        B,
        &[
            ("file", &b_file),
            ("title", b"Two"),
            ("tags", b""),
            ("isnew", b"true"),
            ("progress", b"0"),
        ],
    );
    source.hash(C, &[("file", b"/elsewhere/missing.zip"), ("title", b"C")]);
    let d_file = file("gone.zip");
    source.hash(D, &[("file", &d_file), ("title", b"D")]);
    // 键长 40 但不是十六进制 ID，不是归档。
    source.hash(&"x".repeat(40), &[("file", &file("two.zip"))]);
    let archives = format!("[\"{A}\", \"{D}\"]");
    source.hash(
        "SET_1600000000",
        &[
            ("name", "收藏".as_bytes()),
            ("search", b""),
            ("pinned", b"1"),
            ("archives", archives.as_bytes()),
        ],
    );
    source.hash(
        "SET_1600000001",
        &[
            ("name", "动态".as_bytes()),
            ("search", b"artist:alpha"),
            ("pinned", b"0"),
            ("archives", b"[]"),
        ],
    );
    source.sorted_sets.insert(
        "TANK_1600000002".into(),
        vec![
            ("name_系列".into(), 0.0),
            ("summary_合集简介".into(), -1.0),
            ("tags_artist:Alpha,series:X".into(), -2.0),
            ("progress_3".into(), -3.0),
            (D.into(), 3.0),
            (B.into(), 1.0),
            (A.into(), 2.0),
        ],
    );
    source
}

struct Library {
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
    store: Store,
    one: i64,
    two: i64,
    three: i64,
}

fn library() -> Library {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shuku.sqlite");
    let mut store = Store::open(&path).unwrap();
    let one = add_archive(&mut store, "a/one.cbz", 10);
    let two = add_archive(&mut store, "two.zip", 3);
    let three = add_archive(&mut store, "three.zip", 3);
    Library {
        _directory: directory,
        path,
        store,
        one,
        two,
        three,
    }
}

fn rows(connection: &Connection, sql: &str) -> Vec<Vec<String>> {
    let mut statement = connection.prepare(sql).unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| {
                    row.get::<_, rusqlite::types::Value>(index)
                        .map(|value| format!("{value:?}"))
                })
                .collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn imports_archive_metadata_tags_and_progress_matched_by_path() {
    let mut library = library();
    let report = migrate(&mut lrr(), &mut library.store, &format!("{PREFIX}/")).unwrap();

    assert_eq!(report.archives, 2);
    // gone.zip 在内容目录下但还没入库，暂存；C 的路径不在内容目录下。
    assert_eq!(report.pending, 1);
    assert_eq!(report.unmatched, ["/elsewhere/missing.zip"]);
    // B 和 D 没有 date_added 标签。
    assert_eq!(report.undated, 2);

    let one = library.store.archive(library.one).unwrap().unwrap();
    assert_eq!(one.metadata.title, "漫画");
    assert_eq!(one.metadata.summary, "简介");
    assert_eq!(one.metadata.date_added, 1_600_000_000);
    assert_eq!(
        one.metadata.source_url.as_deref(),
        Some("https://example.com/g/1")
    );
    assert!(!one.metadata.is_new);
    assert_eq!(one.metadata.lrr_id.as_deref(), Some(A));
    assert_eq!(one.metadata.path, "a/one.cbz");
    assert_eq!(one.page_count, 10);
    // 第二个 source 标签不覆盖第一个，作为普通标签保留。
    assert_eq!(
        sorted(one.tags),
        sorted(vec![
            tag("", "独立"),
            tag("artist", "Alpha"),
            tag("language", "japanese"),
            tag("source", "https://mirror"),
        ])
    );
    // LRR 的进度从 1 开始计，阅读时间是 Unix 秒。
    let progress = one.progress.unwrap();
    assert_eq!(
        (progress.page, progress.client_time),
        (4, 1_700_000_000_000)
    );

    let two = library.store.archive(library.two).unwrap().unwrap();
    assert_eq!(two.metadata.title, "Two");
    assert!(two.tags.is_empty());
    // 没有 date_added 标签时用文件的修改时间，不用扫描时记录的加入时间。
    assert_eq!(two.metadata.date_added, 1_700_000_001);
    assert!(two.metadata.is_new);
    assert!(two.progress.is_none());

    let three = library.store.archive(library.three).unwrap().unwrap();
    assert_eq!(three.metadata.title, "扫描时的标题");
    assert!(three.metadata.lrr_id.is_none());
}

#[test]
fn imports_categories_and_tankoubon_with_ordered_members() {
    let mut library = library();
    let report = migrate(&mut lrr(), &mut library.store, PREFIX).unwrap();
    assert_eq!((report.categories, report.tankoubons), (2, 1));
    // D 还没扫描入库，暂存着，不算缺失。
    assert_eq!(report.missing_members, 0);

    let connection = open_connection(&library.path).unwrap();
    assert_eq!(
        rows(
            &connection,
            "SELECT lrr_id, name, search, pinned FROM category ORDER BY lrr_id"
        ),
        [
            [
                "Text(\"SET_1600000000\")",
                "Text(\"收藏\")",
                "Text(\"\")",
                "Integer(1)"
            ],
            [
                "Text(\"SET_1600000001\")",
                "Text(\"动态\")",
                "Text(\"artist:alpha\")",
                "Integer(0)"
            ],
        ]
    );
    assert_eq!(
        rows(
            &connection,
            "SELECT c.lrr_id, ca.archive_id FROM category_archive ca JOIN category c ON c.id = ca.category_id"
        ),
        [[
            "Text(\"SET_1600000000\")".to_string(),
            format!("Integer({})", library.one)
        ]]
    );
    assert_eq!(
        rows(&connection, "SELECT lrr_id, name, summary FROM tankoubon"),
        [[
            "Text(\"TANK_1600000002\")",
            "Text(\"系列\")",
            "Text(\"合集简介\")"
        ]]
    );
    assert_eq!(
        rows(
            &connection,
            "SELECT archive_id, position FROM tankoubon_archive ORDER BY position"
        ),
        [
            [format!("Integer({})", library.two), "Integer(1)".into()],
            [format!("Integer({})", library.one), "Integer(2)".into()],
        ]
    );
    assert_eq!(
        rows(
            &connection,
            "SELECT t.namespace, t.value FROM tankoubon_tag tt JOIN tag t ON t.id = tt.tag_id ORDER BY t.full"
        ),
        [
            ["Text(\"artist\")", "Text(\"Alpha\")"],
            ["Text(\"series\")", "Text(\"X\")"]
        ]
    );
}

#[test]
fn members_that_are_not_in_lrr_are_reported_missing() {
    let mut library = library();
    let mut source = lrr();
    let archives = format!("[\"{A}\", \"{}\"]", "e".repeat(40));
    source
        .hashes
        .get_mut("SET_1600000000")
        .unwrap()
        .insert("archives".into(), archives.into_bytes());
    let report = migrate(&mut source, &mut library.store, PREFIX).unwrap();
    assert_eq!(report.missing_members, 1);
}

#[test]
fn archives_scanned_after_the_import_get_the_staged_lrr_metadata() {
    let mut library = library();
    migrate(&mut lrr(), &mut library.store, PREFIX).unwrap();
    assert_eq!(library.store.lrr_pending_count().unwrap(), 1);

    // 像扫描那样在同一个事务里新增归档并套用暂存的元数据。
    let gone = add_archive(&mut library.store, "gone.zip", 3);
    let applied = library
        .store
        .write(|writer| writer.apply_lrr_pending(gone, "gone.zip"))
        .unwrap();
    assert!(applied);
    assert_eq!(library.store.lrr_pending_count().unwrap(), 0);
    let applied_again = library
        .store
        .write(|writer| writer.apply_lrr_pending(gone, "gone.zip"))
        .unwrap();
    assert!(!applied_again);

    let archive = library.store.archive(gone).unwrap().unwrap();
    assert_eq!(archive.metadata.title, "D");
    assert_eq!(archive.metadata.lrr_id.as_deref(), Some(D));
    // D 没有 date_added，用文件的修改时间。
    assert_eq!(archive.metadata.date_added, 1_700_000_001);
    let connection = open_connection(&library.path).unwrap();
    assert_eq!(
        rows(
            &connection,
            "SELECT c.lrr_id, ca.archive_id FROM category_archive ca JOIN category c ON c.id = ca.category_id ORDER BY ca.archive_id"
        ),
        [
            [
                "Text(\"SET_1600000000\")".to_string(),
                format!("Integer({})", library.one)
            ],
            [
                "Text(\"SET_1600000000\")".to_string(),
                format!("Integer({gone})")
            ],
        ]
    );
    // D 在 Tankoubon 里排第三，补进来后仍是第三。
    assert_eq!(
        rows(
            &connection,
            "SELECT archive_id, position FROM tankoubon_archive ORDER BY position"
        ),
        [
            [format!("Integer({})", library.two), "Integer(1)".into()],
            [format!("Integer({})", library.one), "Integer(2)".into()],
            [format!("Integer({gone})"), "Integer(3)".into()],
        ]
    );

    // 再导入一次时它已经入库，直接写入，不再暂存。
    let report = migrate(&mut lrr(), &mut library.store, PREFIX).unwrap();
    assert_eq!((report.archives, report.pending), (3, 0));
    assert_eq!(library.store.lrr_pending_count().unwrap(), 0);
}

#[test]
fn a_member_removed_in_lrr_is_not_added_back_by_an_old_staged_record() {
    let mut library = library();
    migrate(&mut lrr(), &mut library.store, PREFIX).unwrap();
    // LRR 里把 D 移出了分类，再导入一次；上次暂存的 D 还记着旧的分类。
    let mut changed = lrr();
    changed
        .hashes
        .get_mut("SET_1600000000")
        .unwrap()
        .insert("archives".into(), format!("[\"{A}\"]").into_bytes());
    migrate(&mut changed, &mut library.store, PREFIX).unwrap();
    let gone = add_archive(&mut library.store, "gone.zip", 3);
    library
        .store
        .write(|writer| writer.apply_lrr_pending(gone, "gone.zip"))
        .unwrap();

    let connection = open_connection(&library.path).unwrap();
    assert_eq!(
        rows(&connection, "SELECT archive_id FROM category_archive"),
        [[format!("Integer({})", library.one)]]
    );
}

#[test]
fn staged_progress_is_applied_with_the_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path().join("shuku.sqlite")).unwrap();
    // 什么都还没扫描时也能导入。
    let report = migrate(&mut lrr(), &mut store, PREFIX).unwrap();
    assert_eq!((report.archives, report.pending), (0, 3));
    let one = add_archive(&mut store, "a/one.cbz", 10);
    store
        .write(|writer| writer.apply_lrr_pending(one, "a/one.cbz"))
        .unwrap();
    let archive = store.archive(one).unwrap().unwrap();
    assert_eq!(archive.metadata.date_added, 1_600_000_000);
    // 首次导入时分类在暂存之前写入，扫描套用暂存时能把这本加进分类。
    let connection = open_connection(directory.path().join("shuku.sqlite")).unwrap();
    assert_eq!(
        rows(
            &connection,
            "SELECT c.lrr_id, ca.archive_id FROM category_archive ca JOIN category c ON c.id = ca.category_id"
        ),
        [[
            "Text(\"SET_1600000000\")".to_string(),
            format!("Integer({one})")
        ]]
    );
    let progress = archive.progress.unwrap();
    assert_eq!(
        (progress.page, progress.client_time),
        (4, 1_700_000_000_000)
    );
}

#[test]
fn running_again_overwrites_from_lrr_without_duplicating_rows() {
    let mut library = library();
    migrate(&mut lrr(), &mut library.store, PREFIX).unwrap();
    let connection = open_connection(&library.path).unwrap();
    let snapshot = |connection: &Connection| {
        [
            "SELECT * FROM archive_tag ORDER BY 1, 2",
            "SELECT id, lrr_id, name, search, pinned FROM category ORDER BY id",
            "SELECT * FROM category_archive ORDER BY 1, 2",
            "SELECT id, lrr_id, name, summary FROM tankoubon ORDER BY id",
            "SELECT * FROM tankoubon_archive ORDER BY 1, 2",
            "SELECT * FROM tankoubon_tag ORDER BY 1, 2",
            "SELECT archive_id, page, client_time FROM progress ORDER BY 1",
            "SELECT path FROM lrr_pending ORDER BY 1",
        ]
        .map(|sql| rows(connection, sql))
    };
    let first = snapshot(&connection);
    migrate(&mut lrr(), &mut library.store, PREFIX).unwrap();
    assert_eq!(snapshot(&connection), first);

    let mut changed = lrr();
    let fields = changed.hashes.get_mut(A).unwrap();
    fields.insert("title".into(), b"New title".to_vec());
    fields.insert("tags".into(), b"artist:Alpha".to_vec());
    changed
        .hashes
        .get_mut("SET_1600000000")
        .unwrap()
        .insert("name".into(), b"Renamed".to_vec());
    migrate(&mut changed, &mut library.store, PREFIX).unwrap();
    let one = library.store.archive(library.one).unwrap().unwrap();
    assert_eq!(one.metadata.title, "New title");
    assert_eq!(one.tags, [tag("artist", "Alpha")]);
    assert_eq!(
        rows(
            &connection,
            "SELECT name FROM category WHERE lrr_id = 'SET_1600000000'"
        ),
        [["Text(\"Renamed\")"]]
    );
    assert_eq!(
        rows(&connection, "SELECT count(*) FROM category"),
        [["Integer(2)"]]
    );
}

#[test]
fn malformed_lrr_values_stop_the_import_with_the_key_and_field() {
    let cases: [(&str, &str, &[u8]); 4] = [
        (A, "tags", b"date_added:yesterday"),
        (A, "progress", b"five"),
        (A, "lastreadtime", b"soon"),
        ("SET_1600000000", "archives", b"not json"),
    ];
    for (key, field, value) in cases {
        let mut library = library();
        let mut source = lrr();
        source
            .hashes
            .get_mut(key)
            .unwrap()
            .insert(field.into(), value.to_vec());
        let error = migrate(&mut source, &mut library.store, PREFIX).unwrap_err();
        assert!(
            matches!(&error, Error::InvalidField { key: k, field: f, .. } if k == key && *f == field),
            "{key} {field}: {error}"
        );
    }
}
