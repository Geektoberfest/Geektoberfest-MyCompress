use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use crate::algo::{CompressionAlgorithm, Registry};
use crate::archive::format::{
    ENTRY_FIXED_SIZE, FORMAT_VERSION, HEADER_SIZE, MAGIC, MAX_ENTRY_COUNT,
    MAX_ENTRY_UNCOMPRESSED_SIZE, MAX_PATH_LEN,
};
use crate::archive::{EntryKind, EntryMetadata};
use crate::crc32::crc32;
use crate::error::{Error, Result};
use crate::paths;

/// Private alias for "any readable and seekable stream".
trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// Reads and validates `.mcz` archives.
///
/// All structural and security validation happens while the archive is
/// opened: by the time a reader exists, its paths are known to be safe
/// relative paths with no duplicates and no file/directory conflicts, and
/// all declared sizes are plausible. Entry data is read, decompressed, and
/// verified (size + CRC32) on demand via [`ArchiveReader::read_entry_data`],
/// so `list` and `info` never decompress file contents.
///
/// Entries appear in archive order; MyCompress writes them sorted by path,
/// which places parents before children.
pub struct ArchiveReader {
    reader: BufReader<Box<dyn ReadSeek>>,
    algorithm: Arc<dyn CompressionAlgorithm>,
    version: u16,
    algorithm_id: u16,
    entries: Vec<EntryMetadata>,
    data_offsets: Vec<u64>,
}

impl ArchiveReader {
    /// Opens and validates an archive from disk.
    pub fn open(path: &Path, registry: &Registry) -> Result<Self> {
        let file = File::open(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                Error::InvalidInput(format!("archive '{}' not found", path.display()))
            }
            _ => Error::Io(e),
        })?;
        Self::read_from(file, registry)
    }

    /// Reads and validates an archive from any seekable stream. Used by tools
    /// and tests that already have the bytes in memory.
    pub fn read_from(reader: impl Read + Seek + 'static, registry: &Registry) -> Result<Self> {
        let mut reader = BufReader::new(Box::new(reader) as Box<dyn ReadSeek>);

        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let mut header = [0u8; HEADER_SIZE];
        read_bytes(&mut reader, &mut header)?;
        if header[..4] != MAGIC {
            return Err(Error::BadMagic([
                header[0], header[1], header[2], header[3],
            ]));
        }
        let version = le_u16(&header[4..6]);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion {
                found: version,
                supported: FORMAT_VERSION,
            });
        }
        let algorithm_id = le_u16(&header[6..8]);
        let entry_count = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let reserved = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        if reserved != 0 {
            return Err(Error::CorruptArchive(
                "reserved header field is not zero".to_string(),
            ));
        }
        if entry_count > MAX_ENTRY_COUNT {
            return Err(Error::CorruptArchive(format!(
                "entry count {entry_count} exceeds the limit of {MAX_ENTRY_COUNT}"
            )));
        }
        let Some(algorithm) = registry.get(algorithm_id) else {
            return Err(Error::UnknownAlgorithm(algorithm_id));
        };
        // Every entry needs at least its fixed record plus one path byte.
        let min_entry_bytes = (ENTRY_FIXED_SIZE + 1) as u64;
        if u64::from(entry_count) > file_len.saturating_sub(HEADER_SIZE as u64) / min_entry_bytes {
            return Err(Error::CorruptArchive(
                "entry count exceeds the archive size".to_string(),
            ));
        }

        let mut entries = Vec::new();
        let mut data_offsets = Vec::new();
        let mut offset = HEADER_SIZE as u64;

        for _ in 0..entry_count {
            let mut record = [0u8; ENTRY_FIXED_SIZE];
            read_bytes(&mut reader, &mut record)?;
            let path_len = u16::from_le_bytes([record[0], record[1]]) as usize;
            let kind = EntryKind::from_u8(record[2]).ok_or_else(|| {
                Error::CorruptArchive(format!("invalid entry kind {}", record[2]))
            })?;
            if record[3] != 0 {
                return Err(Error::CorruptArchive(
                    "reserved entry field is not zero".to_string(),
                ));
            }
            let mode = le_u32(&record[4..8]);
            let mtime = le_u64(&record[8..16]);
            let uncompressed_size = le_u64(&record[16..24]);
            let compressed_size = le_u64(&record[24..32]);
            let crc = le_u32(&record[32..36]);

            if path_len == 0 || path_len > MAX_PATH_LEN {
                return Err(Error::CorruptArchive(format!(
                    "invalid path length {path_len}"
                )));
            }

            let mut path_bytes = vec![0u8; path_len];
            read_bytes(&mut reader, &mut path_bytes)?;
            let path = String::from_utf8(path_bytes)
                .map_err(|_| Error::CorruptArchive("path is not valid UTF-8".to_string()))?;
            // Security validation: rejects absolute paths, '..', drive
            // letters, reserved device names, and more.
            paths::parse_archive_path(&path)?;

            if mode > 0o777 {
                return Err(Error::CorruptArchive(format!(
                    "entry '{path}' has invalid mode {mode:o}"
                )));
            }
            if kind == EntryKind::Directory
                && (uncompressed_size != 0 || compressed_size != 0 || crc != 0)
            {
                return Err(Error::CorruptArchive(format!(
                    "directory entry '{path}' must not carry data"
                )));
            }

            let record_and_path = (ENTRY_FIXED_SIZE as u64)
                .checked_add(path_len as u64)
                .ok_or_else(|| {
                    Error::CorruptArchive("offset calculation overflowed".to_string())
                })?;
            offset = offset.checked_add(record_and_path).ok_or_else(|| {
                Error::CorruptArchive("offset calculation overflowed".to_string())
            })?;
            let remaining = file_len.saturating_sub(offset);
            if compressed_size > remaining {
                return Err(Error::CorruptArchive(format!(
                    "entry '{path}' declares {compressed_size} compressed bytes \
                     but only {remaining} remain"
                )));
            }
            if uncompressed_size > MAX_ENTRY_UNCOMPRESSED_SIZE {
                return Err(Error::UnreasonableSize {
                    path: path.clone(),
                    size: uncompressed_size,
                });
            }

            data_offsets.push(offset);
            offset = offset.checked_add(compressed_size).ok_or_else(|| {
                Error::CorruptArchive("offset calculation overflowed".to_string())
            })?;
            entries.push(EntryMetadata {
                path,
                kind,
                mode,
                mtime,
                uncompressed_size,
                compressed_size,
                crc32: crc,
            });
            reader.seek(SeekFrom::Start(offset))?;
        }

        validate_entry_set(&entries)?;

        Ok(Self {
            reader,
            algorithm,
            version,
            algorithm_id,
            entries,
            data_offsets,
        })
    }

    /// The archive format version.
    pub const fn format_version(&self) -> u16 {
        self.version
    }

    /// The numeric id of the compression algorithm used by the archive.
    pub const fn algorithm_id(&self) -> u16 {
        self.algorithm_id
    }

    /// The name of the compression algorithm used by the archive.
    pub fn algorithm_name(&self) -> &'static str {
        self.algorithm.name()
    }

    /// Number of entries in the archive.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// All entry metadata, in archive order.
    pub fn entries(&self) -> &[EntryMetadata] {
        &self.entries
    }

    /// Metadata for a single entry.
    pub fn entry(&self, index: usize) -> &EntryMetadata {
        &self.entries[index]
    }

    /// Reads, decompresses, and verifies the data of one entry. The
    /// decompressed size must match the header and the CRC32 must match the
    /// data, otherwise the entry is rejected.
    pub fn read_entry_data(&mut self, index: usize) -> Result<Vec<u8>> {
        let Some(metadata) = self.entries.get(index) else {
            return Err(Error::InvalidInput(format!("no entry at index {index}")));
        };
        if metadata.kind == EntryKind::Directory {
            return Err(Error::InvalidInput(format!(
                "entry '{}' is a directory and has no data",
                metadata.path
            )));
        }
        if metadata.compressed_size > usize::MAX as u64 {
            return Err(Error::CorruptArchive(format!(
                "entry '{}' is too large for this platform",
                metadata.path
            )));
        }
        let mut compressed = vec![0u8; metadata.compressed_size as usize];
        self.reader
            .seek(SeekFrom::Start(self.data_offsets[index]))?;
        read_bytes(&mut self.reader, &mut compressed)?;

        let data = self.algorithm.decompress(&compressed).map_err(|e| {
            Error::CorruptArchive(format!("failed to decompress '{}': {e}", metadata.path))
        })?;
        if data.len() as u64 != metadata.uncompressed_size {
            return Err(Error::SizeMismatch {
                path: metadata.path.clone(),
                expected: metadata.uncompressed_size,
                actual: data.len() as u64,
            });
        }
        let actual_crc = crc32(&data);
        if actual_crc != metadata.crc32 {
            return Err(Error::IntegrityCheck {
                path: metadata.path.clone(),
                expected: metadata.crc32,
                actual: actual_crc,
            });
        }
        Ok(data)
    }
}

/// Rejects duplicate paths and file/directory conflicts.
fn validate_entry_set(entries: &[EntryMetadata]) -> Result<()> {
    let mut seen = HashSet::new();
    let mut files = HashSet::new();
    for entry in entries {
        let relative = paths::parse_archive_path(&entry.path)?;
        if !seen.insert(relative.clone()) {
            return Err(Error::DuplicatePath(entry.path.clone()));
        }
        if entry.kind == EntryKind::File {
            files.insert(relative);
        }
    }
    // A path may never live inside another path that is stored as a file.
    for entry in entries {
        let relative = paths::parse_archive_path(&entry.path)?;
        for ancestor in relative.ancestors().skip(1) {
            if files.contains(ancestor) {
                return Err(Error::ConflictingPath(format!(
                    "'{}' cannot live inside '{}', which is stored as a file",
                    entry.path,
                    ancestor.display()
                )));
            }
        }
    }
    Ok(())
}

fn read_bytes<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<()> {
    reader.read_exact(buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::TruncatedArchive(
                "the archive ended while a structure was being read".to_string(),
            )
        } else {
            Error::Io(e)
        }
    })
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

// Keep `le_u16` honest: it is currently used implicitly by header parsing
// above; the helper exists so all widths read uniformly.
#[cfg(test)]
mod tests {
    #[test]
    fn little_endian_helpers() {
        use super::*;
        assert_eq!(le_u16(&[0x01, 0x02]), 0x0201);
        assert_eq!(le_u32(&[0x01, 0x02, 0x03, 0x04]), 0x0403_0201);
        assert_eq!(le_u64(&[1, 2, 3, 4, 5, 6, 7, 8]), 0x0807_0605_0403_0201);
    }
}
