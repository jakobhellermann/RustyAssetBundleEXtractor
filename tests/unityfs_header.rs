//! Reads bundles whose bytes are built here, so the layout of the UnityFS header is checked
//! without any game data.

use std::io::Cursor;

use rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};

const CONTENT: &[u8] = b"hello bundle";

/// A one-file bundle. The block info is LZ4-compressed and follows the header directly, padded
/// with zeros to 16 bytes when `pad_block_info` is set.
fn bundle(version: u32, revision: &str, flags: u32, pad_block_info: bool) -> Vec<u8> {
    // Block info: 16-byte hash, one block, one file
    let mut info = vec![0u8; 16];
    info.extend_from_slice(&1i32.to_be_bytes());
    info.extend_from_slice(&(CONTENT.len() as u32).to_be_bytes()); // uncompressed size
    info.extend_from_slice(&(CONTENT.len() as u32).to_be_bytes()); // compressed size
    info.extend_from_slice(&0u16.to_be_bytes()); // stored
    info.extend_from_slice(&1i32.to_be_bytes());
    info.extend_from_slice(&0i64.to_be_bytes()); // offset
    info.extend_from_slice(&(CONTENT.len() as i64).to_be_bytes()); // size
    info.extend_from_slice(&0u32.to_be_bytes()); // flags
    info.extend_from_slice(b"CAB-test\0");
    let compressed = lz4_flex::block::compress(&info);

    let mut header = Vec::new();
    header.extend_from_slice(b"UnityFS\0");
    header.extend_from_slice(&version.to_be_bytes());
    header.extend_from_slice(b"5.x.x\0");
    header.extend_from_slice(revision.as_bytes());
    header.push(0);
    let size_at = header.len();
    header.extend_from_slice(&0i64.to_be_bytes()); // total size, filled in below
    header.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    header.extend_from_slice(&(info.len() as u32).to_be_bytes());
    header.extend_from_slice(&flags.to_be_bytes());
    if pad_block_info {
        while header.len() % 16 != 0 {
            header.push(0);
        }
    }
    header.extend_from_slice(&compressed);
    header.extend_from_slice(CONTENT);
    let total = header.len() as i64;
    header[size_at..size_at + 8].copy_from_slice(&total.to_be_bytes());
    header
}

fn read(bytes: Vec<u8>) -> std::io::Result<Vec<u8>> {
    let reader = BundleFileReader::from_reader(Cursor::new(bytes), &ExtractionConfig::default())?;
    Ok(reader.read_at("CAB-test")?.expect("the file is listed"))
}

// Compression LZ4 (2) with the block info combined with the directory (0x40)
const LZ4_COMBINED: u32 = 0x42;

#[test]
fn unity_2019_4_version_6_without_padding() {
    // The block info starts right after the flags, at offset 49
    let bytes = bundle(6, "2019.4.9f1", LZ4_COMBINED, false);
    assert_eq!(read(bytes).unwrap(), CONTENT);
}

#[test]
fn unity_2019_4_version_6_with_padding() {
    let bytes = bundle(6, "2019.4.9f1", LZ4_COMBINED, true);
    assert_eq!(read(bytes).unwrap(), CONTENT);
}

#[test]
fn unity_2022_version_7_is_padded_to_16_bytes() {
    let bytes = bundle(7, "2022.3.18f1", LZ4_COMBINED, true);
    assert_eq!(read(bytes).unwrap(), CONTENT);
}
