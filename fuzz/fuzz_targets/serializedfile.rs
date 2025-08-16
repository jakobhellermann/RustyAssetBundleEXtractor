#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use rabex::files::SerializedFile;

fuzz_target!(|data: &[u8]| {
    let mut reader = Cursor::new(data);
    let _ = SerializedFile::from_reader(&mut reader);
});
