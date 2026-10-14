//! Bounded fuzzing smoke test for archive parsing and decompression.
//!
//! Exercises `ArchiveReader::read_from`, `Rle::decompress`, and `Huffman::decompress`
//! against thousands of generated pseudo-random and adversarially mutated byte sequences.
//!
//! Unlike long-running fuzzing campaigns, this smoke test executes in under a second
//! during regular CI and local development (`cargo test -p mycompress-core --test fuzz_smoke`)
//! while providing reproducible regression detection.

use std::io::Cursor;

use mycompress_core::algo::{CompressionAlgorithm, Huffman, Rle};
use mycompress_core::archive::{ArchiveReader, ArchiveWriter};
use mycompress_core::Registry;

/// Deterministic 64-bit splitmix PRNG for fast, reproducible pseudo-random byte generation.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn next_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let val = self.next_u64().to_le_bytes();
            let take = (len - out.len()).min(8);
            out.extend_from_slice(&val[..take]);
        }
        out
    }
}

fn build_seed_archive(algo_name: &str) -> Vec<u8> {
    let registry = Registry::new();
    let algo = registry.get_by_name(algo_name).unwrap();
    let mut buf = Vec::new();
    {
        let mut writer = ArchiveWriter::new(Cursor::new(&mut buf), algo).unwrap();
        writer.add_dir("data", 0o755, 0).unwrap();
        writer
            .add_file("data/doc.txt", b"Hello MyCompress World!", 0o644, 12345)
            .unwrap();
        writer
            .add_file("data/run.bin", &[b'Z'; 500], 0o644, 12346)
            .unwrap();
        writer.finish().unwrap();
    }
    buf
}

#[test]
fn test_fuzz_smoke_raw_arbitrary_inputs() {
    let registry = Registry::new();
    let rle = Rle;
    let huffman = Huffman;
    let mut rng = SplitMix64::new(0xDEADBEEF_CAFE1234);

    // Test 1,000 completely random byte streams of varying lengths (0 to 8,192 bytes)
    for _ in 0..1000 {
        let len = (rng.next_u64() as usize) % 8192;
        let data = rng.next_bytes(len);

        // Decompress with RLE (must never panic)
        let _ = rle.decompress(&data);

        // Decompress with Huffman (must never panic)
        let _ = huffman.decompress(&data);

        // Open archive (must never panic)
        if let Ok(mut reader) = ArchiveReader::read_from(Cursor::new(data), &registry) {
            for i in 0..reader.entry_count() {
                let _ = reader.read_entry_data(i);
            }
        }
    }
}

#[test]
fn test_fuzz_smoke_mutated_valid_archives() {
    let registry = Registry::new();
    let mut rng = SplitMix64::new(0x1337_C0DE_5EED_9999);

    for algo in ["rle", "huffman"] {
        let base_archive = build_seed_archive(algo);

        // 1. Bit-flip mutations
        for _ in 0..500 {
            let mut mutated = base_archive.clone();
            let flips = ((rng.next_u64() % 4) + 1) as usize;
            for _ in 0..flips {
                let idx = (rng.next_u64() as usize) % mutated.len();
                let bit = (rng.next_u64() % 8) as u8;
                mutated[idx] ^= 1 << bit;
            }

            if let Ok(mut reader) = ArchiveReader::read_from(Cursor::new(mutated), &registry) {
                for i in 0..reader.entry_count() {
                    let _ = reader.read_entry_data(i);
                }
            }
        }

        // 2. Truncation mutations (every possible cut offset)
        for cut in 0..base_archive.len() {
            let truncated = &base_archive[..cut];
            if let Ok(mut reader) =
                ArchiveReader::read_from(Cursor::new(truncated.to_vec()), &registry)
            {
                for i in 0..reader.entry_count() {
                    let _ = reader.read_entry_data(i);
                }
            }
        }

        // 3. Byte insertion and splicing mutations
        for _ in 0..300 {
            let mut spliced = base_archive.clone();
            let insert_pos = (rng.next_u64() as usize) % spliced.len();
            let insert_len = ((rng.next_u64() % 32) + 1) as usize;
            let insert_data = rng.next_bytes(insert_len);
            spliced.splice(insert_pos..insert_pos, insert_data);

            if let Ok(mut reader) = ArchiveReader::read_from(Cursor::new(spliced), &registry) {
                for i in 0..reader.entry_count() {
                    let _ = reader.read_entry_data(i);
                }
            }
        }
    }
}
