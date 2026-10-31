mod common;

use common::{metadata, pages};
use shuku_store::{Error, LrrTankoubon, Store, Tag};

fn tag(namespace: &str, value: &str) -> Tag {
    Tag {
        namespace: namespace.into(),
        value: value.into(),
    }
}

fn tag_names(store: &Store) -> Vec<String> {
    store
        .library()
        .unwrap()
        .tags
        .into_iter()
        .map(|entry| entry.full)
        .collect()
}

#[test]
fn title_and_summary_change_only_when_given() {
    let mut store = Store::open(":memory:").unwrap();
    let id = store.save_archive(None, &metadata("a.cbz"), &[]).unwrap();
    store
        .write(|writer| writer.edit_archive(id, Some("新标题"), None))
        .unwrap();
    let archive = store.archive(id).unwrap().unwrap();
    assert_eq!(archive.metadata.title, "新标题");
    assert_eq!(archive.metadata.summary, "测试摘要");
    store
        .write(|writer| writer.edit_archive(id, None, Some("")))
        .unwrap();
    let archive = store.archive(id).unwrap().unwrap();
    assert_eq!(archive.metadata.title, "新标题");
    assert_eq!(archive.metadata.summary, "");
    assert!(matches!(
        store.write(|writer| writer.edit_archive(id + 1, Some("x"), None)),
        Err(Error::ArchiveNotFound(_))
    ));
}

#[test]
fn adding_and_removing_tags_keeps_the_rest_and_drops_unused_tags() {
    let mut store = Store::open(":memory:").unwrap();
    let one = store.save_archive(None, &metadata("one.cbz"), &[]).unwrap();
    let two = store.save_archive(None, &metadata("two.cbz"), &[]).unwrap();
    store
        .replace_tags(one, &[tag("artist", "a"), tag("", "typo")])
        .unwrap();
    store.replace_tags(two, &[tag("artist", "a")]).unwrap();

    store
        .write(|writer| {
            writer.add_tags(one, &[tag("artist", "A"), tag("language", "中文")])?;
            writer.remove_tags(one, &[tag("", "TYPO"), tag("", "absent")])
        })
        .unwrap();
    let tags = store.archive(one).unwrap().unwrap().tags;
    assert_eq!(tags, [tag("artist", "a"), tag("language", "中文")]);
    // 去掉的标签没有别的归档用，从标签表里删掉，不再出现在补全里。
    assert_eq!(tag_names(&store), ["artist:a", "language:中文"]);

    // 另一本还在用的标签保留。
    store
        .write(|writer| writer.remove_tags(one, &[tag("artist", "a")]))
        .unwrap();
    assert_eq!(tag_names(&store), ["artist:a", "language:中文"]);

    store.replace_tags(two, &[]).unwrap();
    assert_eq!(tag_names(&store), ["language:中文"]);

    assert!(matches!(
        store.write(|writer| writer.add_tags(two + 1, &[tag("", "x")])),
        Err(Error::ArchiveNotFound(_))
    ));
    assert!(matches!(
        store.replace_tags(two + 1, &[tag("", "x")]),
        Err(Error::ArchiveNotFound(_))
    ));
}

#[test]
fn renaming_a_tag_to_an_existing_one_merges_them() {
    let mut store = Store::open(":memory:").unwrap();
    let one = store.save_archive(None, &metadata("one.cbz"), &[]).unwrap();
    let two = store.save_archive(None, &metadata("two.cbz"), &[]).unwrap();
    store
        .replace_tags(one, &[tag("artist", "alpha"), tag("artist", "alfa")])
        .unwrap();
    store.replace_tags(two, &[tag("artist", "alfa")]).unwrap();
    store
        .import_lrr_tankoubon(&LrrTankoubon {
            lrr_id: "TANK_1".into(),
            name: "合集".into(),
            summary: String::new(),
            tags: vec![tag("artist", "alfa")],
            archives: vec![],
        })
        .unwrap();

    let uses = store
        .write(|writer| writer.rename_tag(&tag("artist", "ALFA"), &tag("artist", "alpha")))
        .unwrap();
    assert_eq!(uses, 2);
    assert_eq!(
        store.archive(one).unwrap().unwrap().tags,
        [tag("artist", "alpha")]
    );
    assert_eq!(
        store.archive(two).unwrap().unwrap().tags,
        [tag("artist", "alpha")]
    );
    assert_eq!(tag_names(&store), ["artist:alpha"]);

    assert!(matches!(
        store.write(|writer| writer.rename_tag(&tag("artist", "alfa"), &tag("artist", "x"))),
        Err(Error::TagNotFound(name)) if name == "artist:alfa"
    ));
}

#[test]
fn renaming_a_tag_changes_its_name_or_only_its_case() {
    let mut store = Store::open(":memory:").unwrap();
    let id = store.save_archive(None, &metadata("a.cbz"), &[]).unwrap();
    store.replace_tags(id, &[tag("parody", "foo")]).unwrap();
    store
        .write(|writer| writer.rename_tag(&tag("parody", "foo"), &tag("series", "Bar")))
        .unwrap();
    assert_eq!(
        store.archive(id).unwrap().unwrap().tags,
        [tag("series", "Bar")]
    );
    store
        .write(|writer| writer.rename_tag(&tag("series", "bar"), &tag("series", "BAR")))
        .unwrap();
    assert_eq!(
        store.archive(id).unwrap().unwrap().tags,
        [tag("series", "BAR")]
    );
    assert_eq!(tag_names(&store), ["series:bar"]);
}

#[test]
fn deleting_an_archive_removes_everything_that_refers_to_it() {
    let mut store = Store::open(":memory:").unwrap();
    let gone = store
        .save_archive(None, &metadata("gone.cbz"), &pages())
        .unwrap();
    let kept = store
        .save_archive(None, &metadata("kept.cbz"), &pages())
        .unwrap();
    store
        .replace_tags(gone, &[tag("artist", "shared"), tag("", "only")])
        .unwrap();
    store
        .replace_tags(kept, &[tag("artist", "shared")])
        .unwrap();
    store.write_progress(gone, 1, 1000).unwrap();
    let category = store
        .write(|writer| {
            let category = writer.create_category("收藏", "", false)?;
            writer.set_category_member(category, gone, true)?;
            writer.set_category_member(category, kept, true)?;
            Ok::<_, Error>(category)
        })
        .unwrap();

    store.write(|writer| writer.delete_archive(gone)).unwrap();
    assert!(store.archive(gone).unwrap().is_none());
    assert!(store.archive_with_pages(gone).unwrap().is_none());
    assert_eq!(store.find_by_path("gone.cbz").unwrap(), None);
    assert!(!store.progress().unwrap().contains_key(&gone));
    assert_eq!(store.categories().unwrap()[0].id, category);
    assert_eq!(store.categories().unwrap()[0].archives, [kept]);
    assert_eq!(tag_names(&store), ["artist:shared"]);

    assert!(matches!(
        store.write(|writer| writer.delete_archive(gone)),
        Err(Error::ArchiveNotFound(_))
    ));
    // 删掉的编号不会再分给新归档。
    let next = store
        .save_archive(None, &metadata("gone.cbz"), &pages())
        .unwrap();
    assert!(next > kept);
}
