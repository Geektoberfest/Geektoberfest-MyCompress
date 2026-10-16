use std::path::PathBuf;

use clap::Args;
use mycompress_core::files::{self, ExtractOptions};
use mycompress_core::{Error, Registry};

use super::{progress::Progress, thousands};

#[derive(Args, Debug)]
pub struct ExtractArgs {
    /// Archive to extract
    archive: PathBuf,

    /// Destination directory
    #[arg(short, long, default_value = ".")]
    dest: PathBuf,

    /// Overwrite existing files
    #[arg(short, long)]
    force: bool,
}

pub fn run(args: &ExtractArgs, registry: &Registry) -> Result<(), Error> {
    let options = ExtractOptions { force: args.force };
    let mut progress = Progress::new("Extracting", None, None);

    let result = files::extract_with_progress(
        &args.archive,
        &args.dest,
        &options,
        registry,
        |completed_entries, completed_bytes| {
            progress.update(completed_entries, completed_bytes);
        },
    );

    let stats = result?;
    progress.finish();

    println!(
        "extracted {} file(s), {} dir(s) to {}",
        stats.files_written,
        stats.dirs_created,
        args.dest.display()
    );
    println!("  {} bytes written", thousands(stats.bytes_written));

    Ok(())
}
