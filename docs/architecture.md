# Architecture

MyCompress is a Cargo workspace with two packages:

```text
mycompress-core   reusable archive, compression, filesystem, and security code
mycompress-cli    clap-based command-line application
```

## Data Flow

Compression walks the input tree, skips unsupported entries such as symbolic
links, validates archive paths, compresses file bytes, and writes sorted
entries to an `.mcz` archive. The output archive is created before the walk so
the compressor can exclude the archive itself when it is inside the input
directory.

Extraction opens and validates the archive, checks every path against the
destination, decompresses and verifies file data, then creates directories and
files. Files are created without overwrite by default; `--force` enables
replacement.

## Modules

- `archive`: versioned format, reader, writer, entry metadata, and layout rules.
- `algo`: compression algorithm trait, registry, and RLE implementation.
- `bench`: benchmarking runner and duration calculations for registered algorithms.
- `files`: filesystem walking, compression, and secure extraction.
- `paths`: archive path parsing and traversal protection.
- `crc32`: dependency-free CRC32 implementation for integrity checks.
- `error`: shared error type used by the library and CLI.

The core crate intentionally has no third-party dependencies. The CLI uses
Clap for argument parsing and delegates all archive behavior to the core
crate.

## Error and Security Boundaries

Archive structure is validated when opening an archive. File contents are
validated when read, including decompressed size and CRC32. Extraction uses
validated relative paths and refuses to overwrite existing files unless the
caller explicitly requests `--force`.

## Testing

Run the complete workspace suite from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

Integration tests cover archive round trips, filesystem behavior, CLI flows,
corruption handling, and path traversal attacks.
