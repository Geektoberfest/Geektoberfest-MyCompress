//! Command implementations. Each module holds one clap `Args` struct plus a
//! `run` function; the commands only orchestrate I/O and output formatting.

pub mod benchmark;
pub mod compress;
pub mod extract;
pub mod info;
pub mod list;

use std::time::Duration;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Benchmark compression algorithms against input files
    Benchmark(benchmark::BenchmarkArgs),
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

/// Formats a duration compactly and consistently.
///
/// Durations >= 1s are displayed in seconds with 3 decimals (e.g. `1.234 s`).
/// Durations >= 1ms are displayed in milliseconds with 3 decimals (e.g. `1.234 ms`).
/// Durations < 1ms are displayed in microseconds with 3 decimals (e.g. `123.456 µs`).
pub fn format_duration(d: Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{:.3} s", d.as_secs_f64())
    } else if d >= Duration::from_millis(1) {
        format!("{:.3} ms", d.as_secs_f64() * 1_000.0)
    } else {
        format!("{:.3} µs", d.as_secs_f64() * 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_formatting() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn ratio_formatting() {
        assert_eq!(ratio(0, 0), "n/a");
        assert_eq!(ratio(50, 100), "50.0%");
        assert_eq!(ratio(100, 50), "200.0%");
    }

    #[test]
    fn format_duration_compact() {
        assert_eq!(format_duration(Duration::from_nanos(0)), "0.000 µs");
        assert_eq!(format_duration(Duration::from_micros(45)), "45.000 µs");
        assert_eq!(format_duration(Duration::from_micros(999)), "999.000 µs");
        assert_eq!(format_duration(Duration::from_millis(1)), "1.000 ms");
        assert_eq!(format_duration(Duration::from_millis(125)), "125.000 ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.500 s");
    }
}
