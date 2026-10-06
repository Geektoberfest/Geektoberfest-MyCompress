//! Security tests: malformed and malicious archives must be rejected before
//! anything is written to disk.

mod common;

use std::io::Cursor;

use mycompress_core::archive::{format, ArchiveReader};
use mycompress_core::crc32::crc32;
use mycompress_core::files::{self, ExtractOptions};
use mycompress_core::{Error, Registry};

/// Builds one raw entry record. `data` is stored verbatim; tests that need
/// readable data go through the writer instead.
#[allow(clippy::too_many_arguments)]
fn record(
    path: &[u8],
    kind: u8,
    mode: u32,
    mtime: u64,
    uncompressed: u64,
    compressed: u64,
    crc: u32,
    data: &[u8],
) -> Vec<u8> {
    let mut record = Vec::with_capacity(format::ENTRY_FIXED_SIZE + path.len() + data.len());
    record.extend_from_slice(&(path.len() as u16).to_le_bytes());
    record.push(kind);
    record.push(0); // reserved
    record.extend_from_slice(&mode.to_le_bytes());
    record.extend_from_slice(&mtime.to_le_bytes());
    record.extend_from_slice(&uncompressed.to_le_bytes());
    record.extend_from_slice(&compressed.to_le_bytes());
    record.extend_from_slice(&crc.to_le_bytes());
    record.extend_from_slice(path);
    record.extend_from_slice(data);
    record
}

fn file_record(path: &str, data: &[u8]) -> Vec<u8> {
    record(
        path.as_bytes(),
        0,
        0o644,
        0,
        data.len() as u64,
        data.len() as u64,
        crc32(data),
        data,
    )
}

fn archive(records: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&format::MAGIC);
    bytes.extend_from_slice(&1u16.to_le_bytes()); // format version
    bytes.extend_from_slice(&1u16.to_le_bytes()); // algorithm: RLE
    bytes.extend_from_slice(&(records.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    for record in records {
        bytes.extend_from_slice(record);
    }
    bytes
}

fn open(bytes: Vec<u8>) -> Result<ArchiveReader, Error> {
    ArchiveReader::read_from(Cursor::new(bytes), &Registry::new())
}

#[test]
fn path_traversal_is_rejected() {
    for path in [
        "../evil.txt",
        "a/../../evil.txt",
        "..",
        "a/../b",
        "a/b/../../c",
    ] {
        let bytes = archive(&[file_record(path, b"malicious")]);
        assert!(
            matches!(open(bytes), Err(Error::PathTraversal(_))),
            "traversal path '{path}' must be rejected"
        );
    }
}

#[test]
fn unsafe_paths_are_rejected() {
    let paths = [
        "/etc/passwd",
        "C:/evil.txt",
        "c:",
        "a\\b",
        "a//b",
        "a/b/",
        "",
        "a/.",
        "a/CON.txt",
        "a/NUL",
        "a/b.",
        "a/b ",
    ];
    for path in paths {
        let bytes = archive(&[file_record(path, b"x")]);
        assert!(open(bytes).is_err(), "path '{path}' must be rejected");
    }
}

#[test]
fn duplicate_paths_are_rejected() {
    let bytes = archive(&[file_record("a.txt", b"1"), file_record("a.txt", b"2")]);
    assert!(matches!(
        open(bytes),
        Err(Error::DuplicatePath(p)) if p == "a.txt"
    ));
}

#[test]
fn file_directory_conflicts_are_rejected() {
    // 'config' is stored both as a file and as a directory prefix.
    let bytes = archive(&[
        file_record("config", b"not a directory"),
        file_record("config/settings.toml", b"x"),
    ]);
    assert!(matches!(open(bytes), Err(Error::ConflictingPath(_))));
}

#[test]
fn unreasonable_sizes_are_rejected() {
    let entry = record(
        b"big.bin",
        0,
        0o644,
        0,
        format::MAX_ENTRY_UNCOMPRESSED_SIZE + 1,
        0,
        0,
        &[],
    );
    assert!(matches!(
        open(archive(&[entry])),
        Err(Error::UnreasonableSize { .. })
    ));
}

#[test]
fn implausible_entry_counts_are_rejected() {
    let bytes = archive(&[file_record("a.txt", b"hi")]);

    // Claim 10 entries while the archive contains only one.
    let mut lies = bytes.clone();
    lies[8..12].copy_from_slice(&10u32.to_le_bytes());
    assert!(matches!(open(lies), Err(Error::CorruptArchive(_))));

    // Claim more entries than the global limit allows.
    let mut huge = bytes;
    huge[8..12].copy_from_slice(&11_000_000u32.to_le_bytes());
    assert!(matches!(open(huge), Err(Error::CorruptArchive(_))));
}

#[test]
fn malformed_entries_are_rejected() {
    // Invalid entry kind.
    let bad_kind = record(b"a.txt", 9, 0o644, 0, 1, 1, 0, b"x");
    assert!(matches!(
        open(archive(&[bad_kind])),
        Err(Error::CorruptArchive(_))
    ));

    // Permission bits outside 0o777.
    let bad_mode = record(b"a.txt", 0, 0o7777, 0, 1, 1, 0, b"x");
    assert!(matches!(
        open(archive(&[bad_mode])),
        Err(Error::CorruptArchive(_))
    ));

    // Directory entries must not carry data.
    let dir_with_data = record(b"dir", 1, 0o755, 0, 5, 5, 0, b"hello");
    assert!(matches!(
        open(archive(&[dir_with_data])),
        Err(Error::CorruptArchive(_))
    ));

    // Path that is not valid UTF-8.
    let bad_utf8 = record(&[0x66, 0xFF, 0xFE], 0, 0o644, 0, 1, 1, 0, b"x");
    assert!(matches!(
        open(archive(&[bad_utf8])),
        Err(Error::CorruptArchive(_))
    ));

    // Reserved entry flag byte must be zero.
    let mut reserved = record(b"a.txt", 0, 0o644, 0, 1, 1, 0, b"x");
    let mut bytes = archive(&[reserved.clone()]);
    reserved.clear();
    bytes[format::HEADER_SIZE + 3] = 1;
    assert!(matches!(open(bytes), Err(Error::CorruptArchive(_))));
}

#[test]
fn malicious_archives_cannot_extract_outside_the_destination() {
    let dir = common::TempDir::new("traversal");
    let dest = dir.path().join("out");
    let archive_path = dir.path().join("evil.mcz");

    let bytes = archive(&[file_record("../escaped.txt", b"escaped")]);
    std::fs::write(&archive_path, bytes).unwrap();

    let result = files::extract(
        &archive_path,
        &dest,
        &ExtractOptions::default(),
        &Registry::new(),
    );
    assert!(matches!(result, Err(Error::PathTraversal(_))));
    assert!(
        !dir.path().join("escaped.txt").exists(),
        "nothing may ever be written outside the destination"
    );
}
