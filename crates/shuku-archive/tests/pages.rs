mod common;

use common::{bmp, patch_first_entry_headers, write_zip};
use shuku_archive::{CompressionMethod, Error, list_pages, read_page};
use zip::CompressionMethod::{Deflated, Stored};

#[test]
fn lists_images_in_case_insensitive_natural_order_and_ignores_junk() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.cbz");
    let image = bmp(7, 11);
    write_zip(
        &path,
        &[
            ("10.jpg", Stored, &image),
            ("2.jpg", Stored, &image),
            ("b1.PNG", Stored, &image),
            ("A2.jpeg", Stored, &image),
            ("a1.GIF", Stored, &image),
            ("c.webp", Stored, &image),
            ("d.avif", Stored, &image),
            ("e.jxl", Stored, &image),
            ("f.bmp", Stored, &image),
            ("__MACOSX/1.jpg", Stored, b"junk"),
            ("folder/__MACOSX/1.jpg", Stored, b"junk"),
            ("Thumbs.db", Stored, b"junk"),
            ("folder/.DS_Store", Stored, b"junk"),
            ("._1.jpg", Stored, b"junk"),
            ("folder/._2.jpg", Stored, b"junk"),
            ("notes.txt", Stored, b"junk"),
            ("cover.svg", Stored, b"junk"),
        ],
    );

    let pages = list_pages(&path).unwrap();
    let names: Vec<_> = pages.iter().map(|page| page.entry_name.as_str()).collect();
    assert_eq!(
        names,
        [
            "2.jpg", "10.jpg", "a1.GIF", "A2.jpeg", "b1.PNG", "c.webp", "d.avif", "e.jxl", "f.bmp"
        ]
    );
    for page in pages {
        assert_eq!((page.width, page.height), (7, 11));
        assert_eq!(page.size, image.len() as u64);
        assert_eq!(page.compressed_size, image.len() as u64);
        assert_eq!(page.method, CompressionMethod::Stored);
    }
}

#[test]
fn reads_stored_and_deflated_pages_using_recorded_offsets_without_the_directory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    let first = bmp(17, 29);
    let second = bmp(31, 43);
    write_zip(
        &path,
        &[("2.jpg", Stored, &first), ("10.jpg", Deflated, &second)],
    );
    let original = std::fs::read(&path).unwrap();
    let pages = list_pages(&path).unwrap();

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].method, CompressionMethod::Stored);
    assert_eq!(pages[1].method, CompressionMethod::Deflate);
    assert_eq!((pages[0].width, pages[0].height), (17, 29));
    assert_eq!((pages[1].width, pages[1].height), (31, 43));
    assert_eq!(pages[1].size, second.len() as u64);
    assert!(pages[1].compressed_size < pages[1].size);
    assert_eq!(read_page(&path, &pages[0]).unwrap(), first);
    assert_eq!(read_page(&path, &pages[1]).unwrap(), second);
    assert_eq!(std::fs::read(&path).unwrap(), original);

    let start = pages[0].data_offset as usize;
    assert_eq!(&original[start..start + first.len()], first);
    // 删除中央目录后仍能读页，证明读页只依赖已记录的数据区间。
    let data_end = pages[1].data_offset + pages[1].compressed_size;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(data_end)
        .unwrap();
    assert_eq!(read_page(&path, &pages[0]).unwrap(), first);
    assert_eq!(read_page(&path, &pages[1]).unwrap(), second);
}

#[test]
fn recognizes_zip_by_magic_and_distinguishes_corrupt_zip_from_other_formats() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-a-zip.rar");
    write_zip(&path, &[("1.bmp", Stored, &bmp(2, 3))]);
    assert_eq!(list_pages(&path).unwrap().len(), 1);
    write_zip(&path, &[]);
    assert!(list_pages(&path).unwrap().is_empty());

    let path = directory.path().join("looks-like-a-zip.zip");
    for bytes in [
        &b""[..],
        &b"PK"[..],
        &b"7z\xbc\xaf\x27\x1c"[..],
        &b"Rar!\x1a\x07\x00"[..],
        &b"%PDF-1.7"[..],
        &b"not a zip archive"[..],
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(list_pages(&path), Err(Error::UnsupportedFormat)));
    }
    std::fs::write(&path, b"PK\x03\x04broken zip").unwrap();
    assert!(matches!(list_pages(&path), Err(Error::Zip(_))));
    assert!(matches!(
        list_pages(directory.path().join("missing")),
        Err(Error::Io(_))
    ));
}

#[test]
fn reading_a_page_reports_truncation_after_the_manifest_was_built() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    for method in [Stored, Deflated] {
        write_zip(&path, &[("1.bmp", method, &bmp(17, 29))]);
        let pages = list_pages(&path).unwrap();
        let page = &pages[0];
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(page.data_offset + page.compressed_size - 1)
            .unwrap();
        assert!(read_page(&path, page).is_err(), "{method:?}");
    }
}

#[test]
fn unsupported_compression_and_encryption_are_explicit_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    write_zip(&path, &[("1.bmp", Stored, &bmp(2, 3))]);
    patch_first_entry_headers(&path, 8, 10, &12_u16.to_le_bytes());
    assert!(matches!(
        list_pages(&path),
        Err(Error::UnsupportedCompression(zip::CompressionMethod::BZIP2))
    ));

    write_zip(&path, &[("1.bmp", Stored, &bmp(2, 3))]);
    patch_first_entry_headers(&path, 6, 8, &1_u16.to_le_bytes());
    assert!(matches!(
        list_pages(&path),
        Err(Error::Zip(zip::result::ZipError::UnsupportedArchive(_)))
    ));
}

#[test]
fn corrupt_image_headers_and_entry_ranges_are_not_silently_skipped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    for method in [Stored, Deflated] {
        write_zip(
            &path,
            &[
                ("1.bmp", method, b"not an image"),
                ("2.bmp", method, &bmp(2, 3)),
            ],
        );
        assert!(
            matches!(list_pages(&path), Err(Error::Image { entry_name, .. }) if entry_name == "1.bmp")
        );
    }
    let image = bmp(2, 3);
    write_zip(&path, &[("1.bmp", Stored, &image)]);
    patch_first_entry_headers(&path, 22, 24, &(image.len() as u32 + 1).to_le_bytes());
    assert!(matches!(list_pages(&path), Err(Error::CorruptEntry { .. })));

    write_zip(&path, &[("1.bmp", Deflated, &image)]);
    patch_first_entry_headers(&path, 18, 20, &1_000_000_u32.to_le_bytes());
    assert!(matches!(list_pages(&path), Err(Error::CorruptEntry { .. })));
}

#[test]
fn dimensions_do_not_require_decompressing_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    let image = bmp(7, 11);
    // 第一个 deflate stored 块只有 BMP 头部；后面的块使用保留的 BTYPE=3。
    let mut compressed = vec![0, 26, 0, 229, 255];
    compressed.extend_from_slice(&image[..26]);
    compressed.push(7);
    write_zip(&path, &[("1.bmp", Stored, &compressed)]);
    patch_first_entry_headers(&path, 8, 10, &8_u16.to_le_bytes());
    patch_first_entry_headers(&path, 22, 24, &(image.len() as u32).to_le_bytes());

    let pages = list_pages(&path).unwrap();
    assert_eq!((pages[0].width, pages[0].height), (7, 11));
    assert!(read_page(&path, &pages[0]).is_err());
}

#[test]
fn dimensions_can_follow_large_jpeg_metadata_in_stored_and_deflated_entries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    let mut jpeg = vec![0xff, 0xd8];
    // 两个 APP1 段把 SOF 推到 64 KiB 之后，不能用固定长度前缀猜测尺寸。
    for _ in 0..2 {
        jpeg.extend_from_slice(&[0xff, 0xe1, 0xff, 0xff]);
        jpeg.resize(jpeg.len() + 65_533, 0);
    }
    jpeg.extend_from_slice(&[
        0xff, 0xc0, 0, 17, 8, 0x09, 0x29, 0x04, 0xd2, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0,
    ]);
    write_zip(
        &path,
        &[("1.jpg", Stored, &jpeg), ("2.jpg", Deflated, &jpeg)],
    );
    let pages = list_pages(&path).unwrap();
    for page in pages {
        assert_eq!((page.width, page.height), (1234, 2345));
        assert_eq!(read_page(&path, &page).unwrap(), jpeg);
    }
}

#[test]
fn deflated_page_size_mismatch_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    let image = bmp(17, 29);
    for size in [image.len() as u32 - 1, image.len() as u32 + 1] {
        write_zip(&path, &[("1.bmp", Deflated, &image)]);
        patch_first_entry_headers(&path, 22, 24, &size.to_le_bytes());
        let pages = list_pages(&path).unwrap();
        assert!(matches!(
            read_page(&path, &pages[0]),
            Err(Error::CorruptEntry { .. })
        ));
    }
}

#[test]
fn natural_order_compares_numbers_without_integer_overflow() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.zip");
    let image = bmp(2, 3);
    write_zip(
        &path,
        &[
            ("第100000000000000000000话.bmp", Stored, &image),
            ("第99999999999999999999话.bmp", Stored, &image),
            ("第0002话.bmp", Stored, &image),
            ("第2话.bmp", Stored, &image),
            ("第1话.bmp", Stored, &image),
        ],
    );
    let pages = list_pages(&path).unwrap();
    let names: Vec<_> = pages.iter().map(|page| page.entry_name.as_str()).collect();
    assert_eq!(
        names,
        [
            "第1话.bmp",
            "第0002话.bmp",
            "第2话.bmp",
            "第99999999999999999999话.bmp",
            "第100000000000000000000话.bmp",
        ]
    );
}
