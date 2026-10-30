use shuku_store::{ArchiveMetadata, CompressionMethod, Page};

pub fn metadata(path: &str) -> ArchiveMetadata {
    ArchiveMetadata {
        path: path.into(),
        title: "漫画 10".into(),
        summary: "测试摘要".into(),
        file_size: 1000,
        mtime: 1_700_000_001,
        quick_hash: "quick".into(),
        content_hash: None,
        date_added: 1_700_000_000,
        source_url: None,
        is_new: true,
        lrr_id: None,
    }
}

pub fn pages() -> Vec<Page> {
    vec![
        Page {
            entry_name: "2.jpg".into(),
            data_offset: 42,
            compressed_size: 100,
            size: 100,
            method: CompressionMethod::Stored,
            width: 800,
            height: 1200,
        },
        Page {
            entry_name: "10.jpg".into(),
            data_offset: 184,
            compressed_size: 200,
            size: 300,
            method: CompressionMethod::Deflate,
            width: 900,
            height: 1300,
        },
    ]
}
