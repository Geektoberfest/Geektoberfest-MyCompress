//! End-to-end tests over the real filesystem: compress a tree, extract it,
//! compare it.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use mycompress_core::algo::AlgorithmId;
use mycompress_core::files::{self, CompressOptions, ExtractOptions};
use mycompress_core::{ArchiveReader, Error, Registry};

fn options() -> CompressOptions {
    CompressOptions::explicit(AlgorithmId::Rle, false)
}

fn sample_tree(root: &Path) {
    fs::create_dir_all(root.join("docs/nested")).unwrap();
    fs::write(root.join("readme.md"), b"MyCompress sample\n").unwrap();
    fs::write(root.join("docs/format.md"), b"# Format\nmagic: MCZ1\n").unwrap();
    fs::write(root.join("docs/nested/deep.bin"), [7u8; 1000]).unwrap();
    fs::create_dir(root.join("empty-dir")).unwrap();
}

fn collect_tree(root: &Path, prefix: &str, out: &mut Vec<String>) {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let path = root.join(&name);
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            collect_tree(&path, &rel, out);
        } else {
            out.push(rel);
        }
    }
}

#[test]
fn directory_round_trip_preserves_files_and_structure() {
    let dir = common::TempDir::new("dir-roundtrip");
    let input = dir.path().join("input");
    let archive = dir.path().join("archive.mcz");
    let output = dir.path().join("out");
    fs::create_dir_all(&input).unwrap();
    sample_tree(&input);

    files::compress(&input, &archive, &options(), &Registry::new()).unwrap();
    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();

    let mut expected = Vec::new();
    collect_tree(&input, "", &mut expected);
    let mut actual = Vec::new();
    collect_tree(&output, "", &mut actual);
    assert_eq!(expected, actual);
    for rel in &expected {
        assert_eq!(
            fs::read(input.join(rel)).unwrap(),
            fs::read(output.join(rel)).unwrap(),
            "content mismatch: {rel}"
        );
    }
    assert!(
        output.join("empty-dir").is_dir(),
        "empty directories must be preserved"
    );
}

#[test]
fn single_file_round_trip() {
    let dir = common::TempDir::new("file-roundtrip");
    let input = dir.path().join("notes.txt");
    let archive = dir.path().join("notes.mcz");
    let output = dir.path().join("out");
    fs::write(&input, b"some notes\n").unwrap();

    files::compress(&input, &archive, &options(), &Registry::new()).unwrap();
    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("notes.txt")).unwrap(), b"some notes\n");
}

#[test]
fn compression_reports_stats() {
    let dir = common::TempDir::new("stats");
    let input = dir.path().join("input");
    fs::create_dir_all(&input).unwrap();
    sample_tree(&input);

    let stats = files::compress(
        &input,
        &dir.path().join("a.mcz"),
        &options(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(stats.file_count, 3);
    assert_eq!(stats.dir_count, 3); // docs, docs/nested, empty-dir
    let expected: u64 = ["readme.md", "docs/format.md", "docs/nested/deep.bin"]
        .iter()
        .map(|p| fs::read(input.join(p)).unwrap().len() as u64)
        .sum();
    assert_eq!(stats.total_uncompressed, expected);
    assert!(stats.archive_size > 0);
    assert!(stats.warnings.is_empty());
}

#[test]
fn never_overwrites_existing_files() {
    let dir = common::TempDir::new("no-overwrite");
    let input = dir.path().join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("a.txt"), b"first").unwrap();
    let archive = dir.path().join("a.mcz");
    let out = dir.path().join("out");

    files::compress(&input, &archive, &options(), &Registry::new()).unwrap();
    files::extract(&archive, &out, &ExtractOptions::default(), &Registry::new()).unwrap();

    // Extracting again must refuse to overwrite, and leave content untouched.
    let err =
        files::extract(&archive, &out, &ExtractOptions::default(), &Registry::new()).unwrap_err();
    assert!(matches!(err, Error::OutputExists(_)));
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"first");

    // Compressing onto an existing archive must refuse too.
    let err = files::compress(&input, &archive, &options(), &Registry::new()).unwrap_err();
    assert!(matches!(err, Error::OutputExists(_)));

    // Explicit force is allowed.
    files::extract(
        &archive,
        &out,
        &ExtractOptions { force: true },
        &Registry::new(),
    )
    .unwrap();
    files::compress(
        &input,
        &archive,
        &CompressOptions::explicit(AlgorithmId::Rle, true),
        &Registry::new(),
    )
    .unwrap();
}

#[test]
fn archive_does_not_contain_itself() {
    let dir = common::TempDir::new("self");
    let input = dir.path().join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("data.txt"), b"payload").unwrap();
    let archive = input.join("self.mcz"); // output inside the input directory

    files::compress(&input, &archive, &options(), &Registry::new()).unwrap();
    let registry = Registry::new();
    let reader = ArchiveReader::open(&archive, &registry).unwrap();
    let paths: Vec<&str> = reader.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["data.txt"]);
}

#[test]
fn missing_input_is_reported() {
    let dir = common::TempDir::new("missing");
    let err = files::compress(
        &dir.path().join("nope"),
        &dir.path().join("x.mcz"),
        &options(),
        &Registry::new(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
}

#[test]
#[cfg(unix)]
fn symlinks_are_skipped_with_warnings() {
    let dir = common::TempDir::new("symlinks");
    let input = dir.path().join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("real.txt"), b"data").unwrap();
    std::os::unix::fs::symlink("real.txt", input.join("link.txt")).unwrap();

    let stats = files::compress(
        &input,
        &dir.path().join("a.mcz"),
        &options(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(stats.file_count, 1);
    assert!(stats.warnings.iter().any(|w| w.contains("link.txt")));
}

#[test]
#[cfg(unix)]
fn permissions_are_preserved() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::TempDir::new("perms");
    let input = dir.path().join("input");
    fs::create_dir_all(&input).unwrap();
    let secret = input.join("secret.txt");
    fs::write(&secret, b"top secret").unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();

    let archive = dir.path().join("a.mcz");
    let out = dir.path().join("out");
    files::compress(&input, &archive, &options(), &Registry::new()).unwrap();
    files::extract(&archive, &out, &ExtractOptions::default(), &Registry::new()).unwrap();

    let mode = fs::metadata(out.join("secret.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn modification_times_are_preserved() {
    let dir = common::TempDir::new("mtime");
    let input = dir.path().join("input");
    fs::create_dir_all(&input).unwrap();
    let file = input.join("timed.txt");
    fs::write(&file, b"timeless").unwrap();
    let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(stamp)
        .unwrap();

    let archive = dir.path().join("a.mcz");
    let out = dir.path().join("out");
    files::compress(&input, &archive, &options(), &Registry::new()).unwrap();
    files::extract(&archive, &out, &ExtractOptions::default(), &Registry::new()).unwrap();

    let restored = fs::metadata(out.join("timed.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        restored
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        1_600_000_000
    );
}

#[test]
fn huffman_directory_round_trip() {
    let dir = common::TempDir::new("huffman-dir-roundtrip");
    let input = dir.path().join("input");
    let archive = dir.path().join("archive.mcz");
    let output = dir.path().join("out");
    fs::create_dir_all(&input).unwrap();
    sample_tree(&input);

    let opts = CompressOptions::explicit(AlgorithmId::Huffman, false);
    files::compress(&input, &archive, &opts, &Registry::new()).unwrap();

    let reader = ArchiveReader::open(&archive, &Registry::new()).unwrap();
    assert_eq!(reader.algorithm_id(), 2);
    assert_eq!(reader.algorithm_name(), "huffman");

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();

    let mut original = Vec::new();
    let mut restored = Vec::new();
    collect_tree(&input, "", &mut original);
    collect_tree(&output, "", &mut restored);
    assert_eq!(original, restored);

    for rel in &original {
        let src = fs::read(input.join(rel)).unwrap();
        let dst = fs::read(output.join(rel)).unwrap();
        assert_eq!(src, dst, "content mismatch for '{rel}'");
    }
}

fn lcg_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        out.push((state >> 33) as u8);
    }
    out
}

#[test]
fn auto_single_file_repetitive_round_trip() {
    let dir = common::TempDir::new("auto-repetitive");
    let input = dir.path().join("runs.bin");
    let archive = dir.path().join("runs.mcz");
    let output = dir.path().join("out");

    let mut repetitive = Vec::new();
    for _ in 0..50 {
        repetitive.extend_from_slice(&[b'A'; 100]);
        repetitive.extend_from_slice(&[b'B'; 100]);
    }
    fs::write(&input, &repetitive).unwrap();

    let stats = files::compress(
        &input,
        &archive,
        &CompressOptions::auto(false),
        &Registry::new(),
    )
    .unwrap();
    assert!(matches!(
        stats.algorithm,
        AlgorithmId::Rle | AlgorithmId::Huffman
    ));

    let reader = ArchiveReader::open(&archive, &Registry::new()).unwrap();
    // Archive header must store concrete id 1 or 2
    assert!(reader.algorithm_id() == 1 || reader.algorithm_id() == 2);

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("runs.bin")).unwrap(), repetitive);
}

#[test]
fn auto_single_file_text_selects_huffman_and_round_trips() {
    let dir = common::TempDir::new("auto-text");
    let input = dir.path().join("document.txt");
    let archive = dir.path().join("document.mcz");
    let output = dir.path().join("out");

    let sample = b"The quick brown fox jumps over the lazy dog. Huffman coding generates prefix codes suited for skewed byte distributions. ";
    let mut text = Vec::new();
    while text.len() < 2_000 {
        text.extend_from_slice(sample);
    }
    fs::write(&input, &text).unwrap();

    let stats = files::compress(
        &input,
        &archive,
        &CompressOptions::auto(false),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(stats.algorithm, AlgorithmId::Huffman);

    let reader = ArchiveReader::open(&archive, &Registry::new()).unwrap();
    assert_eq!(reader.algorithm_id(), 2);
    assert_eq!(reader.algorithm_name(), "huffman");

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("document.txt")).unwrap(), text);
}

#[test]
fn auto_single_file_empty_falls_back_to_rle_and_round_trips() {
    let dir = common::TempDir::new("auto-empty");
    let input = dir.path().join("empty.txt");
    let archive = dir.path().join("empty.mcz");
    let output = dir.path().join("out");
    fs::write(&input, b"").unwrap();

    let stats = files::compress(
        &input,
        &archive,
        &CompressOptions::auto(false),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(stats.algorithm, AlgorithmId::Rle);

    let reader = ArchiveReader::open(&archive, &Registry::new()).unwrap();
    assert_eq!(reader.algorithm_id(), 1);
    assert_eq!(reader.algorithm_name(), "rle");

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("empty.txt")).unwrap(), b"");
}

#[test]
fn auto_single_file_incompressible_falls_back_to_rle() {
    let dir = common::TempDir::new("auto-incompressible");
    let input = dir.path().join("random.bin");
    let archive = dir.path().join("random.mcz");
    let output = dir.path().join("out");

    let rand_data = lcg_bytes(999, 10_000);
    fs::write(&input, &rand_data).unwrap();

    let stats = files::compress(
        &input,
        &archive,
        &CompressOptions::auto(false),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(stats.algorithm, AlgorithmId::Rle);

    let reader = ArchiveReader::open(&archive, &Registry::new()).unwrap();
    assert_eq!(reader.algorithm_id(), 1);

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("random.bin")).unwrap(), rand_data);
}

#[test]
fn auto_directory_round_trip_preserves_files_and_structure() {
    let dir = common::TempDir::new("auto-dir-roundtrip");
    let input = dir.path().join("input");
    let archive = dir.path().join("archive.mcz");
    let output = dir.path().join("out");
    fs::create_dir_all(&input).unwrap();
    sample_tree(&input);

    let stats = files::compress(
        &input,
        &archive,
        &CompressOptions::auto(false),
        &Registry::new(),
    )
    .unwrap();
    assert!(matches!(
        stats.algorithm,
        AlgorithmId::Rle | AlgorithmId::Huffman
    ));

    let reader = ArchiveReader::open(&archive, &Registry::new()).unwrap();
    assert!(reader.algorithm_id() == 1 || reader.algorithm_id() == 2);

    files::extract(
        &archive,
        &output,
        &ExtractOptions::default(),
        &Registry::new(),
    )
    .unwrap();

    let mut expected = Vec::new();
    collect_tree(&input, "", &mut expected);
    let mut actual = Vec::new();
    collect_tree(&output, "", &mut actual);
    assert_eq!(expected, actual);
    for rel in &expected {
        assert_eq!(
            fs::read(input.join(rel)).unwrap(),
            fs::read(output.join(rel)).unwrap(),
            "content mismatch: {rel}"
        );
    }
}
