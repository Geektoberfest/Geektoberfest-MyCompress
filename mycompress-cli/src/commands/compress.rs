use std::path::{Path, PathBuf};

use clap::Args;
use mycompress_core::files::{self, CompressOptions};
use mycompress_core::{Error, Registry};

use super::{ratio, thousands};

#[derive(Args, Debug)]
pub struct CompressArgs {
    /// File or directory to compress
    input: PathBuf,

    /// Output archive path [default: <input>.mcz]
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Compression algorithm
    #[arg(short, long, default_value = "rle")]
    algorithm: String,

    /// Overwrite the output archive if it already exists
    #[arg(short, long)]
    force: bool,
}

pub fn run(args: &CompressArgs, registry: &Registry) -> Result<(), Error> {
    let algorithm = registry
        .get_by_name(&args.algorithm)
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "unknown algorithm '{}' (available: {})",
                args.algorithm,
                registry.names().join(", ")
            ))
        })?
        .id();

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_output(&args.input));
    let options = CompressOptions {
        algorithm,
        force: args.force,
    };

    let stats = files::compress(&args.input, &output, &options, registry)?;

    for warning in &stats.warnings {
        eprintln!("warning: {warning}");
    }
    println!("created {}", output.display());
    println!("  {} file(s), {} dir(s)", stats.file_count, stats.dir_count);
    println!(
        "  {} bytes -> {} bytes ({})",
        thousands(stats.total_uncompressed),
        thousands(stats.archive_size),
        ratio(stats.archive_size, stats.total_uncompressed)
    );
    Ok(())
}

/// Appends `.mcz` to the input path (`docs` -> `docs.mcz`, `a.txt` -> `a.txt.mcz`).
fn default_output(input: &Path) -> PathBuf {
    let mut name = input.as_os_str().to_os_string();
    name.push(".mcz");
    PathBuf::from(name)
}
