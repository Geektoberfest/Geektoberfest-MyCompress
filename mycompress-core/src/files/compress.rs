use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::algo::{AlgorithmId, CompressionAlgorithm, Registry, MAX_DECOMPRESSED_BYTES};
use crate::archive::writer::ArchiveWriter;
use crate::archive::EntryKind;
use crate::crc32::Crc32;
use crate::error::{Error, Result};

use super::file_op;
use super::walk::{SourceEntry, collect_entries};

/// Options for [`compress`].
#[derive(Clone, Copy, Debug)]
pub struct CompressOptions {
    /// Which algorithm to use.
    pub algorithm: AlgorithmId,
    /// Overwrite the output archive if it already exists.
    pub force: bool,
}

/// Summary of a completed compression run.
#[derive(Clone, Debug, Default)]
pub struct CompressStats {
    pub file_count: u32,
    pub dir_count: u32,
    /// Total uncompressed size of all stored files.
    pub total_uncompressed: u64,
    /// Size of the finished archive on disk (including headers).
    pub archive_size: u64,
    /// Non-fatal issues encountered while walking the input.
    pub warnings: Vec<String>,
}

/// Compresses `input` (a file or a directory tree) into a `.mcz` archive at
/// `output`.
///
/// Files using RLE are streamed in fixed-size chunks, so the uncompressed
/// contents are never fully held in memory. Other algorithms need the whole
/// input (Huffman builds a global frequency table), so those files are read
/// with a hard size cap instead; see the limitations section below.
///
/// Guarantees:
///
/// - the output archive is never overwritten unless `options.force` is set;
/// - the compressor never archives the archive it is writing, even when the
///   output lives inside the input directory;
/// - if compression fails midway, the partial archive is deleted so that no
///   broken archives are left behind.
///
/// # Limitations
///
/// The [`CompressionAlgorithm`] interface compresses whole slices, and Huffman
/// codes depend on frequencies across the entire input, so Huffman (and custom)
/// algorithms still buffer one file at a time, capped at
/// [`MAX_DECOMPRESSED_BYTES`] (64 MiB) to fail cleanly instead of exhausting
/// memory. Only RLE compresses incrementally.
///
/// # Errors
///
/// Returns an error if the input is missing or unsupported, if the output
/// exists and `force` is unset, or if any I/O or compression error occurs.
pub fn compress(
    input: &Path,
    output: &Path,
    options: &CompressOptions,
    registry: &Registry,
) -> Result<CompressStats> {
    validate_input(input)?;

    if same_path(input, output)? {
        return Err(Error::InvalidInput(
            "the output archive must be different from the input".to_string(),
        ));
    }
    if output.exists() && !options.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }

    // Create the destination before walking the input, so that the walker
    // can recognize (and skip) the archive itself.
    let file = if options.force {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output)
    } else {
        OpenOptions::new().write(true).create_new(true).open(output)
    }
    .map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists => Error::OutputExists(output.to_path_buf()),
        _ => Error::Io(e),
    })?;

    match write_archive(input, output, file, options, registry) {
        Ok(stats) => Ok(stats),
        Err(error) => {
            // Never leave a half-written archive behind.
            let _ = fs::remove_file(output);
            Err(error)
        }
    }
}

fn validate_input(input: &Path) -> Result<()> {
    let symlink = fs::symlink_metadata(input).map_err(|e| {
        Error::InvalidInput(format!("cannot read input '{}': {e}", input.display()))
    })?;
    if symlink.file_type().is_symlink() {
        return Err(Error::UnsupportedEntry(input.to_path_buf()));
    }
    let metadata = fs::metadata(input).map_err(|e| {
        Error::InvalidInput(format!("cannot read input '{}': {e}", input.display()))
    })?;
    if !metadata.is_dir() && !metadata.is_file() {
        return Err(Error::UnsupportedEntry(input.to_path_buf()));
    }
    Ok(())
}

/// Cheap lexical/`canonicalize` equality check between two paths.
fn same_path(a: &Path, b: &Path) -> Result<bool> {
    if a == b {
        return Ok(true);
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => Ok(a == b),
        // If either path does not exist yet, they cannot be the same file.
        _ => Ok(false),
    }
}

fn write_archive(
    input: &Path,
    output: &Path,
    file: File,
    options: &CompressOptions,
    registry: &Registry,
) -> Result<CompressStats> {
    let algorithm = registry.get(options.algorithm.as_u16()).ok_or_else(|| {
        Error::InvalidInput(format!(
            "algorithm '{}' is not registered",
            options.algorithm
        ))
    })?;

    let skip: Option<PathBuf> = output.canonicalize().ok();
    let (entries, warnings) = collect_entries(input, skip.as_deref())?;

    let mut stats = CompressStats {
        warnings,
        ..Default::default()
    };
    let writer_algorithm = Arc::clone(&algorithm);
    let mut writer = ArchiveWriter::new(BufWriter::new(file), algorithm)?;
    for entry in &entries {
        match entry.kind {
            EntryKind::Directory => {
                writer.add_dir(&entry.archive_path, entry.mode, entry.mtime)?;
                stats.dir_count += 1;
            }
            EntryKind::File => {
                // Fail fast on files already known to exceed the per-entry cap.
                if entry.size > MAX_DECOMPRESSED_BYTES {
                    return Err(Error::UnreasonableSize {
                        path: entry.archive_path.clone(),
                        size: entry.size,
                    });
                }
                let total = if options.algorithm == AlgorithmId::Rle {
                    compress_rle_streamed(entry, &mut writer, &writer_algorithm)?
                } else {
                    compress_buffered(entry, &mut writer)?
                };
                stats.file_count += 1;
                stats.total_uncompressed += total;
            }
        }
    }
    writer.finish()?;
    stats.archive_size = fs::metadata(output).map_or(0, |m| m.len());
    Ok(stats)
}

/// Fixed read size for streaming compression (64 KiB).
const COMPRESS_CHUNK_BYTES: usize = 64 * 1024;

/// Streams one file through RLE chunk by chunk.
///
/// RLE blocks are self-delimiting, so encoding each chunk independently and
/// concatenating the outputs still decodes to the original bytes (slightly
/// less optimal across chunk edges only). The uncompressed file is never
/// fully materialized: peak memory is one chunk plus the compressed output.
fn compress_rle_streamed(
    entry: &SourceEntry,
    writer: &mut ArchiveWriter<BufWriter<File>>,
    algorithm: &Arc<dyn CompressionAlgorithm>,
) -> Result<u64> {
    let mut source = File::open(&entry.fs_path).map_err(|e| file_op("read", &entry.fs_path, e))?;
    let mut hasher = Crc32::new();
    let mut compressed: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; COMPRESS_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let n = match source.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(file_op("read", &entry.fs_path, e)),
        };
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_DECOMPRESSED_BYTES {
            return Err(Error::UnreasonableSize {
                path: entry.archive_path.clone(),
                size: total,
            });
        }
        let chunk = &buf[..n];
        hasher.update(chunk);
        let part = algorithm.compress(chunk)?;
        compressed.extend_from_slice(&part);
    }
    let crc = hasher.finalize();
    writer.add_file_precompressed(
        &entry.archive_path,
        total,
        &compressed,
        entry.mode,
        entry.mtime,
        crc,
    )?;
    Ok(total)
}

/// Reads one file with a hard cap for algorithms that need the whole input.
///
/// Huffman builds a global frequency table, so it cannot encode incrementally
/// through the current [`CompressionAlgorithm`] interface. The file is still
/// read in chunks (CRC computed on the fly) so growth past the cap fails
/// cleanly instead of exhausting memory.
fn compress_buffered(
    entry: &SourceEntry,
    writer: &mut ArchiveWriter<BufWriter<File>>,
) -> Result<u64> {
    let mut source = File::open(&entry.fs_path).map_err(|e| file_op("read", &entry.fs_path, e))?;
    let mut data: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; COMPRESS_CHUNK_BYTES];
    loop {
        let n = match source.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(file_op("read", &entry.fs_path, e)),
        };
        if n == 0 {
            break;
        }
        if data.len() as u64 + n as u64 > MAX_DECOMPRESSED_BYTES {
            return Err(Error::UnreasonableSize {
                path: entry.archive_path.clone(),
                size: data.len() as u64 + n as u64,
            });
        }
        data.extend_from_slice(&buf[..n]);
    }
    let total = data.len() as u64;
    writer.add_file(&entry.archive_path, &data, entry.mode, entry.mtime)?;
    Ok(total)
}
