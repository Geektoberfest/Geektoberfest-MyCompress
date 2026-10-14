#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use mycompress_core::archive::ArchiveReader;
use mycompress_core::Registry;

fuzz_target!(|data: &[u8]| {
    let registry = Registry::new();
    if let Ok(mut reader) = ArchiveReader::read_from(Cursor::new(data.to_vec()), &registry) {
        // Exercise entry inspection and bounded decompression
        for i in 0..reader.entry_count() {
            let _ = reader.read_entry_data(i);
        }
    }
});
