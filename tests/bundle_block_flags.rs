//! The storage block flags of written archives are the ones Unity writes: each of Unity's
//! archives, written again with its own compression, has exactly its blocks' flags.
#![cfg_attr(not(feature = "compression-lz4hc"), expect(unused))]
use std::io::Cursor;

use rabex::files::BundleFile;
use rabex::files::bundlefile::{self, BundleFileReader, CompressionType, ExtractionConfig};

fn block_flags(data: &[u8]) -> Vec<u32> {
    let reader =
        BundleFileReader::from_reader(Cursor::new(data), &ExtractionConfig::default()).unwrap();
    reader.blocks().iter().map(|block| block.flags).collect()
}

#[test]
#[cfg(feature = "compression-lz4hc")]
fn written_blocks_are_flagged_like_unitys() {
    let mut checked = 0;
    for entry in std::fs::read_dir("tests/files").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "bundle") {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let unitys = block_flags(&data);
        let compression = CompressionType::try_from(unitys[0] & 0x3f).unwrap();

        let config = ExtractionConfig::default();
        let bundle = BundleFile::from_reader(&mut Cursor::new(data.as_slice()), &config).unwrap();
        let mut out = Cursor::new(Vec::new());
        bundlefile::write_bundle(
            &bundle.m_Header,
            &mut out,
            CompressionType::None,
            compression,
            &bundle.m_DirectoryInfo,
            &bundle.m_BlockData,
        )
        .unwrap();
        assert_eq!(block_flags(&out.into_inner()), unitys, "{}", path.display());
        checked += 1;
    }
    assert!(checked > 0);
}
