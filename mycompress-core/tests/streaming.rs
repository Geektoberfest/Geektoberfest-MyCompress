//! Streaming compression tests: files are compressed in fixed-size chunks,
//! so these verify correctness exactly where chunking could break it.

mod common;

use std::fs;

use mycompress_core::algo::{AlgorithmId, MAX_DECOMPRESSED_BYTES};
use mycompress_core::files::{self, CompressOptions, ExtractOptions};
use mycompress_core::{Error, Registry};

const CHUNK: usize = 64 * 1024;

fn rle_options() -> CompressOptions {
    CompressOptions {
        algorithm: AlgorithmId::Rle,
        force: false,
    }
}

fn huffman_options() -> CompressOptions {
    CompressOptions {
        algorithm: AlgorithmId::Huffman,
        force: false,
    }
}

/// Compresses `data` as a single file and checks the extracted bytes match.
fn roundtrip_file(data: &[u8], options: &CompressOptions) {
    let dir = common::TempDir::new("stream-roundtrip");
    let input = dir.path().join("input.bin");
    let archive = dir.path().join("archive.mcz");
    let output = dir.path().join("out");
    fs::write(&input, data).unwrap();

    let stats = files::compress(&input, &archive, options, &Registry::new()).unwrap();
    assert_eq!(stats.total_uncompressed, data.len() as u64);
    assert_eq!(stats.file_count, 1);

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("input.bin")).unwrap(), data);
}

#[test]
fn empty_file_round_trips() {
    roundtrip_file(b"", &rle_options());
    roundtrip_file(b"", &huffman_options());
}

#[test]
fn rle_round_trips_across_chunk_boundaries() {
    // Just under, exactly, and just over one chunk.
    for size in [1, 1000, CHUNK - 1, CHUNK, CHUNK + 1] {
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        roundtrip_file(&data, &rle_options());
    }
    // Several chunks plus a partial tail.
    let data: Vec<u8> = (0..3 * CHUNK + 7).map(|i| (i % 251) as u8).collect();
    roundtrip_file(&data, &rle_options());
}

#[test]
fn rle_runs_straddling_chunk_edges_survive() {
    // A long run crossing the first chunk edge.
    roundtrip_file(&vec![b'A'; CHUNK + 100], &rle_options());
    // Distinct prefix pushing a run across the edge, then more data.
    let mut data: Vec<u8> = (0..CHUNK - 6).map(|i| (i % 251) as u8).collect();
    data.extend_from_slice(&[b'Q'; 200]);
    data.extend_from_slice(b"tail bytes here");
    roundtrip_file(&data, &rle_options());
    // Alternating bytes over multiple chunks (no runs at all).
    let data: Vec<u8> = (0..200_000).map(|i| b'a' + (i % 2) as u8).collect();
    roundtrip_file(&data, &rle_options());
}

#[test]
fn huffman_small_files_still_round_trip() {
    let text = b"The quick brown fox jumps over the lazy dog. ".repeat(200);
    roundtrip_file(&text, &huffman_options());
    let mixed: Vec<u8> = (0..5000).map(|i| (i % 251) as u8).collect();
    roundtrip_file(&mixed, &huffman_options());
}

#[test]
fn files_over_the_cap_fail_cleanly() {
    let dir = common::TempDir::new("over-cap");
    let input = dir.path().join("big.bin");
    fs::write(&input, vec![0u8; MAX_DECOMPRESSED_BYTES as usize + 1]).unwrap();

    for options in [rle_options(), huffman_options()] {
        let archive = dir.path().join(format!(
            "big-{}.mcz",
            match options.algorithm {
                AlgorithmId::Rle => "rle",
                AlgorithmId::Huffman => "huffman",
                AlgorithmId::Custom(id) => unreachable!("no custom id {id}"),
            }
        ));
        let err = files::compress(&input, &archive, &options, &Registry::new()).unwrap_err();
        assert!(
            matches!(err, Error::UnreasonableSize { .. }),
            "expected UnreasonableSize, got: {err:?}"
        );
        assert!(!archive.exists(), "failed compress must not leave an archive");
    }
}
