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
    let mut reader = ArchiveReader::open(archive, registry)?;
    extract_from_reader(&mut reader, dest, options)
}

/// Extracts every entry of an already-opened archive into `dest`.
///
/// Security guarantees:
///
/// - entry paths were validated when the archive was opened and are
///   re-validated here (defense in depth): no `..`, no absolute paths, no
///   drive letters, no reserved device names — extraction can never write
///   outside `dest`;
/// - files are created with `create_new`, so an existing file is never
///   silently overwritten unless `options.force` is set;
/// - every entry is verified (decompressed size + CRC32) before it is
///   written to disk;
/// - setuid/setgid/sticky permission bits are never restored.
///
/// Existing directories are reused; existing files are never touched without
/// `force`. Modification times and supported permissions are restored on a
/// best-effort basis for both files and directories. For directory entries
/// present in the archive, metadata is restored in reverse depth order after
/// all entries and children have been extracted so child processing does not
/// alter parent modification times; pre-existing directories matching archive
/// entries have their metadata updated, while the destination root directory
/// itself is untouched. On Unix, permission bits are masked to `0o777`
/// (setuid, setgid, and sticky bits are never restored). On Windows, permissions
/// map missing write bits to the read-only attribute. Failures to restore metadata
/// are ignored and do not cause extraction to fail.
pub fn extract_from_reader(
    reader: &mut ArchiveReader,
    dest: &Path,
    options: &ExtractOptions,
) -> Result<ExtractStats> {
    prepare_destination(dest)?;

    let mut stats = ExtractStats::default();
    let metadata: Vec<_> = reader.entries().to_vec();
    let mut pending_dirs = Vec::new();
    for (index, entry) in metadata.iter().enumerate() {
        // Re-validate even though the reader already did: cheap and safe.
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
                pending_dirs.push(PendingDir {
                    target,
                    depth: relative.components().count(),
                    mtime: entry.mtime,
                    mode: entry.mode,
                });
            }
            EntryKind::File => {
                // Decompress + verify (size and CRC32) before touching disk.
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
    }

    // Restore directory metadata in reverse depth order (deepest child directory first)
    // so processing child entries does not alter parent modification times.
    pending_dirs.sort_by_key(|b| std::cmp::Reverse(b.depth));
    for dir in pending_dirs {
        if dir.mtime != 0 {
            restore_mtime(&dir.target, dir.mtime);
        }
        if dir.mode != 0 {
            restore_mode(&dir.target, dir.mode);
        }
    }

    Ok(stats)
}

struct PendingDir {
    target: std::path::PathBuf,
    depth: usize,
    mtime: u64,
    mode: u32,
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

/// Best-effort restoration of a file or directory modification time.
fn restore_mtime(path: &Path, secs: u64) {
    let Some(time) = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs)) else {
        return; // Implausible timestamp; ignore.
    };
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .or_else(|_| {
                OpenOptions::new()
                    .write(true)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(path)
            })
            .or_else(|_| OpenOptions::new().write(true).open(path))
    };
    #[cfg(unix)]
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path));
    #[cfg(not(any(windows, unix)))]
    let file = OpenOptions::new().write(true).open(path);

    if let Ok(file) = file {
        let _ = file.set_modified(time);
    }
}

#[cfg(unix)]
fn restore_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    // Drop setuid/setgid/sticky bits: an archive must never be able to
    // grant elevated permissions during extraction.
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
