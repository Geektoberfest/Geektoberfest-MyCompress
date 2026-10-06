use std::collections::HashSet;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use crate::algo::CompressionAlgorithm;
use crate::archive::format::{ENTRY_FIXED_SIZE, FORMAT_VERSION, HEADER_SIZE, MAGIC};
use crate::archive::{EntryKind, EntryMetadata};
use crate::crc32::crc32;
use crate::error::{Error, Result};
use crate::paths;

/// Writes `.mcz` archives to any seekable output.
///
/// The entry count is part of the header but is only known once every entry
/// has been written, so the writer starts with a zero placeholder and patches
/// the real count in [`ArchiveWriter::finish`]. This is why `W` must
/// implement [`Seek`]. Wrap files in [`std::io::BufWriter`] for speed.
///
/// The writer validates every path ([`paths::parse_archive_path`]) and
/// rejects duplicate or conflicting paths, so it can never produce an
/// archive that the reader would refuse to open.
///
/// # Example
///
/// ```
/// use std::io::Cursor;
/// use mycompress_core::{ArchiveWriter, Registry};
///
/// # fn main() -> mycompress_core::Result<()> {
/// let registry = Registry::new();
/// let rle = registry.get_by_name("rle").unwrap();
/// let mut buffer = Vec::new();
/// {
///     let mut writer = ArchiveWriter::new(Cursor::new(&mut buffer), rle)?;
///     writer.add_file("greeting.txt", b"hello", 0o644, 0)?;
///     writer.finish()?;
/// }
/// assert!(buffer.starts_with(b"MCZ1"));
/// # Ok(())
/// # }
/// ```
pub struct ArchiveWriter<W> {
    out: W,
    algorithm: Arc<dyn CompressionAlgorithm>,
    entry_count: u32,
    paths: HashSet<String>,
    file_paths: HashSet<String>,
}

impl<W: Write + Seek> ArchiveWriter<W> {
    /// Writes the archive header (with a placeholder entry count).
    pub fn new(mut out: W, algorithm: Arc<dyn CompressionAlgorithm>) -> Result<Self> {
        let mut header = [0u8; HEADER_SIZE];
        header[..MAGIC.len()].copy_from_slice(&MAGIC);
        header[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        header[6..8].copy_from_slice(&algorithm.id().as_u16().to_le_bytes());
        // Entry count (offset 8) is patched by `finish`; offset 12 stays zero.
        out.write_all(&header)?;
        Ok(Self {
            out,
            algorithm,
            entry_count: 0,
            paths: HashSet::new(),
            file_paths: HashSet::new(),
        })
    }

    /// Adds a file entry. `data` is the uncompressed file contents.
    pub fn add_file(&mut self, path: &str, data: &[u8], mode: u32, mtime: u64) -> Result<()> {
        self.check_new_path(path, EntryKind::File)?;
        let compressed = self.algorithm.compress(data)?;
        let metadata = EntryMetadata {
            path: path.to_string(),
            kind: EntryKind::File,
            mode,
            mtime,
            uncompressed_size: data.len() as u64,
            compressed_size: compressed.len() as u64,
            crc32: crc32(data),
        };
        self.write_record(&metadata)?;
        self.out.write_all(&compressed)?;
        self.bump_entry_count()
    }

    /// Adds a directory entry (directories carry no data).
    pub fn add_dir(&mut self, path: &str, mode: u32, mtime: u64) -> Result<()> {
        self.check_new_path(path, EntryKind::Directory)?;
        let metadata = EntryMetadata {
            path: path.to_string(),
            kind: EntryKind::Directory,
            mode,
            mtime,
            uncompressed_size: 0,
            compressed_size: 0,
            crc32: 0,
        };
        self.write_record(&metadata)?;
        self.bump_entry_count()
    }

    /// Number of entries written so far.
    pub fn entry_count(&self) -> u32 {
        self.entry_count
    }

    /// Patches the entry count into the header and flushes the output.
    pub fn finish(mut self) -> Result<()> {
        self.out.seek(SeekFrom::Start(8))?;
        self.out.write_all(&self.entry_count.to_le_bytes())?;
        self.out.flush()?;
        Ok(())
    }

    fn bump_entry_count(&mut self) -> Result<()> {
        self.entry_count = self.entry_count.checked_add(1).ok_or_else(|| {
            Error::InvalidInput("too many entries for a single archive".to_string())
        })?;
        Ok(())
    }

    /// Rejects duplicate paths and files stored inside files.
    fn check_new_path(&self, path: &str, kind: EntryKind) -> Result<()> {
        paths::parse_archive_path(path)?;
        if self.paths.contains(path) {
            return Err(Error::DuplicatePath(path.to_string()));
        }
        if kind == EntryKind::File {
            for ancestor in Path::new(path).ancestors().skip(1) {
                if let Some(ancestor) = ancestor.to_str() {
                    if self.file_paths.contains(ancestor) {
                        return Err(Error::ConflictingPath(format!(
                            "'{path}' cannot be stored inside '{ancestor}', which was added as a file"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn write_record(&mut self, metadata: &EntryMetadata) -> Result<()> {
        let path_bytes = metadata.path.as_bytes();
        let path_len = u16::try_from(path_bytes.len())
            .map_err(|_| Error::InvalidArchivePath(metadata.path.clone()))?;
        let mut record = [0u8; ENTRY_FIXED_SIZE];
        record[0..2].copy_from_slice(&path_len.to_le_bytes());
        record[2] = metadata.kind.as_u8();
        // record[3] is reserved and stays zero.
        record[4..8].copy_from_slice(&metadata.mode.to_le_bytes());
        record[8..16].copy_from_slice(&metadata.mtime.to_le_bytes());
        record[16..24].copy_from_slice(&metadata.uncompressed_size.to_le_bytes());
        record[24..32].copy_from_slice(&metadata.compressed_size.to_le_bytes());
        record[32..36].copy_from_slice(&metadata.crc32.to_le_bytes());
        self.out.write_all(&record)?;
        self.out.write_all(path_bytes)?;
        if kind_is_file(metadata) {
            self.paths.insert(metadata.path.clone());
            self.file_paths.insert(metadata.path.clone());
        } else {
            self.paths.insert(metadata.path.clone());
        }
        Ok(())
    }
}

fn kind_is_file(metadata: &EntryMetadata) -> bool {
    metadata.kind == EntryKind::File
}
