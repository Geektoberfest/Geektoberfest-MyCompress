//! The `.mcz` archive format: entry metadata, reader, and writer.

pub mod format;
pub mod reader;
pub mod writer;

pub use reader::ArchiveReader;
pub use writer::ArchiveWriter;

/// Whether an archive entry describes a file or a directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

impl EntryKind {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::File => 0,
            Self::Directory => 1,
        }
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::File),
            1 => Some(Self::Directory),
            _ => None,
        }
    }
}

/// Metadata for one entry, exactly as stored in the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryMetadata {
    /// Canonical relative path, `/`-separated.
    pub path: String,
    pub kind: EntryKind,
    /// Unix permission bits; 0 means "unspecified".
    pub mode: u32,
    /// Modification time in seconds since the Unix epoch; 0 means "unspecified".
    pub mtime: u64,
    /// Uncompressed size in bytes.
    pub uncompressed_size: u64,
    /// Compressed size in bytes.
    pub compressed_size: u64,
    /// CRC32 of the uncompressed data.
    pub crc32: u32,
}
