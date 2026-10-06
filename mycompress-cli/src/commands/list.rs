use std::path::PathBuf;

use clap::Args;
use mycompress_core::archive::{ArchiveReader, EntryKind};
use mycompress_core::{Error, Registry};

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Archive to inspect
    archive: PathBuf,
}

pub fn run(args: &ListArgs, registry: &Registry) -> Result<(), Error> {
    let reader = ArchiveReader::open(&args.archive, registry)?;
    println!("Archive: {}", args.archive.display());
    println!("{:<4}  {:>12}  {:>12}  Path", "Type", "Size", "Packed");
    for entry in reader.entries() {
        match entry.kind {
            EntryKind::Directory => {
                println!("{:<4}  {:>12}  {:>12}  {}", "dir", "-", "-", entry.path)
            }
            EntryKind::File => println!(
                "{:<4}  {:>12}  {:>12}  {}",
                "file", entry.uncompressed_size, entry.compressed_size, entry.path
            ),
        }
    }
    Ok(())
}
