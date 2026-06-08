#![cfg(feature = "embed-tpk")]

use std::io::Cursor;
use std::path::PathBuf;

use rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};
use rabex::files::{SerializedFile, serializedfile};
use rabex::tpk::TpkTypeTreeBlob;

fn fixture_bundles() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("tests/files")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "bundle"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no bundle fixtures found");
    paths
}

#[test]
fn all_fixtures_roundtrip_byte_exact() {
    let tt = TpkTypeTreeBlob::embedded();

    for bundle_path in fixture_bundles() {
        let data = std::fs::read(&bundle_path).unwrap();
        let config = ExtractionConfig::default().assume_recent_unity();
        let mut bundle = BundleFileReader::from_reader(Cursor::new(data), &config).unwrap();

        while let Some(mut file) = bundle.next_serialized() {
            let ctx = format!("{}:{}", bundle_path.display(), file.path);
            let bytes = file.read().unwrap();

            let file = SerializedFile::from_reader(&mut Cursor::new(&bytes))
                .unwrap_or_else(|e| panic!("{ctx}: read failed: {e}"));

            let com =
                serializedfile::build_common_offset_map(&tt, file.m_UnityVersion.as_ref().unwrap());
            let mut out = Cursor::new(Vec::new());
            serializedfile::write_serialized(&mut out, &file, &bytes, &com).unwrap();
            let out = out.into_inner();

            assert_eq!(out.len(), bytes.len(), "{ctx}: length differs");
            assert_eq!(out, bytes, "{ctx}: bytes differ");
        }
    }
}
