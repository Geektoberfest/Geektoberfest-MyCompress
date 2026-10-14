# MyCompress

MyCompress is a cross-platform command-line tool for compressing files and
directories into `.mcz` archives and extracting them again. It is written in
Rust and includes built-in compression algorithms: run-length encoding
(RLE) and canonical Huffman coding.

## Requirements

- Rust 1.75 or newer, including `cargo`
- A filesystem supported by Rust's standard library

No additional runtime or third-party command-line tools are required.

## Clone and Build

Replace `https://github.com/sai-kore/MyCompress.git` with the URL of this repository:

```sh
git clone <repository-url>
cd MyCompress

# Run the complete test suite.
cargo test --workspace

# Build an optimized executable.
cargo build --release -p mycompress-cli
```

The executable is created at `target/release/mycompress` on Linux and macOS,
or `target/release/mycompress.exe` on Windows.

You can also run the CLI directly through Cargo without installing it:

```sh
cargo run --release -p mycompress-cli -- --help
```

For development builds, omit `--release`.

## Command Overview

```text
mycompress <COMMAND> [OPTIONS]
```

Available commands:

| Command | Purpose |
| --- | --- |
| `compress` | Create an `.mcz` archive from a file or directory |
| `extract` | Extract an archive into a directory |
| `list` | List every file and directory in an archive |
| `info` | Show archive format, algorithm, entry counts, sizes, and ratio |

Run `mycompress --help` or `mycompress <command> --help` for the generated
command reference.

## Compress Files and Directories

Compress a file:

```sh
mycompress compress ./report.txt -o ./report.mcz
```

Compress a directory tree:

```sh
mycompress compress ./photos -o ./photos.mcz
```

If `-o` is omitted, `.mcz` is appended to the input path:

```sh
mycompress compress ./photos
# Creates ./photos.mcz
```

The complete syntax is:

```text
mycompress compress <INPUT> [-o|--output <ARCHIVE>] [-a|--algorithm <NAME>] [-f|--force]
```

- `<INPUT>` is a file or directory.
- `-o, --output` selects the destination archive. The default is
	`<INPUT>.mcz`.
- `-a, --algorithm` selects the compression algorithm: `rle` (default) or
	`huffman`.
- `-f, --force` overwrites an existing output archive.

If the output archive is inside the input directory, MyCompress skips that
archive while walking the input. Symbolic links are not archived.

## Inspect an Archive

List archive entries without extracting them:

```sh
mycompress list ./photos.mcz
```

The listing includes each entry's type, uncompressed size, packed size, and
relative path.

Show archive metadata and compression statistics:

```sh
mycompress info ./photos.mcz
```

The information report includes the `.mcz` format version, algorithm name and
ID, file and directory counts, uncompressed bytes, compressed data bytes, and
the compression ratio.

## Extract an Archive

Extract into a new or existing directory:

```sh
mycompress extract ./photos.mcz -d ./restored
```

The complete syntax is:

```text
mycompress extract <ARCHIVE> [-d|--dest <DIRECTORY>] [-f|--force]
```

- `<ARCHIVE>` is an `.mcz` archive.
- `-d, --dest` selects the destination directory. The default is the current
	directory (`.`); the directory is created when necessary.
- `-f, --force` overwrites existing files during extraction.

Existing files are never overwritten by default. Existing directories may be
reused. Every file is decompressed and checked for its expected size and CRC32
before it is written.

## Archive and Compression Details

- Archives use the version 1 `.mcz` format with the `MCZ1` signature.
- RLE is most useful for data containing repeated bytes. Huffman coding
	provides general-purpose compression for data with non-uniform byte distributions.
	Neither algorithm is well suited for already-compressed or random data.
- Files, directories, relative paths, file sizes, modification times, and
	permissions are stored in the archive where supported by the platform.
- Archive paths are validated to prevent absolute paths, parent-directory
	traversal, unsafe Windows device names, and extraction outside the selected
	destination.
- Corrupt archives, unsupported algorithms, duplicate paths, and file/directory
	conflicts are rejected.

## Examples

```sh
# Build and run without installing the binary.
cargo run --release -p mycompress-cli -- compress ./documents

# Use an explicit output name.
mycompress compress ./documents --output ./backup.mcz

# Inspect the archive before extracting it.
mycompress info ./backup.mcz
mycompress list ./backup.mcz

# Restore the archive, allowing replacement of existing files.
mycompress extract ./backup.mcz --dest ./restored --force
```

## Development and Testing

Run the complete automated test suite (unit tests, integration tests, property-based tests, malformed-input tests, and fuzz smoke tests):

```sh
cargo test --workspace
```

### Property-Based Testing

Property-based tests use [`proptest`](https://crates.io/crates/proptest) to verify that `decompress(compress(data)) == data` holds across all supported algorithms (`rle` and `huffman`) and full in-memory `.mcz` archives.

To run only property-based tests:

```sh
cargo test -p mycompress-core --test property
```

The property test suite generates bounded inputs across:
- Empty and single-byte sequences
- Long repeated byte runs (up to 20,000 bytes)
- Alternating two-byte patterns
- Small repeating chunks (2..8 bytes repeated)
- Shuffled sequences covering all 256 byte values
- Format and algorithm boundary lengths (0, 1, 2, 3, 127..131, 255..257, 4096, 8192)
- Multi-file archives with complex nested paths

Failed test cases are automatically minimized and persisted to `proptest-regressions/` for deterministic reproduction.

### Malformed and Adversarial Archive Tests

Adversarial tests ensure corrupt archives, hostile size metadata, invalid paths, and broken compression streams are rejected with controlled `Error` variants rather than panics or excessive allocations:

```sh
cargo test -p mycompress-core --test malformed
```

Scenarios covered include:
- Empty inputs and truncated headers at every single byte offset (1..15)
- Bad magic bytes and unsupported format versions/algorithm IDs
- Non-zero reserved fields in archive headers and entry records
- Truncated entries, truncated paths, and truncated compressed streams
- Size mismatch attacks, integer overflow attacks, and sizes exceeding limits (> 1 TiB)
- Directory entries carrying unexpected file data or non-zero checksums
- Path traversal attacks (`..`, `a/../../b`), absolute paths, backslashes, NUL bytes, Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`), and overlong components
- Corrupted CRC32 checksums and broken RLE/Huffman bitstreams

### Fuzz Testing

1. **Fast Bounded Smoke Test** (runs on stable Rust without additional tools):

   ```sh
   cargo test -p mycompress-core --test fuzz_smoke
   ```

   Exercises `ArchiveReader::read_from`, `Rle::decompress`, and `Huffman::decompress` against thousands of random, truncated, bit-flipped, and spliced byte sequences in under a second.

2. **Dedicated Continuous Fuzzing (`cargo-fuzz`)**:

   For long-running fuzzing campaigns with libFuzzer:

   ```sh
   # Install cargo-fuzz (requires nightly Rust)
   cargo install cargo-fuzz

   # Run archive parser fuzzer
   cargo fuzz run fuzz_archive

   # Run raw decompression fuzzer
   cargo fuzz run fuzz_decompress
   ```

   To reproduce a saved crash artifact found by `cargo-fuzz`:
   ```sh
   cargo fuzz run fuzz_archive <path/to/crash-artifact>
   ```

Useful documentation:

- [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidance and CI checks
- [docs/architecture.md](docs/architecture.md) for the project structure
- [docs/format.md](docs/format.md) for the `.mcz` file format
- [docs/algorithms.md](docs/algorithms.md) for compression algorithms

## License

MyCompress is distributed under the MIT License. See [LICENSE](LICENSE).