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
fn static_category_members_are_added_and_removed_one_archive_at_a_time() {
    let (mut store, one, two) = store_with_two_archives();
    let favorites = store
        .write(|writer| writer.create_category("收藏", "", false))
        .unwrap();
    let changes = store
        .write(|writer| {
            Ok::<_, Error>([
                writer.set_category_member(favorites, two, true)?,
                writer.set_category_member(favorites, one, true)?,
                writer.set_category_member(favorites, one, true)?,
            ])
        })
        .unwrap();
    assert_eq!(changes, [true, true, false]);
    assert_eq!(store.archive_categories(one).unwrap(), [favorites]);
    let categories = store.categories().unwrap();
    assert_eq!(categories[0].archives, [one, two]);

    let removed = store
        .write(|writer| {
            Ok::<_, Error>([
                writer.set_category_member(favorites, one, false)?,
                writer.set_category_member(favorites, one, false)?,
            ])
        })
        .unwrap();
    assert_eq!(removed, [true, false]);
    assert!(store.archive_categories(one).unwrap().is_empty());
}

#[test]
fn membership_changes_reject_missing_targets_and_dynamic_categories() {
    let (mut store, one, _) = store_with_two_archives();
    let (favorites, dynamic) = store
        .write(|writer| {
            Ok::<_, Error>((
                writer.create_category("收藏", "", false)?,
                writer.create_category("作者", "artist:某某", true)?,
            ))
        })
        .unwrap();
    let result = store.write(|writer| writer.set_category_member(dynamic, one, true));
    assert!(matches!(result, Err(Error::DynamicCategory(id)) if id == dynamic));
    let result = store.write(|writer| writer.set_category_member(favorites, one + 100, true));
    assert!(matches!(result, Err(Error::ArchiveNotFound(_))));
    let result = store.write(|writer| writer.set_category_member(dynamic + 100, one, true));
    assert!(matches!(result, Err(Error::CategoryNotFound(_))));
}

#[test]
fn categories_are_renamed_pinned_and_deleted_without_touching_archives() {
    let (mut store, one, _) = store_with_two_archives();
    let (favorites, dynamic) = store
        .write(|writer| {
            Ok::<_, Error>((
                writer.create_category("收藏", "", false)?,
                writer.create_category("作者", "artist:某某", false)?,
            ))
        })
        .unwrap();
    store
        .write(|writer| {
            writer.set_category_member(favorites, one, true)?;
            writer.update_category(favorites, Some("最爱"), None, Some(true))?;
            writer.update_category(dynamic, None, Some("artist:别人"), None)
        })
        .unwrap();
    let categories = store.categories().unwrap();
    assert_eq!(
        categories
            .iter()
            .map(|c| (c.id, c.name.as_str(), c.search.as_str(), c.pinned))
            .collect::<Vec<_>>(),
        [
            (favorites, "最爱", "", true),
            (dynamic, "作者", "artist:别人", false)
        ]
    );

    for (id, search) in [(favorites, "artist:某某"), (dynamic, "")] {
        let result = store.write(|writer| writer.update_category(id, None, Some(search), None));
        assert!(matches!(result, Err(Error::CategoryKindChange(_))));
    }
    let result = store.write(|writer| writer.update_category(dynamic + 100, Some("x"), None, None));
    assert!(matches!(result, Err(Error::CategoryNotFound(_))));

    store
        .write(|writer| writer.delete_category(favorites))
        .unwrap();
    assert!(store.archive_categories(one).unwrap().is_empty());
    assert!(store.archive(one).unwrap().is_some());
    let result = store.write(|writer| writer.delete_category(favorites));
    assert!(matches!(result, Err(Error::CategoryNotFound(_))));
}
