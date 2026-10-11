//! End-to-end tests that run the compiled `mycompress` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mycompress-cli-test-{}-{}-{}",
            label,
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&path).expect("failed to create temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn mycompress(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mycompress"))
        .args(args)
        .output()
        .expect("failed to run the mycompress binary")
}

fn assert_success(output: &Output, context: &str) {
    if !output.status.success() {
        panic!(
            "{context} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn compress_extract_list_info_round_trip() {
    let dir = TempDir::new("roundtrip");
    let input = dir.path().join("input");
    std::fs::create_dir_all(input.join("docs")).unwrap();
    std::fs::write(input.join("docs/format.md"), b"# Format\n").unwrap();
    std::fs::write(input.join("readme.txt"), b"MyCompress\n").unwrap();
    let archive = dir.path().join("project.mcz");
    let out = dir.path().join("out");

    let output = mycompress(&[
        "compress",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
    ]);
    assert_success(&output, "compress");
    assert!(archive.exists());

    let output = mycompress(&[
        "extract",
        archive.to_str().unwrap(),
        "-d",
        out.to_str().unwrap(),
    ]);
    assert_success(&output, "extract");
    assert_eq!(
        std::fs::read(out.join("readme.txt")).unwrap(),
        b"MyCompress\n"
    );
    assert_eq!(
        std::fs::read(out.join("docs/format.md")).unwrap(),
        b"# Format\n"
    );

    let output = mycompress(&["list", archive.to_str().unwrap()]);
    assert_success(&output, "list");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("readme.txt"), "list output: {stdout}");
    assert!(stdout.contains("docs/format.md"), "list output: {stdout}");
    assert!(stdout.contains("docs"), "list output: {stdout}");

    let output = mycompress(&["info", archive.to_str().unwrap()]);
    assert_success(&output, "info");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rle"), "info output: {stdout}");
    assert!(stdout.contains("version 1"), "info output: {stdout}");
}

#[test]
fn compress_refuses_to_overwrite_without_force() {
    let dir = TempDir::new("overwrite");
    let input = dir.path().join("input");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.txt"), b"first").unwrap();
    let archive = dir.path().join("a.mcz");

    let args = [
        "compress",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
    ];
    assert_success(&mycompress(&args), "first compress");

    let output = mycompress(&args);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"), "stderr: {stderr}");

    // The original archive is still intact and readable.
    assert_success(
        &mycompress(&["list", archive.to_str().unwrap()]),
        "list after refusal",
    );

    let mut forced = args.to_vec();
    forced.push("--force");
    assert_success(&mycompress(&forced), "forced compress");
}

#[test]
fn extract_refuses_to_overwrite_without_force() {
    let dir = TempDir::new("extract-overwrite");
    let input = dir.path().join("input");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.txt"), b"first").unwrap();
    let archive = dir.path().join("a.mcz");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("a.txt"), b"do not touch").unwrap();

    assert_success(
        &mycompress(&[
            "compress",
            input.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ]),
        "compress",
    );
    let output = mycompress(&[
        "extract",
        archive.to_str().unwrap(),
        "-d",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"), "stderr: {stderr}");
    assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"do not touch");
}

#[test]
fn meaningful_errors_for_bad_input() {
    let dir = TempDir::new("errors");

    // Missing input file.
    let missing = dir.path().join("missing");
    let output = mycompress(&["compress", missing.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot read input"), "stderr: {stderr}");

    // Unknown algorithm.
    let any_input = dir.path().join("x");
    let output = mycompress(&["compress", any_input.to_str().unwrap(), "-a", "bogus"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown algorithm"), "stderr: {stderr}");

    // Not an archive at all.
    let fake = dir.path().join("fake.mcz");
    std::fs::write(&fake, b"this is not an archive").unwrap();
    let output = mycompress(&["list", fake.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("magic"),
        "expected a magic-bytes error: {stderr}"
    );
}

#[test]
fn help_and_version_work() {
    assert_success(&mycompress(&["--version"]), "--version");
    assert_success(&mycompress(&["--help"]), "--help");
    assert_success(&mycompress(&["compress", "--help"]), "compress --help");
}

#[test]
fn huffman_compress_extract_list_info_round_trip() {
    let dir = TempDir::new("huffman-roundtrip");
    let input = dir.path().join("input");
    std::fs::create_dir_all(input.join("docs")).unwrap();
    std::fs::write(input.join("docs/format.md"), b"# Format\nHuffman coding\n").unwrap();
    std::fs::write(input.join("readme.txt"), b"MyCompress Huffman Test\n").unwrap();
    let archive = dir.path().join("project.mcz");
    let out = dir.path().join("out");

    let output = mycompress(&[
        "compress",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        "-a",
        "huffman",
    ]);
    assert_success(&output, "compress with huffman");
    assert!(archive.exists());

    let output = mycompress(&["info", archive.to_str().unwrap()]);
    assert_success(&output, "info");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("huffman"), "info output: {stdout}");
    assert!(stdout.contains("id 2"), "info output: {stdout}");
    assert!(stdout.contains("version 1"), "info output: {stdout}");

    let output = mycompress(&["list", archive.to_str().unwrap()]);
    assert_success(&output, "list");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("readme.txt"), "list output: {stdout}");
    assert!(stdout.contains("docs/format.md"), "list output: {stdout}");

    let output = mycompress(&[
        "extract",
        archive.to_str().unwrap(),
        "-d",
        out.to_str().unwrap(),
    ]);
    assert_success(&output, "extract");
    assert_eq!(
        std::fs::read(out.join("readme.txt")).unwrap(),
        b"MyCompress Huffman Test\n"
    );
    assert_eq!(
        std::fs::read(out.join("docs/format.md")).unwrap(),
        b"# Format\nHuffman coding\n"
    );
}

#[test]
fn auto_compress_text_selects_huffman_and_extracts() {
    let dir = TempDir::new("cli-auto-text");
    let input = dir.path().join("input");
    std::fs::create_dir_all(&input).unwrap();

    let sample = b"The quick brown fox jumps over the lazy dog. Canonical Huffman coding is effective for text with varied frequencies. ";
    let mut text = Vec::new();
    while text.len() < 2_000 {
        text.extend_from_slice(sample);
    }
    std::fs::write(input.join("essay.txt"), &text).unwrap();

    let archive = dir.path().join("auto.mcz");
    let out = dir.path().join("out");

    let output = mycompress(&[
        "compress",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        "-a",
        "auto",
    ]);
    assert_success(&output, "compress with -a auto");

    let output = mycompress(&["info", archive.to_str().unwrap()]);
    assert_success(&output, "info on auto archive");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("huffman"),
        "expected huffman in info: {stdout}"
    );
    assert!(stdout.contains("id 2"), "expected id 2 in info: {stdout}");

    let output = mycompress(&[
        "extract",
        archive.to_str().unwrap(),
        "-d",
        out.to_str().unwrap(),
    ]);
    assert_success(&output, "extract auto archive");
    assert_eq!(std::fs::read(out.join("essay.txt")).unwrap(), text);
}

#[test]
fn explicit_algorithms_override_auto() {
    let dir = TempDir::new("cli-explicit");
    let input = dir.path().join("file.txt");
    std::fs::write(
        &input,
        b"some simple content for testing explicit algorithm flags",
    )
    .unwrap();

    let archive_rle = dir.path().join("rle.mcz");
    let output = mycompress(&[
        "compress",
        input.to_str().unwrap(),
        "-o",
        archive_rle.to_str().unwrap(),
        "--algorithm",
        "rle",
    ]);
    assert_success(&output, "explicit rle");
    let info_rle = mycompress(&["info", archive_rle.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&info_rle.stdout).contains("rle"));

    let archive_huffman = dir.path().join("huffman.mcz");
    let output = mycompress(&[
        "compress",
        input.to_str().unwrap(),
        "-o",
        archive_huffman.to_str().unwrap(),
        "--algorithm",
        "huffman",
    ]);
    assert_success(&output, "explicit huffman");
    let info_huff = mycompress(&["info", archive_huffman.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&info_huff.stdout).contains("huffman"));
}
