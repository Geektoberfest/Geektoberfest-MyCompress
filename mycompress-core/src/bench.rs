//! Benchmarking utilities for compression algorithms.
//!
//! Provides execution timing and round-trip verification across multiple iterations
//! for any implementation of [`CompressionAlgorithm`].

use std::time::{Duration, Instant};

use crate::algo::CompressionAlgorithm;
use crate::error::{Error, Result};

/// Results of benchmarking a single compression algorithm on an input buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchResult {
    /// The display name of the algorithm.
    pub algorithm: &'static str,
    /// Uncompressed input size in bytes.
    pub input_size: u64,
    /// Compressed output size in bytes.
    pub output_size: u64,
    /// Median compression duration across all iterations.
    pub compress: Duration,
    /// Median decompression duration across all iterations.
    pub decompress: Duration,
}

/// Computes the median duration of a non-empty slice of durations.
///
/// The slice is sorted in place. For an even number of elements, the lower-middle
/// element is returned (i.e. index `(len - 1) / 2`) rather than an interpolated average.
/// This ensures the reported duration corresponds to an actual observed measurement
/// without floating-point precision loss.
fn median(durations: &mut [Duration]) -> Duration {
    assert!(
        !durations.is_empty(),
        "cannot compute median of empty slice"
    );
    durations.sort_unstable();
    durations[(durations.len() - 1) / 2]
}

/// Runs a benchmark of `algorithm` on `data` for the given number of `iterations`.
///
/// On each iteration, compresses and decompresses the data, recording elapsed time
/// using [`Instant::now`]. The decompressed data is verified against `data` on every
/// iteration; if verification fails or an algorithm returns an error, an error is returned
/// and no benchmark results are produced.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if `iterations` is 0 or if decompressed output
/// does not match `data`. Propagates any error returned by [`CompressionAlgorithm::compress`]
/// or [`CompressionAlgorithm::decompress`].
pub fn run(
    algorithm: &dyn CompressionAlgorithm,
    data: &[u8],
    iterations: u32,
) -> Result<BenchResult> {
    if iterations == 0 {
        return Err(Error::InvalidInput(
            "iterations must be greater than zero".to_string(),
        ));
    }

    let mut compress_durations = Vec::with_capacity(iterations as usize);
    let mut decompress_durations = Vec::with_capacity(iterations as usize);
    let mut output_size = 0u64;

    for _ in 0..iterations {
        let start_compress = Instant::now();
        let compressed = algorithm.compress(std::hint::black_box(data))?;
        let compress_elapsed = start_compress.elapsed();
        let _ = std::hint::black_box(&compressed);

        output_size = compressed.len() as u64;

        let start_decompress = Instant::now();
        let decompressed = algorithm.decompress(std::hint::black_box(&compressed))?;
        let decompress_elapsed = start_decompress.elapsed();
        let _ = std::hint::black_box(&decompressed);

        if decompressed != data {
            return Err(Error::InvalidInput(format!(
                "verification failed for algorithm '{}': decompressed data does not match original",
                algorithm.name()
            )));
        }

        compress_durations.push(compress_elapsed);
        decompress_durations.push(decompress_elapsed);
    }

    Ok(BenchResult {
        algorithm: algorithm.name(),
        input_size: data.len() as u64,
        output_size,
        compress: median(&mut compress_durations),
        decompress: median(&mut decompress_durations),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::{AlgorithmId, Rle};

    #[test]
    fn median_single_element() {
        let mut d = vec![Duration::from_millis(10)];
        assert_eq!(median(&mut d), Duration::from_millis(10));
    }

    #[test]
    fn median_odd_count() {
        let mut d = vec![
            Duration::from_millis(30),
            Duration::from_millis(10),
            Duration::from_millis(20),
        ];
        assert_eq!(median(&mut d), Duration::from_millis(20));
    }

    #[test]
    fn median_even_count() {
        // For 4 elements, sorted indices 0, 1, 2, 3: lower-middle is index 1.
        let mut d = vec![
            Duration::from_millis(40),
            Duration::from_millis(10),
            Duration::from_millis(30),
            Duration::from_millis(20),
        ];
        assert_eq!(median(&mut d), Duration::from_millis(20));
    }

    #[test]
    fn median_unsorted_input() {
        let mut d = vec![
            Duration::from_micros(900),
            Duration::from_micros(100),
            Duration::from_micros(500),
            Duration::from_micros(300),
            Duration::from_micros(700),
        ];
        assert_eq!(median(&mut d), Duration::from_micros(500));
    }

    #[test]
    fn run_rle_small_input() {
        let rle = Rle;
        let data = b"AAAAABBBBBCCCCCDDDDD";
        let res = run(&rle, data, 5).expect("benchmark run succeeds");
        assert_eq!(res.algorithm, "rle");
        assert_eq!(res.input_size, data.len() as u64);
        let expected_compressed = rle.compress(data).unwrap();
        assert_eq!(res.output_size, expected_compressed.len() as u64);
    }

    #[test]
    fn run_rejects_zero_iterations() {
        let rle = Rle;
        let err = run(&rle, b"hello", 0).unwrap_err();
        match err {
            Error::InvalidInput(msg) => {
                assert!(msg.contains("greater than zero"), "error message: {msg}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    /// Custom algorithm that produces faulty decompression output.
    struct CorruptingAlgorithm;

    impl CompressionAlgorithm for CorruptingAlgorithm {
        fn id(&self) -> AlgorithmId {
            AlgorithmId::Custom(2000)
        }

        fn name(&self) -> &'static str {
            "corrupting-algo"
        }

        fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
            Ok(data.to_vec())
        }

        fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
            let mut bad = data.to_vec();
            bad.push(0xFF);
            Ok(bad)
        }
    }

    #[test]
    fn run_verification_failure() {
        let algo = CorruptingAlgorithm;
        let err = run(&algo, b"test-data", 3).unwrap_err();
        match err {
            Error::InvalidInput(msg) => {
                assert!(
                    msg.contains("corrupting-algo"),
                    "error message must name the algorithm: {msg}"
                );
            }
            other => panic!("expected InvalidInput naming algorithm, got {other:?}"),
        }
    }

    #[test]
    fn run_empty_input_succeeds() {
        let rle = Rle;
        let res = run(&rle, b"", 3).expect("empty input benchmark succeeds");
        assert_eq!(res.algorithm, "rle");
        assert_eq!(res.input_size, 0);
        assert_eq!(res.output_size, 0);
    }
}
