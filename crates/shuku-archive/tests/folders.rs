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

    let (size, modified) = identity(&book).unwrap();
    assert_eq!(size, (bmp(4, 6).len() + bmp(5, 7).len()) as u64);
    let newest = ["10.bmp", "2.BMP", "empty.jpg"]
        .iter()
        .map(|name| fs::metadata(book.join(name)).unwrap().modified().unwrap())
        .max()
        .unwrap();
    assert_eq!(modified, newest);
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

    // 同样大小、内容不同：快速指纹不变，完整指纹变。
    let mut changed = bmp(5, 7);
    *changed.last_mut().unwrap() = 0xff;
    fs::write(moved.join("2.bmp"), changed).unwrap();
    assert_eq!(folder_quick_hash(&moved).unwrap(), quick);
    assert_ne!(folder_content_hash(&moved).unwrap(), full);
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
    for name in ["../secret.bmp", "..", ""] {
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
