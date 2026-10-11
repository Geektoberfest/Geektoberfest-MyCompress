//! Huffman coding — built-in algorithm, id 2.
//!
//! # Format
//!
//! The compressed stream uses canonical Huffman coding with MSB-first bit
//! packing. All multi-byte header integers are little-endian.
//!
//! - Empty input produces an empty (0-byte) compressed stream.
//! - Non-empty streams begin with an 8-byte `uncompressed_len` (`u64`) followed
//!   by a 2-byte `symbol_count` (`u16`, 1..=256).
//! - Single distinct symbol (`symbol_count == 1`):
//!   Stored as the 10-byte header plus 1 byte for the symbol (11 bytes total).
//!   No code table or bitstream is emitted.
//! - Two or more symbols (`symbol_count >= 2`):
//!   Followed by `symbol_count` pairs of `(symbol: u8, code_length: u8)`.
//!   The remaining bytes contain the MSB-first bitstream, with any unused bits
//!   in the final byte padded with zeroes.
//!
//! The decoder rebuilds the canonical prefix codes from the code lengths,
//! verifies the Kraft sum, and stops decoding once `uncompressed_len` symbols
//! have been produced.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::error::{Error, Result};

use super::{AlgorithmId, CompressionAlgorithm};

/// Maximum code length supported by the canonical bit reader/writer.
const MAX_CODE_LEN: u8 = 64;

/// Upper bound on single-symbol decompression in memory (1 GiB).
const MAX_SINGLE_SYMBOL_DECOMPRESSED_SIZE: u64 = 1 << 30;

/// The Huffman compression algorithm.
#[derive(Debug, Default)]
pub struct Huffman;

impl CompressionAlgorithm for Huffman {
    fn id(&self) -> AlgorithmId {
        AlgorithmId::Huffman
    }

    fn name(&self) -> &'static str {
        "huffman"
    }

    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        Ok(encode(data))
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        decode(data)
    }
}

/// Node representation used during iterative tree construction.
struct BuildNode {
    parent: Option<usize>,
}

fn encode(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }

    let mut freqs = [0u64; 256];
    for &byte in data {
        freqs[byte as usize] += 1;
    }

    let mut used_symbols = Vec::new();
    for (byte, &count) in freqs.iter().enumerate() {
        if count > 0 {
            used_symbols.push((byte as u8, count));
        }
    }

    // A single distinct symbol cannot form a non-degenerate binary tree.
    // Store it as a fixed 11-byte record without an empty or degenerate bitstream.
    if used_symbols.len() == 1 {
        let (sym, _) = used_symbols[0];
        let mut out = Vec::with_capacity(11);
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.push(sym);
        return out;
    }

    // Build the tree iteratively using an array-backed node store. Tie-breaking by
    // (weight, min_symbol, node_index) guarantees deterministic code trees across runs.
    let mut nodes: Vec<BuildNode> = Vec::with_capacity(used_symbols.len() * 2);
    let mut leaf_indices = [None; 256];
    let mut heap: BinaryHeap<Reverse<(u64, u8, usize)>> = BinaryHeap::new();

    for &(sym, weight) in &used_symbols {
        let idx = nodes.len();
        nodes.push(BuildNode { parent: None });
        leaf_indices[sym as usize] = Some(idx);
        heap.push(Reverse((weight, sym, idx)));
    }

    while heap.len() > 1 {
        let Reverse((w1, min_sym1, idx1)) = heap.pop().expect("heap has at least 2 elements");
        let Reverse((w2, min_sym2, idx2)) = heap.pop().expect("heap has at least 1 element");

        let parent_idx = nodes.len();
        nodes[idx1].parent = Some(parent_idx);
        nodes[idx2].parent = Some(parent_idx);

        let combined_weight = w1 + w2;
        let min_symbol = min_sym1.min(min_sym2);

        nodes.push(BuildNode { parent: None });

        heap.push(Reverse((combined_weight, min_symbol, parent_idx)));
    }

    // Derive code lengths iteratively by walking parent links, avoiding recursion.
    let mut code_lengths: Vec<(u8, u8)> = Vec::with_capacity(used_symbols.len());
    for &(sym, _) in &used_symbols {
        let leaf = leaf_indices[sym as usize].expect("leaf exists");
        let mut curr = leaf;
        let mut depth = 0u8;
        while let Some(parent) = nodes[curr].parent {
            depth += 1;
            curr = parent;
        }
        code_lengths.push((sym, depth));
    }

    // Assign canonical prefix codes by sorting (code_length, symbol) ascending.
    code_lengths.sort_by_key(|&(sym, len)| (len, sym));

    let mut canonical_codes = [(0u64, 0u8); 256];
    let mut current_code: u64 = 0;
    let mut prev_len: u8 = 0;

    for (i, &(sym, len)) in code_lengths.iter().enumerate() {
        if i == 0 {
            current_code = 0;
        } else {
            let diff = len - prev_len;
            current_code = (current_code + 1) << diff;
        }
        prev_len = len;
        canonical_codes[sym as usize] = (current_code, len);
    }

    // Serialize fixed header, code table pairs, and MSB-first packed bitstream.
    let mut out = Vec::with_capacity(10 + code_lengths.len() * 2 + data.len() / 2);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&(code_lengths.len() as u16).to_le_bytes());
    for &(sym, len) in &code_lengths {
        out.push(sym);
        out.push(len);
    }

    let mut bit_writer = BitWriter::new(data.len() / 2);
    for &byte in data {
        let (code, len) = canonical_codes[byte as usize];
        bit_writer.write_bits(code, len);
    }
    out.extend(bit_writer.finish());

    out
}

/// Bit writer that accumulates bits MSB-first into bytes.
struct BitWriter {
    bytes: Vec<u8>,
    accumulator: u8,
    bits_in_acc: u8,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            accumulator: 0,
            bits_in_acc: 0,
        }
    }

    fn write_bits(&mut self, code: u64, len: u8) {
        for i in (0..len).rev() {
            let bit = ((code >> i) & 1) as u8;
            self.accumulator = (self.accumulator << 1) | bit;
            self.bits_in_acc += 1;
            if self.bits_in_acc == 8 {
                self.bytes.push(self.accumulator);
                self.accumulator = 0;
                self.bits_in_acc = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits_in_acc > 0 {
            let padded = self.accumulator << (8 - self.bits_in_acc);
            self.bytes.push(padded);
        }
        self.bytes
    }
}

/// Node in the canonical decoding tree.
struct DecodeNode {
    left: Option<u16>,
    right: Option<u16>,
    symbol: Option<u8>,
}

fn decode(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }

    if data.len() < 10 {
        return Err(Error::TruncatedArchive(
            "Huffman stream header is truncated".to_string(),
        ));
    }

    let uncompressed_len = u64::from_le_bytes(
        data[0..8]
            .try_into()
            .map_err(|_| Error::CorruptArchive("invalid header".to_string()))?,
    );
    let symbol_count = u16::from_le_bytes(
        data[8..10]
            .try_into()
            .map_err(|_| Error::CorruptArchive("invalid header".to_string()))?,
    ) as usize;

    if uncompressed_len == 0 {
        return Err(Error::CorruptArchive(
            "non-empty Huffman stream declared zero uncompressed length".to_string(),
        ));
    }

    if symbol_count == 0 || symbol_count > 256 {
        return Err(Error::CorruptArchive(format!(
            "invalid Huffman symbol count {symbol_count}"
        )));
    }

    if symbol_count == 1 {
        if data.len() < 11 {
            return Err(Error::TruncatedArchive(
                "single-symbol Huffman stream missing symbol byte".to_string(),
            ));
        }
        if data.len() > 11 {
            return Err(Error::CorruptArchive(
                "trailing bytes in single-symbol Huffman stream".to_string(),
            ));
        }
        let symbol = data[10];

        if uncompressed_len > MAX_SINGLE_SYMBOL_DECOMPRESSED_SIZE {
            return Err(Error::CorruptArchive(format!(
                "single-symbol declared length {uncompressed_len} exceeds limit"
            )));
        }

        let target_len = usize::try_from(uncompressed_len).map_err(|_| {
            Error::CorruptArchive("single-symbol declared length overflows usize".to_string())
        })?;

        let mut out = Vec::new();
        out.try_reserve_exact(target_len).map_err(|_| {
            Error::CorruptArchive("cannot allocate memory for decompression".to_string())
        })?;
        out.resize(target_len, symbol);
        return Ok(out);
    }

    let table_bytes = symbol_count.checked_mul(2).ok_or_else(|| {
        Error::CorruptArchive("code table size calculation overflowed".to_string())
    })?;
    let header_end = 10 + table_bytes;
    if data.len() < header_end {
        return Err(Error::TruncatedArchive(
            "Huffman code table is truncated".to_string(),
        ));
    }

    let mut seen = [false; 256];
    let mut pairs = Vec::with_capacity(symbol_count);

    for i in 0..symbol_count {
        let sym_idx = 10 + i * 2;
        let sym = data[sym_idx];
        let len = data[sym_idx + 1];

        if seen[sym as usize] {
            return Err(Error::CorruptArchive(format!(
                "duplicate symbol {sym} in Huffman code table"
            )));
        }
        seen[sym as usize] = true;

        if len == 0 || len > MAX_CODE_LEN {
            return Err(Error::CorruptArchive(format!(
                "code length {len} for symbol {sym} is out of range (1..={MAX_CODE_LEN})"
            )));
        }
        pairs.push((sym, len));
    }

    // Verify Kraft-McMillan equality: sum of 2^(64 - len) must equal exactly 2^64
    // for a complete binary prefix code.
    let mut kraft_sum: u128 = 0;
    for &(_, len) in &pairs {
        kraft_sum = kraft_sum
            .checked_add(1u128 << (64 - len))
            .ok_or_else(|| Error::CorruptArchive("Kraft sum calculation overflowed".to_string()))?;
    }
    if kraft_sum != (1u128 << 64) {
        return Err(Error::CorruptArchive(format!(
            "invalid prefix code table: Kraft sum is not 1 (got {kraft_sum} vs {})",
            1u128 << 64
        )));
    }

    let bitstream = &data[header_end..];
    // Each decoded symbol consumes at least 1 bit. Bounding the declared length against
    // theoretical bitstream capacity prevents unbounded allocations on corrupt headers.
    let max_possible_symbols = (bitstream.len() as u64).saturating_mul(8);
    if uncompressed_len > max_possible_symbols {
        return Err(Error::CorruptArchive(format!(
            "declared length {uncompressed_len} exceeds maximum possible symbols ({max_possible_symbols}) from bitstream"
        )));
    }

    pairs.sort_by_key(|&(sym, len)| (len, sym));

    // Reconstruct the canonical prefix tree for bitstream traversal.
    let mut tree = vec![DecodeNode {
        left: None,
        right: None,
        symbol: None,
    }];
    let mut current_code: u64 = 0;
    let mut prev_len: u8 = 0;

    for (i, &(sym, len)) in pairs.iter().enumerate() {
        if i == 0 {
            current_code = 0;
        } else {
            let diff = len - prev_len;
            current_code = (current_code + 1) << diff;
        }
        prev_len = len;

        let mut curr = 0usize;
        for bit_idx in (0..len).rev() {
            let bit = (current_code >> bit_idx) & 1;
            if bit == 0 {
                let next = match tree[curr].left {
                    Some(n) => n as usize,
                    None => {
                        let next = tree.len();
                        let next_u16 = u16::try_from(next).map_err(|_| {
                            Error::CorruptArchive("decode tree size exceeded u16".to_string())
                        })?;
                        tree[curr].left = Some(next_u16);
                        tree.push(DecodeNode {
                            left: None,
                            right: None,
                            symbol: None,
                        });
                        next
                    }
                };
                curr = next;
            } else {
                let next = match tree[curr].right {
                    Some(n) => n as usize,
                    None => {
                        let next = tree.len();
                        let next_u16 = u16::try_from(next).map_err(|_| {
                            Error::CorruptArchive("decode tree size exceeded u16".to_string())
                        })?;
                        tree[curr].right = Some(next_u16);
                        tree.push(DecodeNode {
                            left: None,
                            right: None,
                            symbol: None,
                        });
                        next
                    }
                };
                curr = next;
            }
        }
        tree[curr].symbol = Some(sym);
    }

    let target_len = usize::try_from(uncompressed_len)
        .map_err(|_| Error::CorruptArchive("declared length overflows target usize".to_string()))?;
    let mut out = Vec::with_capacity(target_len);
    let mut curr = 0usize;
    let mut finished_at = None;

    'bit_loop: for (byte_idx, &byte) in bitstream.iter().enumerate() {
        for bit_idx in (0..8).rev() {
            let bit = (byte >> bit_idx) & 1;
            let next = if bit == 0 {
                tree[curr].left
            } else {
                tree[curr].right
            };
            let Some(next_idx) = next else {
                return Err(Error::CorruptArchive(
                    "invalid Huffman code encountered in bitstream".to_string(),
                ));
            };
            curr = next_idx as usize;
            if let Some(sym) = tree[curr].symbol {
                out.push(sym);
                curr = 0;
                if out.len() == target_len {
                    finished_at = Some((byte_idx, bit_idx));
                    break 'bit_loop;
                }
            }
        }
    }

    let Some((finished_byte, finished_bit)) = finished_at else {
        return Err(Error::TruncatedArchive(format!(
            "Huffman bitstream ended prematurely: decoded {} of {target_len} symbols",
            out.len()
        )));
    };

    // Verify padding bits in the final byte are zero to detect corrupt bitstreams.
    if finished_bit > 0 {
        let padding_mask = (1u8 << finished_bit) - 1;
        if bitstream[finished_byte] & padding_mask != 0 {
            return Err(Error::CorruptArchive(
                "non-zero padding bits in Huffman bitstream".to_string(),
            ));
        }
    }

    // Reject unconsumed trailing bytes.
    if finished_byte + 1 < bitstream.len() {
        return Err(Error::CorruptArchive(format!(
            "trailing bytes in Huffman stream: {} unused bytes remain",
            bitstream.len() - (finished_byte + 1)
        )));
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let compressed = encode(data);
        let decompressed = decode(&compressed).expect("decode should succeed");
        assert_eq!(decompressed, data, "round-trip failed");
    }

    /// Linear congruential generator for deterministic pseudo-random bytes.
    fn lcg_random_bytes(seed: u64, len: usize) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
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
        roundtrip(b"a");
        let encoded = encode(b"a");
        assert_eq!(encoded.len(), 11);
    }

    #[test]
    fn single_symbol_many_copies() {
        let data = vec![b'q'; 50_000];
        roundtrip(&data);
        let encoded = encode(&data);
        assert_eq!(encoded.len(), 11);
    }

    #[test]
    fn repeated_bytes() {
        let data = b"AAAAABBBBBCCCCCDDDDD";
        roundtrip(data);
    }

    #[test]
    fn arbitrary_binary_data() {
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        roundtrip(&all_bytes);
        roundtrip(&all_bytes.repeat(10));

        let lcg_data = lcg_random_bytes(12345, 10_000);
        roundtrip(&lcg_data);
    }

    #[test]
    fn text_like_input() {
        let text = b"The quick brown fox jumps over the lazy dog. Huffman coding is optimal.";
        roundtrip(text);
    }

    #[test]
    fn decoder_rejects_truncated_data() {
        let data = b"hello world";
        let compressed = encode(data);

        // Truncated fixed header
        assert!(decode(&compressed[..4]).is_err());
        assert!(decode(&compressed[..9]).is_err());

        // Cut inside the code table
        assert!(decode(&compressed[..15]).is_err());

        // Cut inside the bitstream
        assert!(decode(&compressed[..compressed.len() - 1]).is_err());
    }

    #[test]
    fn decoder_rejects_single_symbol_malformations() {
        // Truncated header missing single-symbol byte.
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&10u64.to_le_bytes());
        truncated.extend_from_slice(&1u16.to_le_bytes());
        assert!(decode(&truncated).is_err());

        // Trailing bytes after the 11-byte single-symbol payload.
        let mut trailing = truncated.clone();
        trailing.push(b'a');
        trailing.push(b'b');
        assert!(decode(&trailing).is_err());

        // Non-empty stream declaring zero uncompressed length.
        let mut zero_len = Vec::new();
        zero_len.extend_from_slice(&0u64.to_le_bytes());
        zero_len.extend_from_slice(&1u16.to_le_bytes());
        zero_len.push(b'a');
        assert!(decode(&zero_len).is_err());
    }

    #[test]
    fn decoder_rejects_invalid_code_tables() {
        let valid = encode(b"ab");

        // Duplicate symbol in code table.
        let mut dup_sym = valid.clone();
        dup_sym[10] = b'a';
        dup_sym[12] = b'a';
        assert!(matches!(decode(&dup_sym), Err(Error::CorruptArchive(_))));

        // Code length cannot be 0.
        let mut zero_len = valid.clone();
        zero_len[11] = 0;
        assert!(matches!(decode(&zero_len), Err(Error::CorruptArchive(_))));

        // Code length cannot exceed MAX_CODE_LEN (64).
        let mut large_len = valid.clone();
        large_len[11] = 65;
        assert!(matches!(decode(&large_len), Err(Error::CorruptArchive(_))));

        // Under-subscribed code lengths (Kraft sum < 1).
        let mut under_sub = valid.clone();
        under_sub[11] = 2;
        under_sub[13] = 2;
        assert!(matches!(decode(&under_sub), Err(Error::CorruptArchive(_))));

        // Over-subscribed code lengths (Kraft sum > 1).
        let mut over_sub = Vec::new();
        over_sub.extend_from_slice(&10u64.to_le_bytes());
        over_sub.extend_from_slice(&3u16.to_le_bytes());
        over_sub.extend_from_slice(&[b'a', 1, b'b', 1, b'c', 1]);
        over_sub.push(0);
        over_sub.push(0);
        assert!(matches!(decode(&over_sub), Err(Error::CorruptArchive(_))));
    }

    #[test]
    fn decoder_rejects_inconsistent_and_hostile_lengths() {
        let valid = encode(b"abcdefgh");

        // Declared length exceeds maximum possible symbols from bitstream.
        let mut fake_len = valid.clone();
        fake_len[0..8].copy_from_slice(&100_000u64.to_le_bytes());
        assert!(matches!(decode(&fake_len), Err(Error::CorruptArchive(_))));

        // Hostile declared length on multi-symbol payload is rejected before allocation.
        let mut huge_len = valid;
        huge_len[0..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(decode(&huge_len), Err(Error::CorruptArchive(_))));

        // Hostile declared length on single-symbol payload is rejected before allocation.
        let mut huge_single = Vec::new();
        huge_single.extend_from_slice(&u64::MAX.to_le_bytes());
        huge_single.extend_from_slice(&1u16.to_le_bytes());
        huge_single.push(b'z');
        assert!(matches!(
            decode(&huge_single),
            Err(Error::CorruptArchive(_))
        ));
    }

    #[test]
    fn decoder_rejects_trailing_bytes_and_bad_padding() {
        let valid = encode(b"abcdefgh");

        // Trailing unused bytes after the bitstream.
        let mut with_trailing = valid.clone();
        with_trailing.push(0x55);
        assert!(matches!(
            decode(&with_trailing),
            Err(Error::CorruptArchive(_))
        ));

        // Non-zero padding bits in the final byte.
        let mut corrupted_padding = encode(b"ab");
        let last_idx = corrupted_padding.len() - 1;
        corrupted_padding[last_idx] |= 1;
        assert!(matches!(
            decode(&corrupted_padding),
            Err(Error::CorruptArchive(_))
        ));
    }

    #[test]
    fn measure_comparison() {
        use crate::algo::Rle;
        let rle = Rle;
        let huffman = Huffman;

        // (1) Long run of one byte: 10,000 bytes of b'A'
        let input_run = vec![b'A'; 10_000];
        let rle_run = rle.compress(&input_run).unwrap().len();
        let huff_run = huffman.compress(&input_run).unwrap().len();

        // (2) English text: sample English paragraph repeated to 1,000 bytes
        let sample = b"The quick brown fox jumps over the lazy dog. Huffman coding produces optimal prefix codes for known symbol frequencies, whereas run-length encoding excels on consecutive repetitions. ";
        let mut input_text = Vec::new();
        while input_text.len() < 1_000 {
            input_text.extend_from_slice(sample);
        }
        input_text.truncate(1_000);
        let rle_text = rle.compress(&input_text).unwrap().len();
        let huff_text = huffman.compress(&input_text).unwrap().len();

        // (3) Uniformly random bytes: 10,000 bytes
        let input_rand = lcg_random_bytes(42, 10_000);
        let rle_rand = rle.compress(&input_rand).unwrap().len();
        let huff_rand = huffman.compress(&input_rand).unwrap().len();

        // (4) Tiny input: 4 bytes
        let input_tiny = b"test";
        let rle_tiny = rle.compress(input_tiny).unwrap().len();
        let huff_tiny = huffman.compress(input_tiny).unwrap().len();

        assert_eq!(rle_run, 154);
        assert_eq!(huff_run, 11);
        assert_eq!(rle_text, 1008);
        assert_eq!(huff_text, 636);
        assert_eq!(rle_rand, 10079);
        assert_eq!(huff_rand, 10516);
        assert_eq!(rle_tiny, 5);
        assert_eq!(huff_tiny, 17);
    }
}
