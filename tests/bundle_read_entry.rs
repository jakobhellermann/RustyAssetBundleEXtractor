//! An entry reads the same whether its blocks are stored raw, which borrows it from the
//! archive, or compressed, which decompresses it.
use std::borrow::Cow;
use std::io::Cursor;

use rabex::files::bundlefile::builder::BundleFileBuilder;
use rabex::files::bundlefile::{BundleFileReader, CompressionType, ExtractionConfig};

fn archive(compression: CompressionType, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut builder = BundleFileBuilder::unityfs(7, &"2020.2.2f1".parse().unwrap());
    for (path, data) in files {
        builder.add_file(path, data.as_slice()).unwrap();
    }
    let mut out = Cursor::new(Vec::new());
    builder.write(&mut out, compression).unwrap();
    out.into_inner()
}

#[test]
fn entries_read_alike_stored_raw_and_compressed() {
    // big enough to span several blocks, and an empty one between
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("a", (0..300_000u32).map(|i| (i * 7) as u8).collect()),
        ("empty", Vec::new()),
        ("b", (0..1_000u32).map(|i| (i * 13) as u8).collect()),
    ];
    for (compression, borrowed) in [(CompressionType::None, true), (CompressionType::Lz4, false)] {
        let data = archive(compression, &files);
        let reader = BundleFileReader::from_reader(
            Cursor::new(data.as_slice()),
            &ExtractionConfig::default(),
        )
        .unwrap();
        for ((path, expected), entry) in files.iter().zip(reader.files()) {
            assert_eq!(&entry.path, path);
            let read = reader.read_entry(entry).unwrap();
            assert_eq!(read.as_ref(), expected.as_slice(), "{path} {compression:?}");
            // an empty entry has nothing to decode
            assert_eq!(
                matches!(read, Cow::Borrowed(_)),
                borrowed || expected.is_empty(),
                "{path} {compression:?}"
            );
        }
    }
}
