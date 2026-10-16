//! On-disk definition of the `.mcz` archive format, version 1.
//!
//! All integers are little-endian. The full specification (including the
//! security model and compatibility policy) lives in `docs/format.md`.
//!
//! ## Header (16 bytes)
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0 | 4 | magic `MCZ1` |
//! | 4 | 2 | format version (`u16`, currently 1) |
//! | 6 | 2 | compression algorithm id (`u16`) |
//! | 8 | 4 | entry count (`u32`) |
//! | 12 | 4 | reserved, must be zero |
//!
//! ## Entry record (36 fixed bytes + path + data)
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0 | 2 | path length in bytes (`u16`) |
//! | 2 | 1 | kind: 0 = file, 1 = directory |
//! | 3 | 1 | reserved, must be zero |
//! | 4 | 4 | Unix permission bits (`u32`, 0 = unspecified) |
//! | 8 | 8 | mtime, seconds since Unix epoch (`u64`, 0 = unspecified) |
//! | 16 | 8 | uncompressed size (`u64`) |
//! | 24 | 8 | compressed size (`u64`) |
//! | 32 | 4 | CRC32 of uncompressed data (`u32`) |
//! | 36 | path length | path bytes (UTF-8, `/`-separated, relative) |
//! | ... | compressed size | compressed file data (empty for directories) |

/// Magic bytes at the start of every archive: ASCII `"MCZ1"`.
pub const MAGIC: [u8; 4] = *b"MCZ1";

/// The archive format version written by this library.
pub const FORMAT_VERSION: u16 = 1;

/// Size of the fixed archive header in bytes.
pub const HEADER_SIZE: usize = 16;

/// Size of the fixed part of an entry record (path and data excluded).
pub const ENTRY_FIXED_SIZE: usize = 36;

/// Maximum length of a stored path, in bytes.
pub const MAX_PATH_LEN: usize = 4096;

/// Maximum number of entries accepted in a single archive.
pub const MAX_ENTRY_COUNT: u32 = 10_000_000;

/// Largest per-entry uncompressed size accepted when reading (1 TiB).
/// Note: decompression itself is additionally capped at 64 MiB per entry
/// (see `algo::MAX_DECOMPRESSED_BYTES`) so tiny inputs can't force huge allocs.
pub const MAX_ENTRY_UNCOMPRESSED_SIZE: u64 = 1 << 40;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_the_specification() {
        assert_eq!(HEADER_SIZE, 16);
        assert_eq!(ENTRY_FIXED_SIZE, 36);
        assert_eq!(MAGIC, *b"MCZ1");
        assert_eq!(FORMAT_VERSION, 1);
    }
}
