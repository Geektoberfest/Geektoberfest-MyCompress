//! Filesystem integration: walking trees, building archives, and extracting
//! them safely.

mod compress;
mod extract;
mod walk;

pub use compress::{compress, compress_with_progress, CompressOptions, CompressStats};
pub use extract::{
    extract, extract_from_reader, extract_from_reader_with_progress, extract_with_progress,
    ExtractOptions, ExtractStats,
};
pub use walk::{collect_entries, SourceEntry};

use std::path::Path;

use crate::error::Error;

/// Attaches a filesystem path to an I/O error for better messages.
pub(crate) fn file_op(op: &'static str, path: &Path, error: std::io::Error) -> Error {
    Error::FileOp {
        op,
        path: path.to_path_buf(),
        source: error,
    }
}
