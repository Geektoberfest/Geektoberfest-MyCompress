use std::path::PathBuf;

use clap::Args;
use mycompress_core::{Error, Registry};

use super::{format_duration, ratio, thousands};

/// Minimum number of iterations for results to be considered statistically reliable.
const MIN_RELIABLE_ITERATIONS: u32 = 5;

/// Minimum uncompressed file size (64 KiB) for timing results to be considered reliable.
const MIN_RELIABLE_INPUT_SIZE: u64 = 64 * 1024;

#[derive(Args, Debug)]
pub struct BenchmarkArgs {
    /// Input file(s) to benchmark
    #[arg(required = true, value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Number of benchmark iterations per algorithm
    #[arg(
        short = 'n',
        long,
        default_value_t = 5,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub iterations: u32,
}

struct TableRow {
    algorithm: &'static str,
    output: String,
    ratio: String,
    compress: String,
    decompress: String,
}

pub fn run(args: &BenchmarkArgs, registry: &Registry) -> Result<(), Error> {
    if args.iterations == 0 {
        return Err(Error::InvalidInput(
            "iterations must be greater than zero".to_string(),
        ));
    }
    if args.files.is_empty() {
        return Err(Error::InvalidInput(
            "at least one file must be specified".to_string(),
        ));
    }

    for (idx, file) in args.files.iter().enumerate() {
        if idx > 0 {
            println!();
        }

        let metadata = std::fs::metadata(file).map_err(|e| {
            Error::InvalidInput(format!("cannot read input '{}': {e}", file.display()))
        })?;
        if metadata.is_dir() {
            return Err(Error::InvalidInput(format!(
                "directories are not supported: '{}'",
                file.display()
            )));
        }
        if !metadata.is_file() {
            return Err(Error::UnsupportedEntry(file.to_path_buf()));
        }

        let data = std::fs::read(file).map_err(|e| {
            Error::InvalidInput(format!("cannot read input '{}': {e}", file.display()))
        })?;

        let mut rows = Vec::new();
        for name in registry.names() {
            let algorithm = registry.get_by_name(name).ok_or_else(|| {
                Error::InvalidInput(format!("algorithm '{name}' is not registered"))
            })?;
            let result = mycompress_core::bench::run(algorithm.as_ref(), &data, args.iterations)?;
            rows.push(result);
        }

        println!(
            "{} ({} bytes)",
            file.display(),
            thousands(data.len() as u64)
        );

        let table_rows: Vec<TableRow> = rows
            .iter()
            .map(|r| TableRow {
                algorithm: r.algorithm,
                output: format!("{} bytes", thousands(r.output_size)),
                ratio: ratio(r.output_size, r.input_size),
                compress: format_duration(r.compress),
                decompress: format_duration(r.decompress),
            })
            .collect();

        let mut w_algo = "Algorithm".len();
        let mut w_output = "Output".len();
        let mut w_ratio = "Ratio".len();
        let mut w_compress = "Compress".len();
        let mut w_decompress = "Decompress".len();

        for r in &table_rows {
            w_algo = w_algo.max(r.algorithm.len());
            w_output = w_output.max(r.output.len());
            w_ratio = w_ratio.max(r.ratio.len());
            w_compress = w_compress.max(r.compress.len());
            w_decompress = w_decompress.max(r.decompress.len());
        }

        println!(
            "{:<w_algo$}  {:>w_output$}  {:>w_ratio$}  {:>w_compress$}  {:>w_decompress$}",
            "Algorithm", "Output", "Ratio", "Compress", "Decompress"
        );
        for r in &table_rows {
            println!(
                "{:<w_algo$}  {:>w_output$}  {:>w_ratio$}  {:>w_compress$}  {:>w_decompress$}",
                r.algorithm, r.output, r.ratio, r.compress, r.decompress
            );
        }

        println!(
            "Timings are the median of {} run(s) on this machine.",
            args.iterations
        );
        if args.iterations < MIN_RELIABLE_ITERATIONS
            || (data.len() as u64) < MIN_RELIABLE_INPUT_SIZE
        {
            println!("Results are indicative only and not statistically meaningful.");
        }
    }

    Ok(())
}
