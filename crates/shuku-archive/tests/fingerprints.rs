use shuku_archive::{Error, content_hash, quick_hash};

#[test]
fn full_fingerprint_hashes_all_bytes_and_preserves_io_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book");
    for (bytes, expected) in [
        (
            &b""[..],
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            &b"abc"[..],
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        ),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(content_hash(&path).unwrap(), expected);
    }
    let bytes: Vec<_> = (0..1_000_000).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(
        content_hash(&path).unwrap(),
        blake3::hash(&bytes).to_hex().as_str()
    );
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(content_hash(&path), Err(Error::Io(_))));
}

#[test]
fn quick_fingerprint_has_a_defined_encoding_for_short_and_overlapping_samples() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book");
    std::fs::write(&path, b"abc").unwrap();
    assert_eq!(
        quick_hash(&path).unwrap(),
        blake3::hash(b"\x03\0\0\0\0\0\0\0abcabc").to_hex().as_str()
    );

    for (size, sampled_bytes) in [
        (0_u64, 0),
        (65_535, 131_070),
        (65_536, 131_072),
        (65_537, 131_072),
        (131_072, 131_072),
        (131_073, 131_072),
    ] {
        std::fs::write(&path, vec![b'x'; size as usize]).unwrap();
        let mut expected_input = size.to_le_bytes().to_vec();
        expected_input.extend(vec![b'x'; sampled_bytes]);
        assert_eq!(
            quick_hash(&path).unwrap(),
            blake3::hash(&expected_input).to_hex().as_str(),
            "size={size}"
        );
    }
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(quick_hash(&path), Err(Error::Io(_))));
}

#[test]
fn quick_fingerprint_samples_only_the_ends_but_also_includes_file_size() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book");
    let mut bytes = vec![b'a'; 65_536];
    bytes.push(b'b');
    bytes.extend(vec![b'c'; 65_536]);
    std::fs::write(&path, &bytes).unwrap();
    let quick = quick_hash(&path).unwrap();
    let full = content_hash(&path).unwrap();
    bytes[65_536] = b'd';
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(quick_hash(&path).unwrap(), quick);
    assert_ne!(content_hash(&path).unwrap(), full);

    for position in [0, 65_535, 65_537, bytes.len() - 1] {
        let previous = bytes[position];
        bytes[position] = b'e';
        std::fs::write(&path, &bytes).unwrap();
        assert_ne!(quick_hash(&path).unwrap(), quick, "position={position}");
        bytes[position] = previous;
    }
    // 追加同样的尾部字节，首尾采样内容不变，只有文件大小发生变化。
    bytes.push(b'c');
    std::fs::write(&path, &bytes).unwrap();
    assert_ne!(quick_hash(&path).unwrap(), quick);
    let moved = directory.path().join("renamed");
    std::fs::rename(&path, &moved).unwrap();
    let before = std::fs::read(&moved).unwrap();
    let moved_hash = quick_hash(&moved).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(quick_hash(&path).unwrap(), moved_hash);
    assert_eq!(std::fs::read(&moved).unwrap(), before);
}
