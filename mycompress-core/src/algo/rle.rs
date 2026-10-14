//! Run-Length Encoding (RLE) — built-in algorithm, id 1.
//!
//! # Format
//!
//! The compressed stream is a sequence of blocks. Each block starts with a
//! control byte `c`:
//!
//! - `0x00..=0x7F` — *literal block*: the next `c + 1` bytes (1..=128) are
//!   copied to the output unchanged.
//! - `0x80..=0xFF` — *repeat block*: the next single byte is repeated
//!   `(c - 0x80) + 3` times (3..=130) in the output.
//!
//! The encoder only emits a repeat block for runs of 3 or more bytes, so the
//! worst-case expansion of incompressible input is about 0.8% (one control
//! byte per 128 literal bytes). Decompression expands by at most 65x, which
//! bounds how much memory a corrupted stream can force the reader to
//! allocate.
//!
//! `decompress(compress(data)) == data` holds for every input, including the
//! empty input (which compresses to the empty output). See also
//! `docs/algorithms.md`.

use crate::error::{Error, Result};

use super::{AlgorithmId, CompressionAlgorithm};

/// Longest run a single repeat block can encode.
const MAX_RUN: usize = 130;
/// Runs shorter than this are stored as literals (a 2-byte repeat block is
/// never smaller than 2 literal bytes).
const MIN_RUN: usize = 3;
/// Longest single literal block.
const MAX_LITERAL: usize = 128;

/// The RLE algorithm.
#[derive(Debug, Default)]
pub struct Rle;

impl CompressionAlgorithm for Rle {
    fn id(&self) -> AlgorithmId {
        AlgorithmId::Rle
    }

    fn name(&self) -> &'static str {
        "rle"
    }

    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        Ok(encode(data))
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        decode(data)
    }
}

fn encode(data: &[u8]) -> Vec<u8> {
    fn flush_literals(out: &mut Vec<u8>, literals: &[u8]) {
        for chunk in literals.chunks(MAX_LITERAL) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
    }

    let mut out = Vec::with_capacity(data.len() / 2);
    let mut literal_start = 0usize;
    let mut i = 0usize;

    while i < data.len() {
        let byte = data[i];
        let mut run = 1usize;
        while i + run < data.len() && data[i + run] == byte && run < MAX_RUN {
            run += 1;
        }
        if run >= MIN_RUN {
            flush_literals(&mut out, &data[literal_start..i]);
            out.push(0x80 | (run - MIN_RUN) as u8);
            out.push(byte);
            i += run;
            literal_start = i;
        } else {
            // Too short to be worth a repeat block; stays pending as literal.
            i += run;
        }
    }
    flush_literals(&mut out, &data[literal_start..]);
    out
}

fn decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0usize;

    while i < data.len() {
        let control = data[i];
        i += 1;
        if control < 0x80 {
            let len = usize::from(control) + 1;
            let end = i
                .checked_add(len)
                .ok_or_else(|| Error::CorruptArchive("RLE literal length overflows".to_string()))?;
            if end > data.len() {
                return Err(Error::TruncatedArchive(format!(
                    "RLE literal block needs {len} bytes but only {} remain",
                    data.len() - i
                )));
            }
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else {
            if i >= data.len() {
                return Err(Error::TruncatedArchive(
                    "RLE repeat block is missing its payload byte".to_string(),
                ));
            }
            let run = (control - 0x80) as usize + MIN_RUN;
            let new_len = out.len().checked_add(run).ok_or_else(|| {
                Error::CorruptArchive("RLE decompressed size overflows usize".to_string())
            })?;
            out.resize(new_len, data[i]);
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let compressed = encode(data);
        let decompressed = decode(&compressed).expect("decode should succeed");
        assert_eq!(decompressed, data, "round-trip failed for {data:?}");
    }

    /// Deterministic pseudo-random data (xorshift), so tests need no RNG crate.
    fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
        let mut state = seed | 1;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out.push((state >> 33) as u8);
        }
        out
    }

    #[test]
    fn empty_input() {
        roundtrip(b"");
        assert!(encode(b"").is_empty());
        assert!(decode(b"").unwrap().is_empty());
    }

    #[test]
    fn single_byte() {
        assert_eq!(encode(b"a"), vec![0x00, b'a']);
        roundtrip(b"a");
    }

    #[test]
    fn two_identical_bytes_stay_literal() {
        assert_eq!(encode(b"aa"), vec![0x01, b'a', b'a']);
        roundtrip(b"aa");
    }

    #[test]
    fn three_identical_bytes_use_a_repeat_block() {
        assert_eq!(encode(b"aaa"), vec![0x80, b'a']);
        roundtrip(b"aaa");
    }

    #[test]
    fn maximum_repeat_block() {
        assert_eq!(encode(&[b'z'; 130]), vec![0xFF, b'z']);
    }

    #[test]
    fn run_longer_than_one_block() {
        let data = vec![b'z'; 131];
        // 130-byte repeat block + 1 literal byte.
        assert_eq!(encode(&data), vec![0xFF, b'z', 0x00, b'z']);
        roundtrip(&data);
    }

    #[test]
    fn very_long_run_round_trips() {
        let data = vec![7u8; 100_000];
        let encoded = encode(&data);
        // 769 full blocks of 130 + one block of 30 = 770 blocks x 2 bytes.
        assert_eq!(encoded.len(), 1_540);
        roundtrip(&data);
    }

    #[test]
    fn literal_block_boundaries() {
        // 128 bytes with no run >= 3: exactly one literal block.
        let data: Vec<u8> = (0..128u16).map(|i| (i % 251) as u8).collect();
        let encoded = encode(&data);
        assert_eq!(encoded.len(), 129);
        assert_eq!(&encoded[1..], &data[..]);
        roundtrip(&data);

        // 129 bytes: two literal blocks.
        let data: Vec<u8> = (0..129u16).map(|i| (i % 251) as u8).collect();
        let encoded = encode(&data);
        assert_eq!(encoded.len(), 131);
        roundtrip(&data);
    }

    #[test]
    fn alternating_and_binary_data() {
        let alternating: Vec<u8> = (0..1_000).map(|i| b'a' + (i % 2) as u8).collect();
        roundtrip(&alternating);

        let all_values: Vec<u8> = (0..=255u8).collect();
        roundtrip(&all_values);
        roundtrip(&all_values.repeat(200));

        let mixed = [vec![0u8; 5_000], b"hello world".to_vec(), vec![0xFF; 700]];
        roundtrip(&mixed.concat());
    }

    #[test]
    fn pseudo_random_data() {
        for (seed, len) in [
            (1u64, 1usize),
            (2, 17),
            (3, 1_024),
            (4, 65_536),
            (5, 100_003),
        ] {
            roundtrip(&random_bytes(seed, len));
        }
    }

    #[test]
    fn expansion_of_incompressible_data_is_bounded() {
        let data = random_bytes(99, 50_000);
        let encoded = encode(&data);
        assert!(
            encoded.len() <= data.len() + data.len() / 128 + 2,
            "worst-case expansion exceeded: {} -> {}",
            data.len(),
            encoded.len()
        );
    }

    #[test]
    fn decoder_handles_every_control_value() {
        let mut stream = Vec::new();
        let mut expected = Vec::new();
        for control in 0..=0x7Fu8 {
            stream.push(control);
            for _ in 0..=control {
                stream.push(b'L');
                expected.push(b'L');
            }
        }
        for control in 0x80..=0xFFu8 {
            stream.push(control);
            stream.push(b'R');
            let run = (control - 0x80) as usize + MIN_RUN;
            expected.resize(expected.len() + run, b'R');
        }
        assert_eq!(decode(&stream).unwrap(), expected);
    }

    #[test]
    fn decoder_rejects_truncated_streams() {
        // "hello world aaa" encodes as a 13-byte literal block + a repeat block.
        let encoded = encode(b"hello world aaa");
        assert_eq!(encoded.len(), 15);

        assert!(decode(&encoded[..1]).is_err()); // control byte, no payload
        assert!(decode(&encoded[..12]).is_err()); // cut inside the literal block
        assert!(decode(&encoded[..13]).is_ok()); // complete literal block
        assert!(decode(&encoded[..14]).is_err()); // repeat control without payload
        assert!(decode(&[0x05, b'a']).is_err()); // literal claims 6 bytes, has 1
        assert!(decode(&[0x80]).is_err()); // repeat without payload
    }
}
