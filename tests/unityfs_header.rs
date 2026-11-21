//! Reads bundles whose bytes are built here, so the layout of the UnityFS header is checked
//! without any game data.

use std::io::Cursor;

use rabex::files::BundleFile;
use rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};

const CONTENT: &[u8] = b"hello bundle";

/// A one-file bundle. The block info is stored or LZ4-compressed and follows the header, padded
/// with zeros to 16 bytes when `pad_block_info` is set.
fn bundle(version: u32, revision: &str, flags: u32, pad_block_info: bool) -> Vec<u8> {
    bundle_layout(version, revision, flags, pad_block_info, 0, &[], 0)
}

fn bundle_layout(
    version: u32,
    revision: &str,
    flags: u32,
    pad_block_info: bool,
    prefix: usize,
    encryption: &[u8],
    hash: u8,
) -> Vec<u8> {
    // Block info: 16-byte hash, one block, one file
    let mut info = vec![hash; 16];
    info.extend_from_slice(&1i32.to_be_bytes());
    info.extend_from_slice(&(CONTENT.len() as u32).to_be_bytes()); // uncompressed size
    info.extend_from_slice(&(CONTENT.len() as u32).to_be_bytes()); // compressed size
    info.extend_from_slice(&0u16.to_be_bytes()); // stored
    info.extend_from_slice(&1i32.to_be_bytes());
    info.extend_from_slice(&0i64.to_be_bytes()); // offset
    info.extend_from_slice(&(CONTENT.len() as i64).to_be_bytes()); // size
    info.extend_from_slice(&0u32.to_be_bytes()); // flags
    info.extend_from_slice(b"CAB-test\0");
    let compressed = match flags & 0x3f {
        0 => info.clone(),
        1 => {
            let mut compressed = Vec::new();
            lzma_rs::lzma_compress_with_options(
                &mut info.as_slice(),
                &mut compressed,
                &lzma_rs::compress::Options {
                    unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader,
                },
            )
            .unwrap();
            compressed
        }
        _ => lz4_flex::block::compress(&info),
    };

    let mut header = vec![0x77; prefix];
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
        while !header.len().is_multiple_of(16) {
            header.push(0);
        }
    }
    header.extend_from_slice(encryption);
    if flags & 0x80 != 0 {
        header.extend_from_slice(CONTENT);
        header.extend_from_slice(&compressed);
    } else {
        header.extend_from_slice(&compressed);
        header.extend_from_slice(CONTENT);
    }
    let total = (header.len() - prefix) as i64;
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

#[test]
fn stored_zero_hash_without_padding_preserves_content() {
    let bytes = bundle(6, "2019.4.9f1", 0x40, false);
    assert_eq!(&bytes[49..65], &[0; 16]);
    assert_eq!(read(bytes).unwrap(), CONTENT);
}

#[test]
fn stored_zero_hash_with_padding_preserves_content() {
    assert_eq!(read(bundle(6, "2019.4.9f1", 0x40, true)).unwrap(), CONTENT);
}

#[test]
fn tail_block_info_preserves_data_position() {
    for flags in [0xc0, 0xc2] {
        for padded in [false, true] {
            assert_eq!(
                read(bundle(6, "2019.4.9f1", flags, padded)).unwrap(),
                CONTENT
            );
        }
    }
}

#[test]
fn version_7_stored_zero_hash_is_padded() {
    assert_eq!(read(bundle(7, "2022.3.18f1", 0x40, true)).unwrap(), CONTENT);
}

#[test]
fn empty_stored_metadata_uses_the_complete_bundle_extent() {
    for padded in [false, true] {
        let mut bytes = bundle(6, "2019.4.9f1", 0x40, false);
        bytes.truncate(49);
        bytes[37..41].copy_from_slice(&24u32.to_be_bytes());
        bytes[41..45].copy_from_slice(&24u32.to_be_bytes());
        if padded {
            bytes.resize(64, 0);
        }
        // Zero hash, zero blocks, zero directory entries.
        bytes.extend_from_slice(&[0; 24]);
        let size = bytes.len() as u64;
        bytes[29..37].copy_from_slice(&(size as i64).to_be_bytes());
        let mut cursor = Cursor::new(bytes);
        BundleFile::from_reader(&mut cursor, &ExtractionConfig::default()).unwrap();
        assert_eq!(cursor.position(), size);
    }
}

fn add_metadata_suffix(mut bytes: Vec<u8>, padded: bool) -> Vec<u8> {
    let info_start = if padded { 64 } else { 49 };
    let size = u32::from_be_bytes(bytes[37..41].try_into().unwrap());
    bytes.splice(
        info_start + size as usize..info_start + size as usize,
        [0x55],
    );
    bytes[37..41].copy_from_slice(&(size + 1).to_be_bytes());
    bytes[41..45].copy_from_slice(&(size + 1).to_be_bytes());
    let total = bytes.len() as i64;
    bytes[29..37].copy_from_slice(&total.to_be_bytes());
    bytes
}

#[test]
fn metadata_suffix_outside_optional_alignment_keeps_legacy_behavior() {
    for (version, revision, padded) in [(7, "2022.3.1f1", true), (6, "2018.4.9f1", false)] {
        let mut bytes = add_metadata_suffix(bundle(version, revision, 0x40, padded), padded);
        bytes[29..37].copy_from_slice(&0i64.to_be_bytes());
        assert_eq!(read(bytes).unwrap(), CONTENT);
    }
}

#[test]
fn bytes_after_declared_bundle_end_do_not_change_optional_alignment() {
    for flags in [0x40, 0x42, 0xc0, 0xc2] {
        for padded in [false, true] {
            let mut bytes = bundle(6, "2019.4.9f1", flags, padded);
            bytes.extend_from_slice(b"extra bytes outside the declared bundle");
            assert_eq!(read(bytes).unwrap(), CONTENT);
        }
    }
}

#[test]
fn embedded_bundles_with_absolute_padding_preserve_content() {
    for prefix in [7, 16] {
        for flags in [0x40, 0x42, 0xc0, 0xc2] {
            for padded in [false, true] {
                let bytes = bundle_layout(6, "2019.4.9f1", flags, padded, prefix, &[], 0);
                let mut cursor = Cursor::new(bytes);
                cursor.set_position(prefix as u64);
                let bundle =
                    BundleFile::from_reader(&mut cursor, &ExtractionConfig::default()).unwrap();
                assert_eq!(bundle.m_BlockData, CONTENT);
            }
        }
    }
}

#[test]
fn large_stored_bundles_keep_their_64_bit_extents() {
    use std::io::{Read, Seek, SeekFrom};
    struct Sparse {
        head: Vec<u8>,
        size: u64,
        pos: u64,
    }
    impl Read for Sparse {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = (self.size.saturating_sub(self.pos)).min(buf.len() as u64) as usize;
            for (i, byte) in buf[..n].iter_mut().enumerate() {
                *byte = self.head.get(self.pos as usize + i).copied().unwrap_or(0);
            }
            self.pos += n as u64;
            Ok(n)
        }
    }
    impl Seek for Sparse {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.pos = match from {
                SeekFrom::Start(n) => n,
                SeekFrom::Current(n) => self.pos.checked_add_signed(n).unwrap(),
                SeekFrom::End(n) => self.size.checked_add_signed(n).unwrap(),
            };
            Ok(self.pos)
        }
    }
    for (count, each) in [(2u32, 3u32 << 30), (4, 2 << 30)] {
        for padded in [false, true] {
            let mut head = bundle(6, "2019.4.9f1", 0x40, padded);
            let start = if padded { 64 } else { 49 };
            head.truncate(start + 16);
            head.extend_from_slice(&(count as i32).to_be_bytes());
            for _ in 0..count {
                head.extend_from_slice(&each.to_be_bytes());
                head.extend_from_slice(&each.to_be_bytes());
                head.extend_from_slice(&0u16.to_be_bytes());
            }
            head.extend_from_slice(&0i32.to_be_bytes());
            let info_size = (head.len() - start) as u32;
            head[37..41].copy_from_slice(&info_size.to_be_bytes());
            head[41..45].copy_from_slice(&info_size.to_be_bytes());
            let size = head.len() as u64 + u64::from(count) * u64::from(each);
            head[29..37].copy_from_slice(&(size as i64).to_be_bytes());
            let bundle = BundleFileReader::from_reader(
                Sparse { head, size, pos: 0 },
                &ExtractionConfig::default(),
            )
            .unwrap();
            assert_eq!(bundle.blocks().len(), count as usize);
        }
    }
}

#[cfg(feature = "unitycn")]
fn unitycn_header(key: [u8; 16]) -> Vec<u8> {
    use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::NoPadding};
    let signature_key = [3; 16];
    let mut signature = signature_key;
    cbc::Encryptor::<aes::Aes128>::new(&key.into(), &[0; 16].into())
        .encrypt_padded::<NoPadding>(&mut signature, 16)
        .unwrap();
    for (byte, plain) in signature.iter_mut().zip(b"#$unity3dchina!@") {
        *byte ^= plain;
    }
    [
        vec![0, 0, 0, 1],
        vec![0x11; 16],
        vec![5; 16],
        vec![0],
        signature.to_vec(),
        signature_key.to_vec(),
        vec![0],
    ]
    .concat()
}

#[cfg(feature = "unitycn")]
#[test]
fn encrypted_metadata_retries_alignment_without_panicking() {
    let key = [0x42; 16];
    let encryption = unitycn_header(key);
    for flags in [0x240, 0x242] {
        for padded in [true, false] {
            let bytes = bundle_layout(6, "2019.4.9f1", flags, padded, 0, &encryption, 0);
            let config = ExtractionConfig::default().with_unitycn_key(key);
            let parsed = BundleFile::from_reader(&mut Cursor::new(bytes.clone()), &config).unwrap();
            assert_eq!(parsed.m_BlockData, CONTENT);
            assert!(
                BundleFile::from_reader(
                    &mut Cursor::new(bytes),
                    &ExtractionConfig::default().with_unitycn_key([0; 16])
                )
                .is_err()
            );
        }
    }
}

#[test]
fn optional_alignment_preserves_older_format_versions() {
    for version in [3, 5] {
        for flags in [0x40, 0x42, 0xc0, 0xc2] {
            for padded in [false, true] {
                assert_eq!(
                    read(bundle(version, "2019.4.9f1", flags, padded)).unwrap(),
                    CONTENT
                );
            }
        }
    }
}

#[test]
fn unreliable_declared_sizes_keep_legacy_reading() {
    let mut failures = vec![];
    let mut cases = 0;
    for flags in [0x40, 0x41, 0x42, 0xc0, 0xc1, 0xc2] {
        for padded in [false, true] {
            for size in [0i64, -1, 4296] {
                let mut bytes = bundle_layout(6, "2019.4.9f1", flags, padded, 0, &[], 0xab);
                bytes[29..37].copy_from_slice(&size.to_be_bytes());
                let result = read(bytes);
                let correct = matches!(&result, Ok(bytes) if bytes == CONTENT);
                println!(
                    "legacy-size flags={flags:x} padded={padded} size={size} correct={correct}"
                );
                cases += 1;
                if !correct {
                    failures.push(format!(
                        "flags={flags:x} pad={padded} size={size}: {result:?}"
                    ));
                }
            }
            if flags & 0x80 == 0 {
                let mut bytes = bundle_layout(6, "2019.4.9f1", flags, padded, 0, &[], 0xab);
                bytes.extend_from_slice(&[0; 16]);
                let size = bytes.len() as i64;
                bytes[29..37].copy_from_slice(&size.to_be_bytes());
                let result = read(bytes);
                let correct = matches!(&result, Ok(bytes) if bytes == CONTENT);
                println!(
                    "legacy-size flags={flags:x} padded={padded} counted-zero-trailer correct={correct}"
                );
                cases += 1;
                if !correct {
                    failures.push(format!(
                        "flags={flags:x} pad={padded} counted-zero-trailer: {result:?}"
                    ));
                }
            }
        }
    }
    assert_eq!(cases, 42);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn revision_before_optional_alignment_keeps_legacy_reading() {
    let mut bytes = bundle_layout(6, "2019.3.9f1", 0x40, false, 0, &[], 0);
    let size = u32::from_be_bytes(bytes[37..41].try_into().unwrap()) as usize;
    let mut cursor = Cursor::new(bytes.clone());
    // Declared sizes in this older revision do not select a metadata position.
    bytes[29..37].copy_from_slice(&0i64.to_be_bytes());
    assert_eq!(read(bytes).unwrap(), CONTENT);
    BundleFile::from_reader(&mut cursor, &ExtractionConfig::default()).unwrap();
    assert_eq!(cursor.position(), (49 + size + CONTENT.len()) as u64);
    let padded = bundle(6, "2019.3.9f1", LZ4_COMBINED, true);
    assert!(
        BundleFileReader::from_reader(Cursor::new(padded), &ExtractionConfig::default()).is_err()
    );
}

#[test]
fn unityweb_and_unityraw_version_six_keep_the_legacy_byte() {
    for signature in [b"UnityWeb\0".as_slice(), b"UnityRaw\0".as_slice()] {
        let original = bundle_layout(6, "2018.4.9f1", 0x40, false, 0, &[], 0xab);
        let mut bytes = signature.to_vec();
        bytes.extend_from_slice(&original[8..49]);
        bytes.push(0);
        bytes.extend_from_slice(&original[49..]);
        let size_at = 29 + signature.len() - 8;
        let size = bytes.len() as i64;
        bytes[size_at..size_at + 8].copy_from_slice(&size.to_be_bytes());
        let parsed =
            BundleFile::from_reader(&mut Cursor::new(bytes), &ExtractionConfig::default()).unwrap();
        assert_eq!(parsed.m_BlockData, CONTENT);
    }
}

#[test]
fn shifted_empty_metadata_does_not_replace_a_readable_bundle() {
    let mut bytes = bundle(6, "2019.4.9f1", 0x40, true);
    let info_size = u32::from_be_bytes(bytes[37..41].try_into().unwrap());
    // The shifted zero counts fit this declared size, but leave unread metadata.
    bytes[29..37].copy_from_slice(&(49i64 + i64::from(info_size)).to_be_bytes());
    let parsed =
        BundleFile::from_reader(&mut Cursor::new(bytes), &ExtractionConfig::default()).unwrap();
    assert_eq!(parsed.m_BlockData, CONTENT);
    assert_eq!(parsed.m_DirectoryInfo.len(), 1);
}

#[test]
fn tail_metadata_without_a_cn_header_does_not_require_a_key() {
    for (version, revision, flags) in [
        (6, "2019.4.9f1", 0x2c0),
        (7, "2019.4.9f1", 0x2c0),
        (8, "2022.3.18f1", 0x4c0),
    ] {
        assert_eq!(
            read(bundle(version, revision, flags, true)).unwrap(),
            CONTENT
        );
    }
}

#[test]
fn truncated_unpadded_stored_data_is_not_selected_by_declared_size() {
    for missing in 1..=CONTENT.len() {
        let mut bytes = bundle(6, "2019.4.9f1", 0x40, false);
        bytes.truncate(bytes.len() - missing);
        assert!(
            BundleFile::from_reader(&mut Cursor::new(bytes), &ExtractionConfig::default(),)
                .is_err(),
            "missing {missing} bytes"
        );
    }
}

#[test]
fn directory_outside_stored_data_is_not_selected_by_declared_size() {
    for offset in [-1i64, 1, i64::MAX] {
        let mut bytes = bundle(6, "2019.4.9f1", 0x40, false);
        bytes[83..91].copy_from_slice(&offset.to_be_bytes());
        assert!(
            BundleFile::from_reader(&mut Cursor::new(bytes), &ExtractionConfig::default(),)
                .is_err(),
            "offset {offset}"
        );
    }
}

#[test]
fn complete_lzma_data_remains_readable() {
    for data in [CONTENT, b"".as_slice()] {
        let mut bytes = bundle(6, "2019.4.9f1", 0x40, false);
        let mut compressed = Vec::new();
        let mut input = data;
        lzma_rs::lzma_compress_with_options(
            &mut input,
            &mut compressed,
            &lzma_rs::compress::Options {
                unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader,
            },
        )
        .unwrap();
        bytes.truncate(bytes.len() - CONTENT.len());
        bytes.extend_from_slice(&compressed);
        bytes[69..73].copy_from_slice(&(data.len() as u32).to_be_bytes());
        bytes[73..77].copy_from_slice(&(compressed.len() as u32).to_be_bytes());
        bytes[78] = 1;
        bytes[91..99].copy_from_slice(&(data.len() as i64).to_be_bytes());
        let size = bytes.len() as i64;
        bytes[29..37].copy_from_slice(&size.to_be_bytes());
        let parsed = BundleFile::from_reader(
            &mut Cursor::new(bytes.clone()),
            &ExtractionConfig::default(),
        )
        .unwrap();
        assert_eq!(parsed.m_BlockData, data);
        let mut reader =
            BundleFileReader::from_reader(Cursor::new(bytes), &ExtractionConfig::default())
                .unwrap();
        assert_eq!(reader.read_at("CAB-test").unwrap().unwrap(), data);
        assert_eq!(reader.next().unwrap().read().unwrap(), data);
    }
}

#[test]
fn invalid_lzma_data_prefix_does_not_select_shifted_metadata() {
    let data = b"first block..";
    let mut bytes = bundle(6, "2019.4.9f1", 0x40, false);
    bytes.truncate(bytes.len() - CONTENT.len());
    bytes.extend_from_slice(data);
    bytes[69..73].copy_from_slice(&(data.len() as u32).to_be_bytes());
    bytes[73..77].copy_from_slice(&(data.len() as u32).to_be_bytes());
    bytes[78] = 1;
    bytes[91..99].copy_from_slice(&(data.len() as i64).to_be_bytes());
    let size = bytes.len() as i64;
    bytes[29..37].copy_from_slice(&size.to_be_bytes());
    // The properties can parse, but the range coder's first byte is nonzero.
    assert!(
        BundleFile::from_reader(&mut Cursor::new(bytes), &ExtractionConfig::default(),).is_err()
    );
}

/// Delegates to a cursor but refuses to seek from the end.
struct NoSeekFromEnd(Cursor<Vec<u8>>);

impl std::io::Read for NoSeekFromEnd {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for NoSeekFromEnd {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match pos {
            std::io::SeekFrom::End(_) => Err(std::io::Error::other("no seek from end")),
            pos => self.0.seek(pos),
        }
    }
}

#[test]
fn reader_without_seek_from_end_uses_the_fallback() {
    for padded in [false, true] {
        let bytes = bundle(6, "2019.4.9f1", LZ4_COMBINED, padded);
        let parsed = BundleFile::from_reader(
            &mut NoSeekFromEnd(Cursor::new(bytes)),
            &ExtractionConfig::default(),
        );
        assert!(parsed.is_ok(), "padded {padded}: {:?}", parsed.err());
    }
}
