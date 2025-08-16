#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use rabex::files::{BundleFile, bundlefile::ExtractionConfig};

fuzz_target!(|data: &[u8]| {
    let mut reader = Cursor::new(data);
    let config = ExtractionConfig::default();
    let _ = BundleFile::from_reader(&mut reader, &config);
});
