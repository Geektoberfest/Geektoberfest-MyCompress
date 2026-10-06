use std::fs;
use std::path::{Path, PathBuf};

use crate::archive::EntryKind;
use crate::error::{Error, Result};
use crate::paths;

/// One filesystem item that will become an archive entry.
#[derive(Clone, Debug)]
pub struct SourceEntry {
    /// Path as it will be stored in the archive (`/`-separated, relative).
    pub archive_path: String,
    /// Actual path on disk.
    pub fs_path: PathBuf,
    pub kind: EntryKind,
    /// Unix permission bits (0 = unspecified).
    pub mode: u32,
    /// Modification time, seconds since the Unix epoch (0 = unspecified).
    pub mtime: u64,
    /// File size in bytes (0 for directories).
    pub size: u64,
}

/// Collects everything under `input` that should be archived.
///
/// - If `input` is a file, the archive contains a single entry named after
///   that file.
/// - If `input` is a directory, the archive contains its *contents*, with
///   paths relative to the directory itself.
///
/// Symbolic links and special files (sockets, fifos, ...) are skipped and
/// reported in the returned warnings; the same goes for names that cannot be
/// represented in the archive (non-UTF-8 names, Windows-reserved names...).
/// The result is sorted by archive path, which places parents before children
/// and makes archives deterministic.
///
/// `skip` optionally names a canonical path to exclude; the compressor uses
/// it to avoid archiving the archive it is currently writing.
pub fn collect_entries(
    input: &Path,
    skip: Option<&Path>,
) -> Result<(Vec<SourceEntry>, Vec<String>)> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    let metadata = fs::metadata(input).map_err(|e| {
        Error::InvalidInput(format!("cannot read input '{}': {e}", input.display()))
    })?;
    if metadata.is_dir() {
        walk_dir(input, "", &mut entries, &mut warnings)?;
    } else if metadata.is_file() {
        let name = input.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
            Error::InvalidInput(format!(
                "input file name '{}' is not valid UTF-8",
                input.display()
            ))
        })?;
        if let Err(reason) = paths::validate_component(name) {
            return Err(Error::InvalidInput(format!(
                "input file name cannot be stored in an archive: {reason}"
            )));
        }
        entries.push(SourceEntry {
            archive_path: name.to_string(),
            fs_path: input.to_path_buf(),
            kind: EntryKind::File,
            mode: read_mode(&metadata),
            mtime: read_mtime(&metadata),
            size: metadata.len(),
        });
    } else {
        return Err(Error::UnsupportedEntry(input.to_path_buf()));
    }

    entries.sort_by(|a, b| a.archive_path.cmp(&b.archive_path));

    if let Some(skip) = skip {
        entries.retain(|entry| {
            if entry.kind != EntryKind::File {
                return true;
            }
            // Fast path first; canonicalize handles ../ and symlinks.
            !(entry.fs_path == skip || entry.fs_path.canonicalize().is_ok_and(|p| p == skip))
        });
    }

    Ok((entries, warnings))
}

fn walk_dir(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<SourceEntry>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let dir_entries = fs::read_dir(dir)?;
    let mut children: Vec<(String, PathBuf, fs::FileType)> = Vec::new();
    for entry in dir_entries {
        let entry = entry?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => {
                warnings.push(format!(
                    "skipping entry with a non-UTF-8 name in '{}'",
                    dir.display()
                ));
                continue;
            }
        };
        children.push((name, entry.path(), entry.file_type()?));
    }
    children.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, path, file_type) in children {
        let archive_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if let Err(reason) = paths::validate_component(&name) {
            warnings.push(format!("skipping '{archive_path}': {reason}"));
            continue;
        }
        if file_type.is_symlink() {
            warnings.push(format!("skipping symlink '{archive_path}'"));
            continue;
        }
        if file_type.is_dir() {
            let metadata = fs::metadata(&path)?;
            out.push(SourceEntry {
                archive_path: archive_path.clone(),
                fs_path: path.clone(),
                kind: EntryKind::Directory,
                mode: read_mode(&metadata),
                mtime: read_mtime(&metadata),
                size: 0,
            });
            walk_dir(&path, &archive_path, out, warnings)?;
        } else if file_type.is_file() {
            let metadata = fs::metadata(&path)?;
            out.push(SourceEntry {
                archive_path,
                fs_path: path,
                kind: EntryKind::File,
                mode: read_mode(&metadata),
                mtime: read_mtime(&metadata),
                size: metadata.len(),
            });
        } else {
            warnings.push(format!("skipping special file '{archive_path}'"));
        }
    }
    Ok(())
}

fn read_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Only permission bits travel into the archive; setuid/setgid/sticky
        // bits are dropped for security.
        metadata.mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        if metadata.permissions().readonly() {
            0o444
        } else {
            0
        }
    }
}

fn read_mtime(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}
