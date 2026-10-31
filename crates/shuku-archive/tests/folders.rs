mod common;

use common::bmp;
use shuku_archive::{
    CompressionMethod, Error, folder_content_hash, folder_quick_hash, identity,
    list_folder_pages_with_broken, read_page,
};
use std::fs;

#[test]
fn folder_pages_are_its_images_in_natural_order() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("Vol.1");
    fs::create_dir(&book).unwrap();
    fs::write(book.join("10.bmp"), bmp(4, 6)).unwrap();
    fs::write(book.join("2.BMP"), bmp(5, 7)).unwrap();
    fs::write(book.join("empty.jpg"), b"").unwrap();
    // 不是页面：隐藏文件、macOS 的附带文件、别的文件和子目录。
    fs::write(book.join(".1.bmp"), bmp(1, 1)).unwrap();
    fs::write(book.join("._3.bmp"), bmp(1, 1)).unwrap();
    fs::write(book.join("info.txt"), b"text").unwrap();
    fs::create_dir(book.join("extra")).unwrap();
    fs::write(book.join("extra/1.bmp"), bmp(1, 1)).unwrap();

    let (pages, broken) = list_folder_pages_with_broken(&book).unwrap();
    let summary: Vec<_> = pages
        .iter()
        .map(|page| (page.entry_name.as_str(), page.width, page.height))
        .collect();
    assert_eq!(
        summary,
        [("2.BMP", 5, 7), ("10.bmp", 4, 6), ("empty.jpg", 0, 0)]
    );
    assert_eq!(broken.len(), 1);
    assert_eq!(broken[0].entry_name, "empty.jpg");
    for page in &pages {
        assert_eq!(page.method, CompressionMethod::Stored);
        assert_eq!(page.data_offset, 0);
        assert_eq!(
            read_page(&book, page).unwrap(),
            fs::read(book.join(&page.entry_name)).unwrap()
        );
    }

    let identity = identity(&book).unwrap();
    assert_eq!(identity.size, (bmp(4, 6).len() + bmp(5, 7).len()) as u64);
    let times: Vec<_> = ["10.bmp", "2.BMP", "empty.jpg", ""]
        .iter()
        .map(|name| fs::metadata(book.join(name)).unwrap().modified().unwrap())
        .collect();
    assert_eq!(identity.newest, *times.iter().max().unwrap());
}

#[test]
fn quick_fingerprint_follows_names_and_sizes_and_survives_moving_the_folder() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(book.join("1.bmp"), bmp(4, 6)).unwrap();
    fs::write(book.join("2.bmp"), bmp(5, 7)).unwrap();
    let quick = folder_quick_hash(&book).unwrap();
    let full = folder_content_hash(&book).unwrap();

    let moved = directory.path().join("moved");
    fs::rename(&book, &moved).unwrap();
    assert_eq!(folder_quick_hash(&moved).unwrap(), quick);
    assert_eq!(folder_content_hash(&moved).unwrap(), full);

    // 同名等长、内容不同：两种指纹都变，图片 URL 的版本号跟着变。
    let mut changed = bmp(5, 7);
    *changed.last_mut().unwrap() = 0xff;
    fs::write(moved.join("2.bmp"), changed).unwrap();
    assert_ne!(folder_quick_hash(&moved).unwrap(), quick);
    assert_ne!(folder_content_hash(&moved).unwrap(), full);
    let quick = folder_quick_hash(&moved).unwrap();
    fs::write(moved.join("3.bmp"), bmp(1, 1)).unwrap();
    assert_ne!(folder_quick_hash(&moved).unwrap(), quick);
}

#[test]
fn folder_pages_only_read_plain_file_names_of_the_recorded_size() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(directory.path().join("secret.bmp"), bmp(1, 1)).unwrap();
    fs::write(book.join("1.bmp"), bmp(4, 6)).unwrap();
    let (mut pages, _) = list_folder_pages_with_broken(&book).unwrap();
    let mut page = pages.remove(0);
    page.size += 1;
    assert!(matches!(
        read_page(&book, &page),
        Err(Error::CorruptEntry { .. })
    ));
    for name in ["../secret.bmp", "..", ".", "", "./1.bmp", "1.bmp/"] {
        page.entry_name = name.into();
        assert!(
            matches!(read_page(&book, &page), Err(Error::CorruptEntry { .. })),
            "{name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_page_replaced_by_a_symlink_is_not_followed() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(directory.path().join("secret.bmp"), bmp(4, 6)).unwrap();
    fs::write(book.join("1.bmp"), bmp(4, 6)).unwrap();
    let (pages, _) = list_folder_pages_with_broken(&book).unwrap();
    fs::remove_file(book.join("1.bmp")).unwrap();
    std::os::unix::fs::symlink(directory.path().join("secret.bmp"), book.join("1.bmp")).unwrap();
    assert!(read_page(&book, &pages[0]).is_err());
    // 扫描也不把符号链接当页面。
    let (pages, _) = list_folder_pages_with_broken(&book).unwrap();
    assert!(pages.is_empty());
}

#[test]
fn identity_changes_on_rename_even_next_to_a_future_timestamp() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(book.join("1.bmp"), bmp(4, 6)).unwrap();
    fs::write(book.join("2.bmp"), bmp(4, 6)).unwrap();
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(86_400 * 365);
    fs::File::options()
        .write(true)
        .open(book.join("2.bmp"))
        .unwrap()
        .set_modified(future)
        .unwrap();
    let before = identity(&book).unwrap();
    // 改名不改变总大小，最新的修改时间也还是那张未来的图，但文件夹本身的修改时间变了。
    fs::rename(book.join("1.bmp"), book.join("3.bmp")).unwrap();
    fs::File::open(&book)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    let after = identity(&book).unwrap();
    assert_eq!((after.size, after.newest), (before.size, before.newest));
    assert_ne!(after.modified, before.modified);
}

#[test]
fn names_equal_in_natural_order_are_ordered_by_their_exact_spelling() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    for name in ["a.bmp", "1.bmp", "A.bmp", "01.bmp"] {
        fs::write(book.join(name), bmp(1, 1)).unwrap();
    }
    let (pages, _) = list_folder_pages_with_broken(&book).unwrap();
    let names: Vec<_> = pages.iter().map(|page| page.entry_name.as_str()).collect();
    assert_eq!(names, ["01.bmp", "1.bmp", "A.bmp", "a.bmp"]);
}

#[test]
fn identity_changes_when_pages_are_renamed_or_swapped_within_the_same_second() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(book.join("1.bmp"), bmp(4, 6)).unwrap();
    fs::write(book.join("2.bmp"), bmp(6, 4)).unwrap();
    // 把文件夹和图片的修改时间都钉在同一个时刻，只剩文件名能区分。
    let pin = |book: &std::path::Path| {
        let moment = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for name in ["1.bmp", "2.bmp", ""] {
            fs::File::open(book.join(name))
                .unwrap()
                .set_modified(moment)
                .unwrap();
        }
    };
    pin(&book);
    let before = identity(&book).unwrap();
    fs::rename(book.join("1.bmp"), book.join("tmp.bmp")).unwrap();
    fs::rename(book.join("2.bmp"), book.join("1.bmp")).unwrap();
    fs::rename(book.join("tmp.bmp"), book.join("2.bmp")).unwrap();
    pin(&book);
    let after = identity(&book).unwrap();
    assert_eq!(after.size, before.size);
    assert_ne!(after.modified, before.modified);
}

#[test]
fn zips_named_like_images_are_not_folder_pages() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(book.join("1.bmp"), bmp(4, 6)).unwrap();
    common::write_zip(
        &book.join("cover.jpg"),
        &[("1.bmp", zip::CompressionMethod::Stored, &bmp(1, 1))],
    );
    let (pages, broken) = list_folder_pages_with_broken(&book).unwrap();
    assert_eq!(pages.len(), 1);
    assert!(broken.is_empty());
    assert_eq!(identity(&book).unwrap().size, bmp(4, 6).len() as u64);
}

#[cfg(unix)]
#[test]
fn backslashes_are_ordinary_characters_in_unix_page_names() {
    let directory = tempfile::tempdir().unwrap();
    let book = directory.path().join("book");
    fs::create_dir(&book).unwrap();
    fs::write(book.join("a\\b.bmp"), bmp(4, 6)).unwrap();
    let (pages, _) = list_folder_pages_with_broken(&book).unwrap();
    assert_eq!(pages[0].entry_name, "a\\b.bmp");
    assert_eq!(read_page(&book, &pages[0]).unwrap(), bmp(4, 6));
}
