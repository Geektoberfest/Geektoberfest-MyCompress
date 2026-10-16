//! `mycompress-gui` — graphical interface for the MyCompress archiver.
//!
//! The GUI is a thin layer: all compression, archive, and filesystem logic
//! lives in `mycompress-core`. Adding a new compression algorithm to the core
//! registry makes it available here without any GUI changes.

use std::path::PathBuf;

use eframe::egui::{CentralPanel, ComboBox, Context, ScrollArea, SidePanel, Ui};
use eframe::{App, Frame, NativeOptions};
use mycompress_core::files::{
    compress, extract, CompressOptions, CompressStats, ExtractOptions, ExtractStats,
};
use mycompress_core::ArchiveReader;
use mycompress_core::{AlgorithmId, Registry};

/// The state the GUI needs while it runs.
struct Gui {
    /// Path typed into the "Input" field, empty when nothing was typed.
    input: String,
    /// Path typed into the "Output archive" field, empty when the default is used.
    output: String,
    /// Algorithm chosen in the "Algorithm" field.
    algorithm: AlgorithmId,
    /// Whether an existing archive may be overwritten.
    force_compress: bool,
    /// Path typed into the "Archive" field, empty when nothing was typed.
    archive: String,
    /// Path typed into the "Destination" field, empty to extract into `.`.
    destination: String,
    /// Whether existing files may be overwritten during extraction.
    force_extract: bool,
    /// Statistics of the last compression run.
    compress_stats: Option<CompressStats>,
    /// Statistics of the last extraction run.
    extract_stats: Option<ExtractStats>,
    /// Explanation of the last action, if one failed.
    error: Option<String>,
}

impl Gui {
    fn new() -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            algorithm: AlgorithmId::Rle,
            force_compress: false,
            archive: String::new(),
            destination: String::new(),
            force_extract: false,
            compress_stats: None,
            extract_stats: None,
            error: None,
        }
    }

    /// Runs the pending action in the "Compress" panel.
    fn run_compress(&mut self) {
        let Some(input) = typed_path(&self.input) else {
            self.error = Some("type a path in the Input field, or press Browse.".to_string());
            return;
        };
        let output = typed_path(&self.output).unwrap_or_else(|| {
            let mut path = input.clone();
            if let Some(name) = input.file_name() {
                path.set_file_name(format!("{}.mcz", name.to_string_lossy()));
            }
            path
        });
        let options = CompressOptions {
            algorithm: self.algorithm,
            force: self.force_compress,
        };

        match compress(&input, &output, &options, &Registry::new()) {
            Ok(stats) => {
                self.compress_stats = Some(stats);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("error: {error}")),
        }
    }

    /// Runs the pending action in the "Extract" panel.
    fn run_extract(&mut self) {
        let Some(archive) = typed_path(&self.archive) else {
            self.error = Some("type a path in the Archive field, or press Browse.".to_string());
            return;
        };
        let destination = typed_path(&self.destination).unwrap_or_else(|| PathBuf::from("."));
        let options = ExtractOptions {
            force: self.force_extract,
        };

        match extract(&archive, &destination, &options, &Registry::new()) {
            Ok(stats) => {
                self.extract_stats = Some(stats);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("error: {error}")),
        }
    }

    /// Shows everything the "Compress" panel needs.
    fn show_compress(&mut self, ui: &mut Ui) {
        ui.heading("Compress");
        ui.separator();

        ui.label("Input:");
        ui.text_edit_singleline(&mut self.input);
        ui.horizontal(|ui| {
            if ui.button("Browse file").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.input = path.display().to_string();
                }
            }
            if ui.button("Browse folder").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.input = path.display().to_string();
                }
            }
        });

        ui.label("Output archive:");
        ui.text_edit_singleline(&mut self.output);

        ui.label("Algorithm:");
        ComboBox::from_label("Algorithm")
            .selected_text(self.algorithm.to_string())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.algorithm, AlgorithmId::Rle, "rle");
                ui.selectable_value(&mut self.algorithm, AlgorithmId::Huffman, "huffman");
            });

        ui.checkbox(&mut self.force_compress, "Overwrite existing archive");
        if ui.button("Compress").clicked() {
            self.run_compress();
        }
        self.show_compress_stats(ui);
        self.show_error(ui);
    }

    /// Shows the statistics of the last compression run, if there was one.
    fn show_compress_stats(&mut self, ui: &mut Ui) {
        if let Some(stats) = &self.compress_stats {
            ui.label(format!(
                "{} files, {} directories, {} bytes uncompressed.",
                stats.file_count, stats.dir_count, stats.total_uncompressed
            ));
            ui.label(format!("Archive size: {} bytes.", stats.archive_size));
            for warning in &stats.warnings {
                ui.label(format!("warning: {warning}"));
            }
        }
    }

    /// Shows everything the "Extract" panel needs.
    fn show_extract(&mut self, ui: &mut Ui) {
        ui.heading("Extract");
        ui.separator();

        ui.label("Archive:");
        ui.text_edit_singleline(&mut self.archive);
        ui.horizontal(|ui| {
            if ui.button("Browse").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("MyCompress archive", &["mcz"])
                    .pick_file()
                {
                    self.archive = path.display().to_string();
                }
            }
        });

        ui.label("Destination:");
        ui.text_edit_singleline(&mut self.destination);
        ui.checkbox(&mut self.force_extract, "Overwrite existing files");
        if ui.button("Extract").clicked() {
            self.run_extract();
        }

        self.show_archive_contents(ui);
        if let Some(stats) = &self.extract_stats {
            ui.label(format!(
                "{} files, {} directories, {} bytes written.",
                stats.files_written, stats.dirs_created, stats.bytes_written
            ));
        }
        self.show_error(ui);
    }

    /// Lists the entries of the archive typed in the "Archive" field.
    fn show_archive_contents(&mut self, ui: &mut Ui) {
        let Some(archive) = typed_path(&self.archive) else {
            return;
        };
        let registry = Registry::new();
        match ArchiveReader::open(&archive, &registry) {
            Ok(reader) => {
                ui.label(format!("{} entries.", reader.entry_count()));
                ScrollArea::vertical().show(ui, |ui| {
                    for index in 0..reader.entry_count() {
                        let entry = reader.entry(index);
                        ui.label(format!(
                            "{}  ({} bytes)",
                            entry.path, entry.uncompressed_size
                        ));
                    }
                });
            }
            Err(error) => {
                ui.label(format!("cannot open this archive: {error}"));
            }
        }
    }

    fn show_error(&self, ui: &mut Ui) {
        if let Some(error) = &self.error {
            ui.label(format!("error: {error}"));
        }
    }
}

/// Returns the path typed into a text field, or `None` when the field is empty.
fn typed_path(field: &str) -> Option<PathBuf> {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

impl App for Gui {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        SidePanel::left("compress-panel")
            .default_width(430.0)
            .show(ctx, |ui| self.show_compress(ui));

        CentralPanel::default().show(ctx, |ui| self.show_extract(ui));
    }
}

fn main() -> eframe::Result<()> {
    let options = NativeOptions::default();
    eframe::run_native("MyCompress", options, Box::new(|_cc| Box::new(Gui::new())))
}
