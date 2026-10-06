use std::path::PathBuf;

use clap::Args;
use mycompress_core::archive::{ArchiveReader, EntryKind};
use mycompress_core::{Error, Registry};

use super::{ratio, thousands};

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Archive to inspect
    archive: PathBuf,
}

pub fn run(args: &InfoArgs, registry: &Registry) -> Result<(), Error> {
    let reader = ArchiveReader::open(&args.archive, registry)?;

    let entries = reader.entries();
    let files = entries.iter().filter(|e| e.kind == EntryKind::File).count();
    let dirs = entries.len() - files;
    let total_uncompressed: u64 = entries.iter().map(|e| e.uncompressed_size).sum();
    let total_compressed: u64 = entries.iter().map(|e| e.compressed_size).sum();

    println!("Archive:          {}", args.archive.display());
    println!("Format:           .mcz version {}", reader.format_version());
    println!(
        "Algorithm:        {} (id {})",
        reader.algorithm_name(),
        reader.algorithm_id()
    );
    println!(
        "Entries:          {} ({} file(s), {} dir(s))",
        entries.len(),
        files,
        dirs
    );
    println!("Uncompressed:     {} bytes", thousands(total_uncompressed));
    println!("Compressed data:  {} bytes", thousands(total_compressed));
    println!(
        "Ratio:            {}",
        ratio(total_compressed, total_uncompressed)
    );
    Ok(())
}
