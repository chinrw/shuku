#![allow(dead_code)]

use std::fs::File;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub fn bmp(width: u32, height: u32) -> Vec<u8> {
    let pixels = (width * 3).next_multiple_of(4) * height;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(54 + pixels).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.resize((54 + pixels) as usize, 0);
    bytes
}

pub fn write_zip(path: &Path, entries: &[(&str, CompressionMethod, &[u8])]) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    for (name, method, bytes) in entries {
        zip.start_file(
            *name,
            SimpleFileOptions::default().compression_method(*method),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

pub fn patch_first_entry_headers(
    path: &Path,
    local_field: usize,
    central_field: usize,
    value: &[u8],
) {
    let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    let entry = zip.by_index_raw(0).unwrap();
    let local = entry.header_start() as usize;
    let central = entry.central_header_start() as usize;
    let mut bytes = std::fs::read(path).unwrap();
    bytes[local + local_field..local + local_field + value.len()].copy_from_slice(value);
    bytes[central + central_field..central + central_field + value.len()].copy_from_slice(value);
    std::fs::write(path, bytes).unwrap();
}
