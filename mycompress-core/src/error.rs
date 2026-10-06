//! The shared error type for the MyCompress core library.

use std::fmt;
use std::path::PathBuf;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by the MyCompress core library.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An underlying I/O operation failed.
    Io(std::io::Error),
    /// A filesystem operation on a specific path failed.
    FileOp {
        op: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    /// The caller supplied an invalid input (missing file, bad option...).
    InvalidInput(String),
    /// The file does not start with the `.mcz` magic bytes.
    BadMagic([u8; 4]),
    /// The archive format version is not supported by this build.
    UnsupportedVersion { found: u16, supported: u16 },
    /// The archive uses an algorithm that is not in the registry.
    UnknownAlgorithm(u16),
    /// The archive ended in the middle of a structure.
    TruncatedArchive(String),
    /// The archive structure is internally inconsistent.
    CorruptArchive(String),
    /// Entry data failed its CRC32 integrity check.
    IntegrityCheck {
        path: String,
        expected: u32,
        actual: u32,
    },
    /// Decompressed data does not match the size recorded in the archive.
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    /// A stored path is malformed.
    InvalidArchivePath(String),
    /// A stored path attempts to escape the extraction directory.
    PathTraversal(String),
    /// Two entries in the archive share the same path.
    DuplicatePath(String),
    /// A path is used both as a file and as a directory prefix.
    ConflictingPath(String),
    /// An entry declares an implausible size.
    UnreasonableSize { path: String, size: u64 },
    /// An output file already exists and would be overwritten.
    OutputExists(PathBuf),
    /// A filesystem entry type that cannot be archived (symlink, socket...).
    UnsupportedEntry(PathBuf),
    /// Attempted to register a custom algorithm with a reserved id.
    ReservedAlgorithmId(u16),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::FileOp { op, path, source } => {
                write!(f, "failed to {op} '{}': {source}", path.display())
            }
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::BadMagic(magic) => write!(f, "not a MyCompress archive (bad magic bytes {magic:x?})"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "unsupported archive format version {found} (this build supports version {supported})"
            ),
            Self::UnknownAlgorithm(id) => write!(
                f,
                "archive uses unsupported compression algorithm id {id} \
                 (it may have been created by a newer version)"
            ),
            Self::TruncatedArchive(msg) => write!(f, "archive is truncated: {msg}"),
            Self::CorruptArchive(msg) => write!(f, "archive is corrupt: {msg}"),
            Self::IntegrityCheck { path, expected, actual } => write!(
                f,
                "integrity check failed for '{path}': expected CRC32 {expected:#010x}, got {actual:#010x}"
            ),
            Self::SizeMismatch { path, expected, actual } => write!(
                f,
                "decompressed size mismatch for '{path}': header says {expected} bytes, got {actual}"
            ),
            Self::InvalidArchivePath(msg) => write!(f, "invalid path in archive: {msg}"),
            Self::PathTraversal(path) => {
                write!(f, "path traversal attempt blocked: '{path}'")
            }
            Self::DuplicatePath(path) => write!(f, "duplicate path in archive: '{path}'"),
            Self::ConflictingPath(msg) => write!(f, "conflicting paths in archive: {msg}"),
            Self::UnreasonableSize { path, size } => {
                write!(f, "entry '{path}' declares an unreasonable size ({size} bytes)")
            }
            Self::OutputExists(path) => write!(
                f,
                "'{}' already exists (use --force to overwrite)",
                path.display()
            ),
            Self::UnsupportedEntry(path) => write!(
                f,
                "unsupported filesystem entry: '{}' (symbolic links and special \
                 files cannot be archived)",
                path.display()
            ),
            Self::ReservedAlgorithmId(id) => write!(
                f,
                "algorithm id {id} is reserved for built-in algorithms; \
                 custom algorithms must use ids >= 1024"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) | Self::FileOp { source: e, .. } => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
