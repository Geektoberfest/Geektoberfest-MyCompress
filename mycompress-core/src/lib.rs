//! # mycompress-core
//!
//! Compression engine and archive format for `MyCompress`.
//!
//! This crate is deliberately dependency-free and contains no UI code.
//! Clients such as `mycompress-cli` (and a future GUI) build on the public
//! API exposed here:
//!
//! - [`algo`]: the [`CompressionAlgorithm`] trait, the [`Registry`], and the
//!   built-in RLE implementation.
//! - [`archive`]: the `.mcz` archive format, reader, and writer.
//! - [`files`]: filesystem helpers that turn files/directories into archives
//!   and archives back into directories (with security validation).
//! - [`paths`]: validation for paths stored inside archives.
//! - [`bench`]: benchmarking utilities for comparing compression algorithms.
//! - [`crc32`]: CRC32 (IEEE) checksums used for entry integrity.
//! - [`error`]: the shared [`Error`] type.
//!
//! ## Example
//!
//! ```no_run
//! use mycompress_core::files::{self, CompressOptions};
//! use mycompress_core::{AlgorithmId, Registry};
//!
//! # fn main() -> mycompress_core::Result<()> {
//! let registry = Registry::new();
//! let options = CompressOptions { algorithm: AlgorithmId::Rle, force: false };
//! files::compress(
//!     std::path::Path::new("docs"),
//!     std::path::Path::new("docs.mcz"),
//!     &options,
//!     &registry,
//! )?;
//! # Ok(())
//! # }
//! ```

pub mod algo;
pub mod archive;
pub mod bench;
pub mod crc32;
pub mod error;
pub mod files;
pub mod paths;

pub use algo::{AlgorithmId, CompressionAlgorithm, Registry};
pub use archive::{ArchiveReader, ArchiveWriter, EntryKind, EntryMetadata};
pub use error::{Error, Result};
