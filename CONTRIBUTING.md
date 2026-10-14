# Contributing

Thanks for helping improve MyCompress. Please open an issue for a bug or
feature discussion before making a large change. Small fixes can be submitted
directly as a pull request.

## Development Setup

Install Rust 1.75 or newer, clone the repository, and run:

```sh
cargo test --workspace
```

The command-line binary can be run without installing it:

```sh
cargo run -p mycompress-cli -- --help
```

## Before Opening A Pull Request

Run the same checks used by continuous integration:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

The test suite includes:
- Unit and integration tests for CLI, archive reader/writer, filesystem, and security.
- Property-based tests via `proptest` (`cargo test -p mycompress-core --test property`).
- Malformed and adversarial input tests (`cargo test -p mycompress-core --test malformed`).
- Bounded fuzz smoke tests (`cargo test -p mycompress-core --test fuzz_smoke`).
- Dedicated `cargo-fuzz` targets in `fuzz/` (`fuzz_archive` and `fuzz_decompress`).

Add or update focused tests when changing archive behavior, filesystem
handling, path validation, or compression algorithms. Do not weaken security
checks to make a test pass.

## Pull Requests

Describe the user-visible behavior, implementation approach, and tests run.
Keep changes focused and avoid committing build output from `target/`.

Changes to the `.mcz` format must update `docs/format.md`, include
compatibility considerations, and add corruption or round-trip coverage.

## Releases

Releases are created by pushing a version tag such as `v0.1.0`. The release
workflow builds Linux x86_64 and Windows x86_64 archives and attaches them to
a GitHub Release. macOS packaging is intentionally not included yet.
