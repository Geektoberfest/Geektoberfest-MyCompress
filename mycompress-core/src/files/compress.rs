use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Read};
use std::path::{Path, PathBuf};

use crate::algo::{select_algorithm, AlgorithmId, Registry};
use crate::archive::writer::ArchiveWriter;
use crate::archive::EntryKind;
use crate::error::{Error, Result};

use super::file_op;
use super::walk::{collect_entries, SourceEntry};

/// Maximum number of bytes sampled from the input to select an algorithm (64 KiB).
pub const MAX_SAMPLE_BYTES: usize = 64 * 1024;

/// How the compression algorithm should be selected for an archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlgorithmSelection {
    /// Automatically select the algorithm by sampling input data.
    Auto,
    /// Explicitly use the given algorithm.
    Explicit(AlgorithmId),
}

impl From<AlgorithmId> for AlgorithmSelection {
    fn from(id: AlgorithmId) -> Self {
        Self::Explicit(id)
    }
}

/// Options for [`compress`].
#[derive(Clone, Copy, Debug)]
pub struct CompressOptions {
    /// Which algorithm to use, or [`AlgorithmSelection::Auto`] to select automatically.
    pub algorithm: AlgorithmSelection,
    /// Overwrite the output archive if it already exists.
    pub force: bool,
}

impl CompressOptions {
    /// Creates options with the given algorithm selection mode.
    pub const fn new(algorithm: AlgorithmSelection, force: bool) -> Self {
        Self { algorithm, force }
    }

    /// Creates options with an explicit algorithm.
    pub const fn explicit(algorithm: AlgorithmId, force: bool) -> Self {
        Self {
            algorithm: AlgorithmSelection::Explicit(algorithm),
            force,
        }
    }

    /// Creates options with automatic algorithm selection.
    pub const fn auto(force: bool) -> Self {
        Self {
            algorithm: AlgorithmSelection::Auto,
            force,
        }
    }
}

impl From<AlgorithmId> for CompressOptions {
    fn from(algorithm: AlgorithmId) -> Self {
        Self::explicit(algorithm, false)
    }
}

/// Summary of a completed compression run.
#[derive(Clone, Debug)]
pub struct CompressStats {
    pub file_count: u32,
    pub dir_count: u32,
    /// Total uncompressed size of all stored files.
    pub total_uncompressed: u64,
    /// Size of the finished archive on disk (including headers).
    pub archive_size: u64,
    /// Non-fatal issues encountered while walking the input.
    pub warnings: Vec<String>,
    /// The concrete algorithm chosen and used for this archive.
    pub algorithm: AlgorithmId,
}

impl Default for CompressStats {
    fn default() -> Self {
        Self {
            file_count: 0,
            dir_count: 0,
            total_uncompressed: 0,
            archive_size: 0,
            warnings: Vec::new(),
            algorithm: AlgorithmId::Rle,
        }
    }
}

/// Compresses `input` (a file or a directory tree) into a `.mcz` archive at
/// `output`.
///
/// Guarantees:
///
/// - the output archive is never overwritten unless `options.force` is set;
/// - the compressor never archives the archive it is writing, even when the
///   output lives inside the input directory;
/// - if compression fails midway, the partial archive is deleted so that no
///   broken archives are left behind.
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

/// Samples up to [`MAX_SAMPLE_BYTES`] cumulatively across regular files in `entries`.
///
/// Directories and metadata are ignored. Bounding the sample per file using
/// [`Read::take`] prevents reading large files solely for algorithm selection.
///
/// # Trade-offs
/// Reading a small bounded sample across regular files incurs minimal I/O overhead
/// (operating system buffer caches keep initial file blocks warm) while keeping
/// archive creation decoupled, simple, and streaming without holding large payloads.
fn collect_sample(entries: &[SourceEntry]) -> Result<Vec<u8>> {
    let mut sample = Vec::with_capacity(MAX_SAMPLE_BYTES);
    for entry in entries {
        if entry.kind != EntryKind::File {
            continue;
        }
        let remaining = MAX_SAMPLE_BYTES.saturating_sub(sample.len());
        if remaining == 0 {
            break;
        }
        let file = File::open(&entry.fs_path).map_err(|e| file_op("open", &entry.fs_path, e))?;
        let mut handle = file.take(remaining as u64);
        handle
            .read_to_end(&mut sample)
            .map_err(|e| file_op("read", &entry.fs_path, e))?;
    }
    Ok(sample)
}

fn write_archive(
    input: &Path,
    output: &Path,
    file: File,
    options: &CompressOptions,
    registry: &Registry,
) -> Result<CompressStats> {
    let skip: Option<PathBuf> = output.canonicalize().ok();
    let (entries, warnings) = collect_entries(input, skip.as_deref())?;

    let algorithm_id = match options.algorithm {
        AlgorithmSelection::Explicit(id) => id,
        AlgorithmSelection::Auto => {
            let sample = collect_sample(&entries)?;
            select_algorithm(&sample, registry)
        }
    };

    let algorithm = registry.get(algorithm_id.as_u16()).ok_or_else(|| {
        Error::InvalidInput(format!("algorithm '{algorithm_id}' is not registered"))
    })?;

    let mut stats = CompressStats {
        warnings,
        algorithm: algorithm_id,
        ..Default::default()
    };
    let mut writer = ArchiveWriter::new(BufWriter::new(file), algorithm)?;
    for entry in &entries {
        match entry.kind {
            EntryKind::Directory => {
                writer.add_dir(&entry.archive_path, entry.mode, entry.mtime)?;
                stats.dir_count += 1;
            }
            EntryKind::File => {
                let data =
                    fs::read(&entry.fs_path).map_err(|e| file_op("read", &entry.fs_path, e))?;
                writer.add_file(&entry.archive_path, &data, entry.mode, entry.mtime)?;
                stats.file_count += 1;
                stats.total_uncompressed += data.len() as u64;
            }
        }
    }
    writer.finish()?;
    stats.archive_size = fs::metadata(output).map_or(0, |m| m.len());
    Ok(stats)
}
