//! Round-trip and corruption tests for the `.mcz` archive layer.

use std::io::Cursor;

use mycompress_core::archive::format;
use mycompress_core::archive::{ArchiveReader, ArchiveWriter, EntryKind};
use mycompress_core::{Error, Registry};

fn write_archive(files: &[(&str, &[u8])], dirs: &[&str]) -> Vec<u8> {
    let registry = Registry::new();
    let algorithm = registry.get_by_name("rle").unwrap();
    let mut buffer = Vec::new();
    {
        let mut writer = ArchiveWriter::new(Cursor::new(&mut buffer), algorithm).unwrap();
        for path in dirs {
            writer.add_dir(path, 0o755, 0).unwrap();
        }
        for (path, data) in files {
            writer.add_file(path, data, 0o644, 0).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer
}

#[test]
fn round_trips_multiple_entries() {
    let files = [
        ("hello.txt", &b"hello world"[..]),
        ("empty.bin", &b""[..]),
        ("data/repeats.bin", &b"AAAAAAAAAA"[..]),
        ("unicode/nihongo.txt", "こんにちは".as_bytes()),
    ];
    let dirs = ["data", "unicode", "empty-dir"];
    let bytes = write_archive(&files, &dirs);

    let registry = Registry::new();
    let mut reader = ArchiveReader::read_from(Cursor::new(bytes), &registry).unwrap();
    assert_eq!(reader.format_version(), 1);
    assert_eq!(reader.algorithm_id(), 1);
    assert_eq!(reader.algorithm_name(), "rle");
    assert_eq!(reader.entry_count(), files.len() + dirs.len());

    for (path, data) in &files {
        let index = reader
            .entries()
            .iter()
            .position(|e| e.path == *path)
            .unwrap_or_else(|| panic!("missing entry '{path}'"));
        assert_eq!(reader.entries()[index].kind, EntryKind::File);
        assert_eq!(reader.entries()[index].uncompressed_size, data.len() as u64);
        assert_eq!(reader.read_entry_data(index).unwrap(), *data);
    }
    for path in &dirs {
        assert!(
            reader
                .entries()
                .iter()
                .any(|e| e.path == *path && e.kind == EntryKind::Directory),
            "missing directory entry '{path}'"
        );
    }
}

#[test]
fn empty_archive_round_trips() {
    let bytes = write_archive(&[], &[]);
    let reader = ArchiveReader::read_from(Cursor::new(bytes), &Registry::new()).unwrap();
    assert!(reader.entries().is_empty());
}

#[test]
fn repetitive_data_is_actually_compressed() {
    let data = vec![b'A'; 10_000];
    let bytes = write_archive(&[("big.bin", data.as_slice())], &[]);
    let registry = Registry::new();
    let mut reader = ArchiveReader::read_from(Cursor::new(bytes), &registry).unwrap();
    let entry = &reader.entries()[0];
    assert_eq!(entry.uncompressed_size, 10_000);
    assert!(
        entry.compressed_size < 200,
        "RLE should crush repetitive data, got {}",
        entry.compressed_size
    );
    assert_eq!(reader.read_entry_data(0).unwrap(), data);
}

#[test]
fn writer_rejects_duplicate_and_conflicting_paths() {
    let registry = Registry::new();
    let algorithm = registry.get_by_name("rle").unwrap();
    let mut buffer = Vec::new();
    let mut writer = ArchiveWriter::new(Cursor::new(&mut buffer), algorithm).unwrap();
    writer.add_file("a", &[1, 2, 3], 0o644, 0).unwrap();
    assert!(matches!(
        writer.add_file("a", &[4, 5, 6], 0o644, 0),
        Err(Error::DuplicatePath(_))
    ));
    assert!(matches!(
        writer.add_file("a/b", &[4, 5, 6], 0o644, 0),
        Err(Error::ConflictingPath(_))
    ));
    writer.finish().unwrap();
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = write_archive(&[("a.txt", &b"hello"[..])], &[]);
    bytes[0] = b'X';
    assert!(matches!(
        ArchiveReader::read_from(Cursor::new(bytes), &Registry::new()),
        Err(Error::BadMagic(_))
    ));
}

#[test]
fn unsupported_version_is_rejected() {
    let mut bytes = write_archive(&[("a.txt", &b"hello"[..])], &[]);
    bytes[4] = 2; // version low byte: 1 -> 2
    assert!(matches!(
        ArchiveReader::read_from(Cursor::new(bytes), &Registry::new()),
        Err(Error::UnsupportedVersion { found: 2, .. })
    ));
}

#[test]
fn unknown_algorithm_is_rejected() {
    let mut bytes = write_archive(&[("a.txt", &b"hello"[..])], &[]);
    bytes[6] = 0xFF; // algorithm id low byte -> 0x7FFF (unregistered)
    bytes[7] = 0x7F;
    assert!(matches!(
        ArchiveReader::read_from(Cursor::new(bytes), &Registry::new()),
        Err(Error::UnknownAlgorithm(0x7FFF))
    ));
}

#[test]
fn truncated_archives_are_rejected() {
    let bytes = write_archive(&[("a.txt", &b"hello world aaa"[..])], &[]);
    // 16-byte header + 36-byte record + 5-byte path + 15-byte RLE stream.
    assert_eq!(bytes.len(), 72);
    for cut in [0usize, 1, 4, 8, 15, 16, 20, 40, 52, 57, 71] {
        assert!(
            ArchiveReader::read_from(Cursor::new(bytes[..cut].to_vec()), &Registry::new()).is_err(),
            "expected an error for an archive truncated to {cut} bytes"
        );
    }
    // The complete archive is fine.
    assert!(ArchiveReader::read_from(Cursor::new(bytes), &Registry::new()).is_ok());
}

#[test]
fn corrupted_data_fails_verification() {
    let bytes = write_archive(&[("a.txt", &b"hello world hello world"[..])], &[]);
    let len = bytes.len();
    let mut corrupted = bytes.clone();
    corrupted[len - 1] ^= 0xFF; // flip the last byte of the compressed stream
    let registry = Registry::new();
    let mut reader = ArchiveReader::read_from(Cursor::new(corrupted), &registry).unwrap();
    assert!(reader.read_entry_data(0).is_err());
}

#[test]
fn corrupted_checksum_is_reported() {
    let bytes = write_archive(&[("a.txt", &b"hello"[..])], &[]);
    let crc_offset = format::HEADER_SIZE + 32;
    let mut corrupted = bytes.clone();
    corrupted[crc_offset] ^= 0xFF;
    let registry = Registry::new();
    let mut reader = ArchiveReader::read_from(Cursor::new(corrupted), &registry).unwrap();
    assert!(matches!(
        reader.read_entry_data(0),
        Err(Error::IntegrityCheck { .. })
    ));
}

#[test]
fn size_mismatch_is_reported() {
    let bytes = write_archive(&[("a.txt", &b"hello"[..])], &[]);
    let size_offset = format::HEADER_SIZE + 16;
    let mut corrupted = bytes.clone();
    corrupted[size_offset..size_offset + 8].copy_from_slice(&99u64.to_le_bytes());
    let registry = Registry::new();
    let mut reader = ArchiveReader::read_from(Cursor::new(corrupted), &registry).unwrap();
    assert!(matches!(
        reader.read_entry_data(0),
        Err(Error::SizeMismatch {
            expected: 99,
            actual: 5,
            ..
        })
    ));
}

fn write_huffman_archive(files: &[(&str, &[u8])], dirs: &[&str]) -> Vec<u8> {
    let registry = Registry::new();
    let algorithm = registry.get_by_name("huffman").unwrap();
    let mut buffer = Vec::new();
    {
        let mut writer = ArchiveWriter::new(Cursor::new(&mut buffer), algorithm).unwrap();
        for path in dirs {
            writer.add_dir(path, 0o755, 0).unwrap();
        }
        for (path, data) in files {
            writer.add_file(path, data, 0o644, 0).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer
}

#[test]
fn huffman_archive_round_trips_multiple_entries() {
    let files = [
        ("hello.txt", &b"hello world"[..]),
        ("empty.bin", &b""[..]),
        ("single.bin", &b"zzzzzzzzzz"[..]),
        ("binary.bin", &[0u8, 1, 2, 3, 4, 5, 255, 254][..]),
        ("unicode/nihongo.txt", "こんにちは".as_bytes()),
    ];
    let dirs = ["unicode", "empty-dir"];
    let bytes = write_huffman_archive(&files, &dirs);

    let registry = Registry::new();
    let mut reader = ArchiveReader::read_from(Cursor::new(bytes), &registry).unwrap();
    assert_eq!(reader.format_version(), 1);
    assert_eq!(reader.algorithm_id(), 2);
    assert_eq!(reader.algorithm_name(), "huffman");
    assert_eq!(reader.entry_count(), files.len() + dirs.len());

    for (path, data) in &files {
        let index = reader
            .entries()
            .iter()
            .position(|e| e.path == *path)
            .unwrap_or_else(|| panic!("missing entry '{path}'"));
        assert_eq!(reader.entries()[index].kind, EntryKind::File);
        assert_eq!(reader.entries()[index].uncompressed_size, data.len() as u64);
        assert_eq!(reader.read_entry_data(index).unwrap(), *data);
    }
    for path in &dirs {
        assert!(
            reader
                .entries()
                .iter()
                .any(|e| e.path == *path && e.kind == EntryKind::Directory),
            "missing directory entry '{path}'"
        );
    }
}
