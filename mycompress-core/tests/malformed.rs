//! Exhaustive tests for malformed, truncated, and adversarial archive inputs.
//!
//! Verifies that corrupt archives, hostile size metadata, invalid paths, and broken
//! compression streams are safely rejected with controlled [`Error`] variants
//! and never cause panics, memory exhaustion, or undefined behavior.

use std::io::Cursor;

use proptest::prelude::*;

use mycompress_core::algo::{CompressionAlgorithm, Huffman, Rle};
use mycompress_core::archive::format::{
    ENTRY_FIXED_SIZE, FORMAT_VERSION, HEADER_SIZE, MAGIC, MAX_ENTRY_COUNT,
    MAX_ENTRY_UNCOMPRESSED_SIZE,
};
use mycompress_core::archive::{ArchiveReader, ArchiveWriter};
use mycompress_core::crc32::crc32;
use mycompress_core::{Error, Registry};

// ---------------------------------------------------------------------------
// Helpers for synthesizing low-level adversarial archive structures
// ---------------------------------------------------------------------------

fn make_header(version: u16, algo_id: u16, entry_count: u32, reserved: u32) -> Vec<u8> {
    let mut header = Vec::with_capacity(HEADER_SIZE);
    header.extend_from_slice(&MAGIC);
    header.extend_from_slice(&version.to_le_bytes());
    header.extend_from_slice(&algo_id.to_le_bytes());
    header.extend_from_slice(&entry_count.to_le_bytes());
    header.extend_from_slice(&reserved.to_le_bytes());
    header
}

#[allow(clippy::too_many_arguments)]
fn make_raw_entry(
    path: &[u8],
    kind: u8,
    reserved: u8,
    mode: u32,
    mtime: u64,
    uncompressed: u64,
    compressed: u64,
    crc: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut entry = Vec::with_capacity(ENTRY_FIXED_SIZE + path.len() + payload.len());
    entry.extend_from_slice(&(path.len() as u16).to_le_bytes());
    entry.push(kind);
    entry.push(reserved);
    entry.extend_from_slice(&mode.to_le_bytes());
    entry.extend_from_slice(&mtime.to_le_bytes());
    entry.extend_from_slice(&uncompressed.to_le_bytes());
    entry.extend_from_slice(&compressed.to_le_bytes());
    entry.extend_from_slice(&crc.to_le_bytes());
    entry.extend_from_slice(path);
    entry.extend_from_slice(payload);
    entry
}

fn make_valid_archive() -> Vec<u8> {
    let registry = Registry::new();
    let rle = registry.get_by_name("rle").unwrap();
    let mut buf = Vec::new();
    {
        let mut writer = ArchiveWriter::new(Cursor::new(&mut buf), rle).unwrap();
        writer
            .add_file("hello.txt", b"hello world", 0o644, 0)
            .unwrap();
        writer.finish().unwrap();
    }
    buf
}

fn open_archive(bytes: &[u8]) -> Result<ArchiveReader, Error> {
    ArchiveReader::read_from(Cursor::new(bytes.to_vec()), &Registry::new())
}

fn assert_open_err(bytes: &[u8]) -> Error {
    match open_archive(bytes) {
        Err(e) => e,
        Ok(_) => panic!("expected archive open to fail, but it succeeded"),
    }
}

// ---------------------------------------------------------------------------
// 1. Archive Structure & Header Validation Tests
// ---------------------------------------------------------------------------

#[test]
fn test_empty_archive_is_rejected() {
    assert!(matches!(open_archive(&[]), Err(Error::TruncatedArchive(_))));
}

#[test]
fn test_truncated_header_at_every_length() {
    let valid = make_valid_archive();
    for len in 1..HEADER_SIZE {
        let truncated = &valid[..len];
        let err = assert_open_err(truncated);
        assert!(
            matches!(err, Error::TruncatedArchive(_)),
            "expected TruncatedArchive at len {len}, got: {err:?}"
        );
    }
}

#[test]
fn test_invalid_magic_bytes_rejected() {
    for bad_magic in [
        *b"MCZ0",
        *b"MCZ2",
        *b"PK\x03\x04",
        *b"\x1f\x8b\x08\x00",
        *b"RIFF",
        *b"\0\0\0\0",
        *b"7z\xbc\xaf",
    ] {
        let mut archive = make_header(1, 1, 0, 0);
        archive[..4].copy_from_slice(&bad_magic);
        let err = assert_open_err(&archive);
        assert!(
            matches!(err, Error::BadMagic(m) if m == bad_magic),
            "expected BadMagic({bad_magic:?}), got: {err:?}"
        );
    }
}

#[test]
fn test_unsupported_format_versions_rejected() {
    for version in [0u16, 2, 3, 100, 65535] {
        let archive = make_header(version, 1, 0, 0);
        let err = assert_open_err(&archive);
        assert!(
            matches!(
                err,
                Error::UnsupportedVersion { found, supported } if found == version && supported == FORMAT_VERSION
            ),
            "expected UnsupportedVersion for {version}, got: {err:?}"
        );
    }
}

#[test]
fn test_unsupported_algorithm_ids_rejected() {
    for algo_id in [0u16, 3, 50, 500, 1023, 1024, 65535] {
        let archive = make_header(1, algo_id, 0, 0);
        let err = assert_open_err(&archive);
        assert!(
            matches!(err, Error::UnknownAlgorithm(id) if id == algo_id),
            "expected UnknownAlgorithm({algo_id}), got: {err:?}"
        );
    }
}

#[test]
fn test_non_zero_reserved_header_rejected() {
    for reserved in [1u32, 2, 0x8000_0000, 0xFFFF_FFFF] {
        let archive = make_header(1, 1, 0, reserved);
        let err = assert_open_err(&archive);
        assert!(
            matches!(err, Error::CorruptArchive(_)),
            "expected CorruptArchive for reserved={reserved}, got: {err:?}"
        );
    }
}

#[test]
fn test_entry_count_exceeding_max_limit_rejected() {
    let mut archive = make_header(1, 1, MAX_ENTRY_COUNT + 1, 0);
    // Pad enough bytes so it passes the capacity check but fails the MAX_ENTRY_COUNT check
    archive.resize(archive.len() + 1000, 0);
    let err = assert_open_err(&archive);
    assert!(
        matches!(err, Error::CorruptArchive(_)),
        "expected CorruptArchive, got: {err:?}"
    );
}

#[test]
fn test_entry_count_exceeding_file_capacity_rejected() {
    // Declares 100 entries but file is only 16 bytes
    let archive = make_header(1, 1, 100, 0);
    let err = assert_open_err(&archive);
    assert!(
        matches!(err, Error::CorruptArchive(_)),
        "expected CorruptArchive, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Entry Record & Metadata Validation Tests
// ---------------------------------------------------------------------------

#[test]
fn test_entry_record_truncated_at_every_offset() {
    let payload = b"\x00a"; // 1-byte literal RLE
    let entry = make_raw_entry(
        b"test.txt",
        0,
        0,
        0o644,
        0,
        1,
        payload.len() as u64,
        crc32(b"a"),
        payload,
    );
    let mut full = make_header(1, 1, 1, 0);
    full.extend_from_slice(&entry);

    for cut in HEADER_SIZE..(full.len() - 1) {
        let truncated = &full[..cut];
        let err = assert_open_err(truncated);
        assert!(
            matches!(err, Error::TruncatedArchive(_) | Error::CorruptArchive(_)),
            "expected TruncatedArchive/CorruptArchive at cut {cut}, got: {err:?}"
        );
    }
}

#[test]
fn test_path_length_zero_rejected() {
    let mut entry = make_raw_entry(b"", 0, 0, 0o644, 0, 0, 0, 0, &[]);
    // Overwrite path_len field to 0
    entry[0..2].copy_from_slice(&0u16.to_le_bytes());
    let mut archive = make_header(1, 1, 1, 0);
    archive.extend_from_slice(&entry);
    let err = assert_open_err(&archive);
    assert!(matches!(err, Error::CorruptArchive(_)));
}

#[test]
fn test_path_length_exceeding_max_rejected() {
    let huge_len = 4097u16;
    let mut entry = make_raw_entry(b"a", 0, 0, 0o644, 0, 0, 0, 0, &[]);
    entry[0..2].copy_from_slice(&huge_len.to_le_bytes());
    let mut archive = make_header(1, 1, 1, 0);
    archive.extend_from_slice(&entry);
    archive.resize(archive.len() + 5000, b'a');
    let err = assert_open_err(&archive);
    assert!(matches!(err, Error::CorruptArchive(_)));
}

#[test]
fn test_invalid_entry_kind_rejected() {
    for kind in [2u8, 3, 10, 255] {
        let entry = make_raw_entry(b"file.txt", kind, 0, 0o644, 0, 0, 0, 0, &[]);
        let mut archive = make_header(1, 1, 1, 0);
        archive.extend_from_slice(&entry);
        let err = assert_open_err(&archive);
        assert!(matches!(err, Error::CorruptArchive(_)));
    }
}

#[test]
fn test_non_zero_reserved_entry_byte_rejected() {
    for reserved in [1u8, 2, 0x80, 0xFF] {
        let entry = make_raw_entry(b"file.txt", 0, reserved, 0o644, 0, 0, 0, 0, &[]);
        let mut archive = make_header(1, 1, 1, 0);
        archive.extend_from_slice(&entry);
        let err = assert_open_err(&archive);
        assert!(matches!(err, Error::CorruptArchive(_)));
    }
}

#[test]
fn test_invalid_unix_mode_rejected() {
    for mode in [0o1000u32, 0o7777, 0o100644, u32::MAX] {
        let entry = make_raw_entry(b"file.txt", 0, 0, mode, 0, 0, 0, 0, &[]);
        let mut archive = make_header(1, 1, 1, 0);
        archive.extend_from_slice(&entry);
        let err = assert_open_err(&archive);
        assert!(matches!(err, Error::CorruptArchive(_)));
    }
}

#[test]
fn test_directory_with_data_rejected() {
    // Directory declaring uncompressed_size > 0
    let bad_uncompressed = make_raw_entry(b"dir", 1, 0, 0o755, 0, 10, 0, 0, &[]);
    let mut a1 = make_header(1, 1, 1, 0);
    a1.extend_from_slice(&bad_uncompressed);
    assert!(matches!(open_archive(&a1), Err(Error::CorruptArchive(_))));

    // Directory declaring compressed_size > 0
    let bad_compressed = make_raw_entry(b"dir", 1, 0, 0o755, 0, 0, 10, 0, &[]);
    let mut a2 = make_header(1, 1, 1, 0);
    a2.extend_from_slice(&bad_compressed);
    assert!(matches!(open_archive(&a2), Err(Error::CorruptArchive(_))));

    // Directory declaring crc32 != 0
    let bad_crc = make_raw_entry(b"dir", 1, 0, 0o755, 0, 0, 0, 12345, &[]);
    let mut a3 = make_header(1, 1, 1, 0);
    a3.extend_from_slice(&bad_crc);
    assert!(matches!(open_archive(&a3), Err(Error::CorruptArchive(_))));
}

#[test]
fn test_compressed_size_exceeding_file_remainder_rejected() {
    // Declares 500 compressed bytes but only 10 remain
    let entry = make_raw_entry(b"data.bin", 0, 0, 0o644, 0, 10, 500, 0, &[0u8; 10]);
    let mut archive = make_header(1, 1, 1, 0);
    archive.extend_from_slice(&entry);
    let err = assert_open_err(&archive);
    assert!(matches!(err, Error::CorruptArchive(_)));
}

#[test]
fn test_unreasonable_and_overflow_uncompressed_sizes_rejected() {
    // Size exceeding 1 TiB limit
    let entry = make_raw_entry(
        b"big.bin",
        0,
        0,
        0o644,
        0,
        MAX_ENTRY_UNCOMPRESSED_SIZE + 1,
        0,
        0,
        &[],
    );
    let mut a1 = make_header(1, 1, 1, 0);
    a1.extend_from_slice(&entry);
    assert!(matches!(
        open_archive(&a1),
        Err(Error::UnreasonableSize { .. })
    ));

    // u64::MAX size
    let entry_max = make_raw_entry(b"max.bin", 0, 0, 0o644, 0, u64::MAX, 0, 0, &[]);
    let mut a2 = make_header(1, 1, 1, 0);
    a2.extend_from_slice(&entry_max);
    assert!(matches!(
        open_archive(&a2),
        Err(Error::UnreasonableSize { .. })
    ));
}

#[test]
fn test_invalid_utf8_path_rejected() {
    let invalid_utf8 = [0x66, 0x69, 0xFF, 0xFE];
    let entry = make_raw_entry(&invalid_utf8, 0, 0, 0o644, 0, 0, 0, 0, &[]);
    let mut archive = make_header(1, 1, 1, 0);
    archive.extend_from_slice(&entry);
    let err = assert_open_err(&archive);
    assert!(matches!(err, Error::CorruptArchive(_)));
}

// ---------------------------------------------------------------------------
// 3. File Path Security & Semantics Tests
// ---------------------------------------------------------------------------

#[test]
fn test_path_traversal_attempts_rejected() {
    let traversal_paths = [
        "..",
        "../",
        "../../",
        "../evil.txt",
        "a/..",
        "a/../b",
        "a/../../b",
        "a/b/../../../etc/passwd",
        "foo/bar/../../baz",
    ];

    for path in traversal_paths {
        let entry = make_raw_entry(path.as_bytes(), 0, 0, 0o644, 0, 0, 0, 0, &[]);
        let mut archive = make_header(1, 1, 1, 0);
        archive.extend_from_slice(&entry);
        let err = assert_open_err(&archive);
        assert!(
            matches!(err, Error::PathTraversal(_)),
            "expected PathTraversal for '{path}', got: {err:?}"
        );
    }
}

#[test]
fn test_empty_path_rejected() {
    assert!(matches!(
        mycompress_core::paths::parse_archive_path(""),
        Err(Error::InvalidArchivePath(_))
    ));
}

#[test]
fn test_absolute_and_malformed_paths_rejected() {
    let malformed_paths = [
        "/",               // root
        "/a",              // absolute leading slash
        "//server/share",  // UNC-style leading double slash
        "a//b",            // empty component
        "a/",              // trailing slash
        "a\\b",            // backslash
        "\\a",             // leading backslash
        "a\0b",            // NUL byte
        ".",               // current dir component
        "a/.",             // current dir component
        "./a",             // current dir component
        "C:/autoexec.bat", // Windows drive letter
        "c:",              // drive without slash
        "a/c:d",           // colon in component
        "a/b.",            // trailing dot
        "a/b ",            // trailing space
        "CON",             // Windows reserved name
        "con.txt",         // Windows reserved name with extension
        "a/PRN",           // Windows reserved in directory
        "a/NUL.tar.gz",    // Windows reserved multi-extension
        "a/AUX/file.txt",  // Windows reserved directory
        "COM1",
        "LPT9.bin",
    ];

    for path in malformed_paths {
        let entry = make_raw_entry(path.as_bytes(), 0, 0, 0o644, 0, 0, 0, 0, &[]);
        let mut archive = make_header(1, 1, 1, 0);
        archive.extend_from_slice(&entry);
        let err = assert_open_err(&archive);
        assert!(
            matches!(err, Error::InvalidArchivePath(_)),
            "expected InvalidArchivePath for '{path}', got: {err:?}"
        );
    }
}

#[test]
fn test_duplicate_and_conflicting_paths_rejected() {
    // Duplicate path
    let e1 = make_raw_entry(b"file.txt", 0, 0, 0o644, 0, 0, 0, 0, &[]);
    let e2 = make_raw_entry(b"file.txt", 0, 0, 0o644, 0, 0, 0, 0, &[]);
    let mut archive = make_header(1, 1, 2, 0);
    archive.extend_from_slice(&e1);
    archive.extend_from_slice(&e2);
    let err = assert_open_err(&archive);
    assert!(matches!(err, Error::DuplicatePath(_)));

    // Conflicting path: 'config' is a file, but 'config/settings.toml' is inside it
    let file_parent = make_raw_entry(b"config", 0, 0, 0o644, 0, 0, 0, 0, &[]);
    let file_child = make_raw_entry(b"config/settings.toml", 0, 0, 0o644, 0, 0, 0, 0, &[]);
    let mut conflict_archive = make_header(1, 1, 2, 0);
    conflict_archive.extend_from_slice(&file_parent);
    conflict_archive.extend_from_slice(&file_child);
    let err2 = assert_open_err(&conflict_archive);
    assert!(matches!(err2, Error::ConflictingPath(_)));
}

// ---------------------------------------------------------------------------
// 4. Payload Corruption & Decompression Validation Tests
// ---------------------------------------------------------------------------

#[test]
fn test_crc32_checksum_mismatch_detected() {
    let payload = b"\x00a"; // 1-byte literal RLE
    let corrupt_crc = crc32(b"a") ^ 0xFFFF_FFFF;
    let entry = make_raw_entry(
        b"test.txt",
        0,
        0,
        0o644,
        0,
        1,
        payload.len() as u64,
        corrupt_crc,
        payload,
    );
    let mut archive = make_header(1, 1, 1, 0);
    archive.extend_from_slice(&entry);

    let mut reader = open_archive(&archive).expect("archive opens successfully");
    let err = match reader.read_entry_data(0) {
        Err(e) => e,
        Ok(_) => panic!("corrupt CRC32 must fail"),
    };
    assert!(matches!(err, Error::IntegrityCheck { .. }));
}

#[test]
fn test_decompressed_size_mismatch_detected() {
    let payload = b"\x00a"; // 1-byte literal RLE
                            // Claims uncompressed size is 100, actual decoded is 1
    let entry = make_raw_entry(
        b"test.txt",
        0,
        0,
        0o644,
        0,
        100,
        payload.len() as u64,
        crc32(b"a"),
        payload,
    );
    let mut archive = make_header(1, 1, 1, 0);
    archive.extend_from_slice(&entry);

    let mut reader = open_archive(&archive).expect("archive opens successfully");
    let err = match reader.read_entry_data(0) {
        Err(e) => e,
        Ok(_) => panic!("size mismatch must fail"),
    };
    assert!(matches!(
        err,
        Error::SizeMismatch {
            expected: 100,
            actual: 1,
            ..
        }
    ));
}

#[test]
fn test_rle_malformed_payloads_rejected() {
    let rle = Rle;

    // Truncated literal block: control byte 0x05 declares 6 bytes, but only 2 follow
    assert!(matches!(
        rle.decompress(&[0x05, b'x', b'y']),
        Err(Error::TruncatedArchive(_))
    ));

    // Truncated repeat block: control byte 0x80 without repeated payload byte
    assert!(matches!(
        rle.decompress(&[0x80]),
        Err(Error::TruncatedArchive(_))
    ));

    // Standalone control byte with nothing following
    assert!(matches!(
        rle.decompress(&[0x00]),
        Err(Error::TruncatedArchive(_))
    ));
}

#[test]
fn test_huffman_malformed_headers_rejected() {
    let huffman = Huffman;

    // Header shorter than 10 bytes
    for len in 1..10 {
        assert!(matches!(
            huffman.decompress(&vec![0u8; len]),
            Err(Error::TruncatedArchive(_))
        ));
    }

    // Non-empty stream declaring zero uncompressed length
    let mut zero_len = Vec::new();
    zero_len.extend_from_slice(&0u64.to_le_bytes());
    zero_len.extend_from_slice(&1u16.to_le_bytes());
    zero_len.push(b'a');
    assert!(matches!(
        huffman.decompress(&zero_len),
        Err(Error::CorruptArchive(_))
    ));

    // Symbol count == 0
    let mut zero_symbols = Vec::new();
    zero_symbols.extend_from_slice(&10u64.to_le_bytes());
    zero_symbols.extend_from_slice(&0u16.to_le_bytes());
    assert!(matches!(
        huffman.decompress(&zero_symbols),
        Err(Error::CorruptArchive(_))
    ));

    // Symbol count > 256
    let mut excessive_symbols = Vec::new();
    excessive_symbols.extend_from_slice(&10u64.to_le_bytes());
    excessive_symbols.extend_from_slice(&257u16.to_le_bytes());
    assert!(matches!(
        huffman.decompress(&excessive_symbols),
        Err(Error::CorruptArchive(_))
    ));
}

#[test]
fn test_huffman_single_symbol_malformations_rejected() {
    let huffman = Huffman;

    // Single symbol stream missing the symbol byte
    let mut truncated = Vec::new();
    truncated.extend_from_slice(&5u64.to_le_bytes());
    truncated.extend_from_slice(&1u16.to_le_bytes());
    assert!(matches!(
        huffman.decompress(&truncated),
        Err(Error::TruncatedArchive(_))
    ));

    // Single symbol stream with trailing bytes
    let mut trailing = truncated.clone();
    trailing.push(b'x');
    trailing.push(b'y'); // 12th byte
    assert!(matches!(
        huffman.decompress(&trailing),
        Err(Error::CorruptArchive(_))
    ));

    // Single symbol stream declaring excessive uncompressed size (> 1 GiB)
    let mut huge_single = Vec::new();
    huge_single.extend_from_slice(&((1 << 30) + 1u64).to_le_bytes());
    huge_single.extend_from_slice(&1u16.to_le_bytes());
    huge_single.push(b'x');
    assert!(matches!(
        huffman.decompress(&huge_single),
        Err(Error::CorruptArchive(_))
    ));
}

#[test]
fn test_huffman_code_table_and_bitstream_adversarial_rejected() {
    let huffman = Huffman;
    let valid = huffman.compress(b"hello world").unwrap();

    // Truncated code table
    assert!(matches!(
        huffman.decompress(&valid[..15]),
        Err(Error::TruncatedArchive(_))
    ));

    // Duplicate symbol in table
    let mut dup_table = valid.clone();
    dup_table[10] = b'e';
    dup_table[12] = b'e';
    assert!(matches!(
        huffman.decompress(&dup_table),
        Err(Error::CorruptArchive(_))
    ));

    // Code length 0 in table
    let mut zero_len = valid.clone();
    zero_len[11] = 0;
    assert!(matches!(
        huffman.decompress(&zero_len),
        Err(Error::CorruptArchive(_))
    ));

    // Code length exceeding MAX_CODE_LEN (64)
    let mut overlong_len = valid.clone();
    overlong_len[11] = 65;
    assert!(matches!(
        huffman.decompress(&overlong_len),
        Err(Error::CorruptArchive(_))
    ));

    // Declared uncompressed length exceeding maximum possible bitstream symbols
    let mut huge_len = valid.clone();
    huge_len[0..8].copy_from_slice(&1_000_000u64.to_le_bytes());
    assert!(matches!(
        huffman.decompress(&huge_len),
        Err(Error::CorruptArchive(_))
    ));

    // Truncated bitstream (cut last byte)
    assert!(matches!(
        huffman.decompress(&valid[..valid.len() - 1]),
        Err(Error::TruncatedArchive(_))
    ));

    // Non-zero padding bits in final byte
    let mut bad_padding = huffman.compress(b"ab").unwrap();
    let last = bad_padding.len() - 1;
    bad_padding[last] |= 1;
    assert!(matches!(
        huffman.decompress(&bad_padding),
        Err(Error::CorruptArchive(_))
    ));

    // Trailing garbage after bitstream
    let mut with_garbage = valid;
    with_garbage.push(0x42);
    assert!(matches!(
        huffman.decompress(&with_garbage),
        Err(Error::CorruptArchive(_))
    ));
}

// ---------------------------------------------------------------------------
// 5. Property-Based Robustness on Arbitrary Byte Sequences (No Panics)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 200,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_archive_reader_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..=4096)) {
        let registry = Registry::new();
        if let Ok(mut reader) = ArchiveReader::read_from(Cursor::new(bytes), &registry) {
            for i in 0..reader.entry_count() {
                // Must return Ok or Err, never panic
                let _ = reader.read_entry_data(i);
            }
        }
    }

    #[test]
    fn prop_rle_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..=4096)) {
        let rle = Rle;
        let _ = rle.decompress(&bytes);
    }

    #[test]
    fn prop_huffman_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..=4096)) {
        let huffman = Huffman;
        let _ = huffman.decompress(&bytes);
    }

    #[test]
    fn prop_archive_reader_robust_under_bit_mutation(
        mutation_index in 0usize..100,
        mutation_byte in any::<u8>(),
    ) {
        let valid = make_valid_archive();
        let mut mutated = valid;
        let idx = mutation_index % mutated.len();
        mutated[idx] ^= mutation_byte.max(1);

        let registry = Registry::new();
        if let Ok(mut reader) = ArchiveReader::read_from(Cursor::new(mutated), &registry) {
            for i in 0..reader.entry_count() {
                let _ = reader.read_entry_data(i);
            }
        }
    }
}
