//! Validation for paths stored inside `.mcz` archives.
//!
//! Archive paths are always relative, `/`-separated, and UTF-8. The rules in
//! this module are deliberately stricter than any single operating system
//! requires so that an archive created anywhere can be extracted safely
//! everywhere:
//!
//! - no empty paths, no absolute paths, no `\` separators, no NUL bytes;
//! - no `.` or `..` components (`..` is reported as a path-traversal attempt);
//! - no Windows drive letters or reserved device names (`CON`, `NUL`, ...);
//! - no components with a trailing space or dot (problematic on Windows);
//! - at most 4096 bytes per path and 255 bytes per component.

use std::path::PathBuf;

use crate::archive::format::MAX_PATH_LEN;
use crate::error::{Error, Result};

/// Validates a path exactly as it may appear in an archive and returns it as
/// a relative [`PathBuf`].
///
/// This is the single source of truth used by both the archive writer (so we
/// never write a path we would refuse to read) and the archive reader (so we
/// never extract something dangerous).
pub fn parse_archive_path(raw: &str) -> Result<PathBuf> {
    if raw.is_empty() {
        return Err(Error::InvalidArchivePath("path is empty".to_string()));
    }
    if raw.len() > MAX_PATH_LEN {
        return Err(Error::InvalidArchivePath(format!(
            "path is longer than {MAX_PATH_LEN} bytes"
        )));
    }
    if raw.contains('\0') {
        return Err(Error::InvalidArchivePath(
            "path contains a NUL byte".to_string(),
        ));
    }
    if raw.contains('\\') {
        return Err(Error::InvalidArchivePath(
            "path contains a backslash (only '/' may be used as separator)".to_string(),
        ));
    }
    if raw.starts_with('/') {
        return Err(Error::InvalidArchivePath("path is absolute".to_string()));
    }

    let mut components: Vec<&str> = Vec::new();
    for component in raw.split('/') {
        if component == ".." {
            return Err(Error::PathTraversal(raw.to_string()));
        }
        if let Err(reason) = validate_component(component) {
            return Err(Error::InvalidArchivePath(format!(
                "invalid component '{component}' in '{raw}': {reason}"
            )));
        }
        components.push(component);
    }
    Ok(components.into_iter().collect())
}

/// Validates a single path component. Returns a static, human-readable reason
/// on failure. The `..` component is handled by [`parse_archive_path`]
/// because it is a security issue rather than a formatting issue.
pub fn validate_component(component: &str) -> std::result::Result<(), &'static str> {
    if component.is_empty() {
        return Err("empty path component");
    }
    if component == "." {
        return Err("'.' path component");
    }
    if component.len() > 255 {
        return Err("component is longer than 255 bytes");
    }
    if component.contains(':') {
        return Err("':' is not allowed in archive paths (would be a drive letter on Windows)");
    }
    if component.ends_with(' ') || component.ends_with('.') {
        return Err("component ends with a space or dot (not portable)");
    }
    if is_windows_reserved(component) {
        return Err("reserved Windows device name");
    }
    Ok(())
}

/// True for names like `CON`, `NUL`, `COM1`... that Windows treats as devices.
/// Case-insensitive, and also matches names with any extension (`CON.txt`).
fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "AUX"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "CON"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "NUL"
            | "PRN"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(path: &str) {
        assert!(
            parse_archive_path(path).is_ok(),
            "expected '{path}' to be valid"
        );
    }

    fn err(path: &str) {
        assert!(
            parse_archive_path(path).is_err(),
            "expected '{path}' to be rejected"
        );
    }

    #[test]
    fn accepts_simple_relative_paths() {
        ok("a");
        ok("a/b");
        ok("a/b/c.txt");
        ok("cafe/hello.txt");
        ok("0123/456.bin");
        ok("a.b/c");
        ok("a...b");
        ok("dot.dir/file.tar.gz");
    }

    #[test]
    fn rejects_empty_paths() {
        err("");
    }

    #[test]
    fn rejects_traversal() {
        for path in ["..", "a/..", "../a", "a/../../b", "a/../b", "a/b/../c"] {
            assert!(
                matches!(parse_archive_path(path), Err(Error::PathTraversal(_))),
                "expected '{path}' to be flagged as traversal"
            );
        }
    }

    #[test]
    fn rejects_absolute_paths_and_bad_separators() {
        err("/a");
        err("/");
        err("//a");
        err("a/");
        err("a//b");
        err("a\\b");
        err("\\a");
    }

    #[test]
    fn rejects_windows_hazards() {
        err("C:/evil.txt");
        err("c:");
        err("a/CON");
        err("con.txt");
        err("a/NUL.txt");
        err("a/aux");
        err("a/b.");
        err("a/b ");
    }

    #[test]
    fn rejects_overlong_components() {
        let long = "x".repeat(256);
        err(&format!("a/{long}"));
        // 255 is fine.
        ok(&format!("a/{}", "x".repeat(255)));
    }

    #[test]
    fn rejects_nul_bytes() {
        err("a\0b");
    }
}
