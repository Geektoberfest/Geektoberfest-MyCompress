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
| `benchmark` | Compare registered compression algorithms on input files |

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

## Benchmark Algorithms

Compare compression algorithms on one or more files:

```sh
mycompress benchmark ./report.txt
```

Compare across multiple files with a custom iteration count:

```sh
mycompress benchmark ./report.txt ./photo.raw -n 10
```

The complete syntax is:

```text
mycompress benchmark <FILE>... [-n|--iterations <N>]
```

- `<FILE>...` is one or more input files. Directories are unsupported.
- `-n, --iterations` selects the number of runs per algorithm (default: `5`).
	Must be greater than `0`.

Benchmarking reads each file fully into memory, tests compression and
decompression across all registered algorithms, and verifies that decompressed
data exactly matches the original input. It only reads inputs and writes
nothing; no output files are created.

How to read the output:
- **Ratio**: output size divided by input size; lower means smaller compressed output.
- **Timings**: compression and decompression times are medians across all iterations on this machine.
- Timings depend on hardware and data characteristics. Results are only indicative when running few iterations (`< 5`) or benchmarking small files (`< 64 KiB`).
- Algorithms are listed alphabetically from the registry; they are not ranked or presented as "best" or "fastest".

For realistic timings, run benchmarks using an optimized release build:

```sh
cargo run --release -p mycompress-cli -- benchmark ./report.txt
```

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

## Development

Run all tests after making changes:

```sh
cargo test --workspace
```

Useful documentation:

- [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidance
- [docs/architecture.md](docs/architecture.md) for the project structure
- [docs/format.md](docs/format.md) for the `.mcz` file format
- [docs/algorithms.md](docs/algorithms.md) for compression algorithms

## License

MyCompress is distributed under the MIT License. See [LICENSE](LICENSE).