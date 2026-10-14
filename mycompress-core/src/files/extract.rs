use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::algo::Registry;
use crate::archive::{ArchiveReader, EntryKind};
use crate::error::{Error, Result};
use crate::paths;

use super::file_op;

/// Options for [`extract`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtractOptions {
    /// Overwrite existing files during extraction.
    pub force: bool,
}

/// Summary of a completed extraction run.
#[derive(Clone, Debug, Default)]
pub struct ExtractStats {
    pub files_written: u32,
    pub dirs_created: u32,
    pub bytes_written: u64,
}

/// Extracts a `.mcz` archive from disk into `dest`.
///
/// # Example
///
/// ```no_run
/// use mycompress_core::files::{self, ExtractOptions};
/// use mycompress_core::Registry;
///
/// # fn main() -> mycompress_core::Result<()> {
/// let registry = Registry::new();
/// files::extract(
///     std::path::Path::new("archive.mcz"),
///     std::path::Path::new("extracted"),
///     &ExtractOptions::default(),
///     &registry,
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn extract(
    archive: &Path,
    dest: &Path,
    options: &ExtractOptions,
    registry: &Registry,
) -> Result<ExtractStats> {
    extract_with_progress(archive, dest, options, registry, |_, _| {})
}

/// Extracts an archive while reporting progress after each completed entry.
///
/// The callback receives:
/// - `completed_entries`: number of successfully processed archive entries;
/// - `completed_bytes`: total bytes written from files so far.
///
/// The callback does not affect the extraction result.
pub fn extract_with_progress<F>(
    archive: &Path,
    dest: &Path,
    options: &ExtractOptions,
    registry: &Registry,
    on_progress: F,
) -> Result<ExtractStats>
where
    F: FnMut(u64, u64),
{
    let mut reader = ArchiveReader::open(archive, registry)?;
    extract_from_reader_with_progress(&mut reader, dest, options, on_progress)
}

/// Extracts every entry of an already-opened archive into `dest`.
///
/// Security guarantees:
///
/// - entry paths are re-validated: no `..`, absolute paths, drive letters, or
///   reserved device names;
/// - files are created with `create_new`, so existing files are not silently
///   overwritten unless `options.force` is set;
/// - every file entry is verified (decompressed size and CRC32) before it is
///   written to disk;
/// - setuid/setgid/sticky permission bits are never restored.
///
/// Existing directories are reused. Modification times and permissions are
/// restored on a best-effort basis for files.
pub fn extract_from_reader(
    reader: &mut ArchiveReader,
    dest: &Path,
    options: &ExtractOptions,
) -> Result<ExtractStats> {
    extract_from_reader_with_progress(reader, dest, options, |_, _| {})
}

/// Extracts entries while reporting progress after each successfully handled
/// entry.
pub fn extract_from_reader_with_progress<F>(
    reader: &mut ArchiveReader,
    dest: &Path,
    options: &ExtractOptions,
    mut on_progress: F,
) -> Result<ExtractStats>
where
    F: FnMut(u64, u64),
{
    prepare_destination(dest)?;

    let mut stats = ExtractStats::default();
    let metadata: Vec<_> = reader.entries().to_vec();

    for (index, entry) in metadata.iter().enumerate() {
        // Re-validate even though the reader already did: defense in depth.
        let relative = paths::parse_archive_path(&entry.path)?;
        let target = dest.join(&relative);

        debug_assert!(
            target.starts_with(dest),
            "validated paths can never escape the destination"
        );

        match entry.kind {
            EntryKind::Directory => {
                if target.exists() && !target.is_dir() {
                    return Err(Error::InvalidInput(format!(
                        "cannot create directory '{}': a file with that name already exists",
                        target.display()
                    )));
                }

                if !target.exists() {
                    fs::create_dir_all(&target)
                        .map_err(|e| file_op("create directory", &target, e))?;
                    stats.dirs_created += 1;
                }
            }

            EntryKind::File => {
                // Decompress and verify size + CRC32 before touching disk.
                let data = reader.read_entry_data(index)?;

                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| file_op("create directory", parent, e))?;
                }

                let mut file = if options.force {
                    OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&target)
                } else {
                    // Atomic never-overwrite: fails if the file already exists.
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)
                }
                .map_err(|e| match e.kind() {
                    ErrorKind::AlreadyExists => Error::OutputExists(target.clone()),
                    _ => file_op("write", &target, e),
                })?;

                file.write_all(&data)
                    .map_err(|e| file_op("write", &target, e))?;
                file.flush().map_err(|e| file_op("write", &target, e))?;
                drop(file);

                stats.files_written += 1;
                stats.bytes_written += data.len() as u64;

                if entry.mtime != 0 {
                    restore_mtime(&target, entry.mtime);
                }

                if entry.mode != 0 {
                    restore_mode(&target, entry.mode);
                }
            }
        }

        // Report progress only after the entry has been processed successfully.
        on_progress((index + 1) as u64, stats.bytes_written);
    }

    Ok(stats)
}

fn prepare_destination(dest: &Path) -> Result<()> {
    match fs::metadata(dest) {
        Ok(metadata) if metadata.is_dir() => Ok(()),

        Ok(_) => Err(Error::InvalidInput(format!(
            "destination '{}' exists and is not a directory",
            dest.display()
        ))),

        Err(_) => fs::create_dir_all(dest).map_err(|e| {
            Error::InvalidInput(format!(
                "cannot create destination '{}': {e}",
                dest.display()
            ))
        }),
    }
}

/// Best-effort restoration of a file's modification time.
fn restore_mtime(path: &Path, secs: u64) {
    let Some(time) = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs)) else {
        return;
    };

    if let Ok(file) = OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(time);
    }
}

#[cfg(unix)]
fn restore_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;

    // Never restore setuid, setgid, or sticky bits from an archive.
    let mode = mode & 0o777;

    if mode == 0 {
        return;
    }

    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(mode);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(windows)]
fn restore_mode(path: &Path, mode: u32) {
    if let Ok(metadata) = fs::metadata(path) {
        // A file without any write bit is mapped to the read-only attribute.
        let read_only = mode & 0o222 == 0;
        let mut permissions = metadata.permissions();
        permissions.set_readonly(read_only);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(any(unix, windows)))]
fn restore_mode(_path: &Path, _mode: u32) {}
