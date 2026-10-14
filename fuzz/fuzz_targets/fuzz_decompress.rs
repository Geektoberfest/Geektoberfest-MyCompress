#![no_main]

use libfuzzer_sys::fuzz_target;

use mycompress_core::algo::{CompressionAlgorithm, Huffman, Rle};

fuzz_target!(|data: &[u8]| {
    let rle = Rle;
    let _ = rle.decompress(data);

    let huffman = Huffman;
    let _ = huffman.decompress(data);
});
