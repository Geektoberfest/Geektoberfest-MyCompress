//! Benchmark comparing automatic algorithm selection against explicit RLE and Huffman.
//!
//! Measures:
//! 1. Selection latency on 64 KiB bounded samples.
//! 2. Compressed sizes across representative inputs (repetitive, text, incompressible).
//! 3. Compression time for explicit vs. automatic algorithm paths.

use std::time::Instant;

use mycompress_core::algo::{select_algorithm, AlgorithmId};
use mycompress_core::Registry;

fn lcg_random(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        out.push((state >> 33) as u8);
    }
    out
}

struct BenchmarkRow {
    category: &'static str,
    original_size: usize,
    rle_size: usize,
    rle_time_us: u128,
    huffman_size: usize,
    huffman_time_us: u128,
    selected_algorithm: AlgorithmId,
    selection_time_us: u128,
}

fn benchmark_input(category: &'static str, data: &[u8], registry: &Registry) -> BenchmarkRow {
    let rle = registry.get(1).unwrap();
    let huffman = registry.get(2).unwrap();

    // 1. Explicit RLE
    let start = Instant::now();
    let rle_compressed = rle.compress(data).unwrap();
    let rle_time_us = start.elapsed().as_micros();

    // 2. Explicit Huffman
    let start = Instant::now();
    let huffman_compressed = huffman.compress(data).unwrap();
    let huffman_time_us = start.elapsed().as_micros();

    // 3. Selection on sample (bounded to 64 KiB)
    let sample = if data.len() > 64 * 1024 {
        &data[..64 * 1024]
    } else {
        data
    };
    let start = Instant::now();
    let selected = select_algorithm(sample, registry);
    let selection_time_us = start.elapsed().as_micros();

    BenchmarkRow {
        category,
        original_size: data.len(),
        rle_size: rle_compressed.len(),
        rle_time_us,
        huffman_size: huffman_compressed.len(),
        huffman_time_us,
        selected_algorithm: selected,
        selection_time_us,
    }
}

#[test]
fn benchmark_algorithm_selection() {
    let registry = Registry::new();

    // Input 1: Highly repetitive (10,000 bytes)
    let mut repetitive = Vec::with_capacity(10_000);
    for _ in 0..100 {
        repetitive.extend_from_slice(&[b'A'; 50]);
        repetitive.extend_from_slice(&[b'B'; 50]);
    }
    let row1 = benchmark_input("Repetitive data", &repetitive, &registry);

    // Input 2: English text (10,000 bytes)
    let sample = b"The quick brown fox jumps over the lazy dog. Canonical Huffman coding is effective for text with skewed symbol frequencies, producing compact variable-length bitstreams. ";
    let mut text = Vec::with_capacity(10_000);
    while text.len() < 10_000 {
        text.extend_from_slice(sample);
    }
    text.truncate(10_000);
    let row2 = benchmark_input("English text", &text, &registry);

    // Input 3: Incompressible random data (10,000 bytes)
    let random_data = lcg_random(12345, 10_000);
    let row3 = benchmark_input("Random/Incompressible", &random_data, &registry);

    println!("\n=== Benchmark: Algorithm Selection Comparison ===");
    println!(
        "{:<22} | {:<8} | {:<16} | {:<16} | {:<10} | {:<12}",
        "Input", "Original", "RLE Size (Time)", "Huff Size (Time)", "Auto Pick", "Select Time"
    );
    println!("{:-<102}", "");

    for row in &[&row1, &row2, &row3] {
        println!(
            "{:<22} | {:>8} | {:>8} ({:>4} µs) | {:>8} ({:>4} µs) | {:<10} | {:>9} µs",
            row.category,
            row.original_size,
            format!("{} B", row.rle_size),
            row.rle_time_us,
            format!("{} B", row.huffman_size),
            row.huffman_time_us,
            row.selected_algorithm.to_string(),
            row.selection_time_us
        );
    }
    println!("{:-<102}\n", "");

    // Assert that the selection logic correctly identifies the winning or fallback algorithm:
    assert!(
        matches!(
            row1.selected_algorithm,
            AlgorithmId::Rle | AlgorithmId::Huffman
        ),
        "repetitive data must choose a compression algorithm"
    );
    assert_eq!(
        row2.selected_algorithm,
        AlgorithmId::Huffman,
        "text data must select Huffman"
    );
    assert_eq!(
        row3.selected_algorithm,
        AlgorithmId::Rle,
        "incompressible data must fall back to Rle"
    );
}
