//! Command implementations. Each module holds one clap `Args` struct plus a
//! `run` function; the commands only orchestrate I/O and output formatting.

pub mod compress;
pub mod extract;
pub mod info;
pub mod list;
mod progress;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Compress a file or directory into a .mcz archive
    Compress(compress::CompressArgs),
    /// Extract an archive
    Extract(extract::ExtractArgs),
    /// List the contents of an archive
    List(list::ListArgs),
    /// Show information about an archive
    Info(info::InfoArgs),
}

/// Formats a number with thousands separators, e.g. `1,234,567`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Human-readable compression ratio, e.g. `23.4%` (or `n/a` for empty input).
pub fn ratio(compressed: u64, uncompressed: u64) -> String {
    if uncompressed == 0 {
        return "n/a".to_string();
    }
    format!("{:.1}%", compressed as f64 / uncompressed as f64 * 100.0)
}
