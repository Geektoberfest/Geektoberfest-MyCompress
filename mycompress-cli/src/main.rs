//! `mycompress` — command-line interface for the MyCompress archiver.
//!
//! The CLI is a thin layer: all compression, archive, and filesystem logic
//! lives in `mycompress-core`. Adding a new compression algorithm to the core
//! registry makes it available here without any CLI changes.

mod commands;

use std::error::Error as _;

use clap::Parser;
use mycompress_core::Registry;

use crate::commands::Command;

#[derive(Parser, Debug)]
#[command(
    name = "mycompress",
    version,
    propagate_version = true,
    about = "Cross-platform file compressor (algorithms: rle, archives: .mcz)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn main() {
    let cli = Cli::parse();
    let registry = Registry::new();

    let result = match cli.command {
        Command::Benchmark(args) => commands::benchmark::run(&args, &registry),
        Command::Compress(args) => commands::compress::run(&args, &registry),
        Command::Extract(args) => commands::extract::run(&args, &registry),
        Command::List(args) => commands::list::run(&args, &registry),
        Command::Info(args) => commands::info::run(&args, &registry),
    };

    if let Err(error) = result {
        print_error(&error);
        std::process::exit(1);
    }
}

/// Prints an error and its full cause chain to stderr.
fn print_error(error: &mycompress_core::Error) {
    eprintln!("error: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}
