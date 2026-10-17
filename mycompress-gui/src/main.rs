//! `mycompress-gui` — graphical interface for the MyCompress archiver.
//!
//! The GUI is a thin layer: all compression, archive, and filesystem logic
//! lives in `mycompress-core`. Adding a new compression algorithm to the core
//! registry makes it available here without any GUI changes.
//!
//! The window follows the familiar archiver layout: a toolbar across the top
//! (Open / Add / Extract / Test / Refresh), an address bar with the archive
//! path, a folder tree on the left, the archive contents in the middle, and a
//! status bar at the bottom.

// Don't pop up a console window alongside the GUI in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui::{
    self, Align, Color32, ComboBox, Context, Grid, Key, Layout, RichText, Rounding, ScrollArea,
    Sense, SidePanel, Stroke, TopBottomPanel, Ui, Vec2,
};
use eframe::{App, NativeOptions};
use mycompress_core::files::{compress, extract, CompressOptions, ExtractOptions};
use mycompress_core::{AlgorithmId, ArchiveReader, EntryKind, EntryMetadata, Registry};

/// Failures.
const ERROR: Color32 = Color32::from_rgb(0xc0, 0x39, 0x2b);
/// Secondary/hint text.
const MUTED: Color32 = Color32::from_rgb(0x6b, 0x70, 0x78);
/// Highlight for the selected row / folder.
const ACCENT: Color32 = Color32::from_rgb(0x2d, 0x5a, 0x9e);
/// Background of the window chrome (toolbar, folder panel, status bar).
const CHROME: Color32 = Color32::from_rgb(0xef, 0xef, 0xef);
/// Background of a resting button.
const BUTTON: Color32 = Color32::from_rgb(0xf8, 0xf8, 0xf8);
/// Background of the column-header row.
const HEADER: Color32 = Color32::from_rgb(0xdf, 0xdf, 0xdf);
/// Background of every other file row.
const STRIPE: Color32 = Color32::from_rgb(0xf6, 0xf6, 0xf6);
/// Thin panel borders.
const BORDER: Color32 = Color32::from_rgb(0xc4, 0xc4, 0xc4);

/// One row of the file table, with everything the table needs to render.
struct Row {
    /// Index of the entry in [`Gui::entries`].
    index: usize,
    /// Name shown in the table (relative to the selected folder).
    name: String,
    kind: EntryKind,
    size: u64,
    packed: u64,
    mtime: u64,
}

/// The state the GUI needs while it runs.
struct Gui {
    /// Path typed into the address bar.
    archive: String,
    /// Whether an archive is currently open.
    loaded: bool,
    /// Metadata of the open archive's entries; empty when nothing is open.
    entries: Vec<EntryMetadata>,
    /// Folder selected in the left panel; `None` means "All files".
    folder: Option<String>,
    /// Index into [`Gui::entries`] of the selected row, if any.
    selected: Option<usize>,
    /// Algorithm used by "Add".
    algorithm: AlgorithmId,
    /// Whether existing files may be overwritten.
    force: bool,
    /// The last failure, if any.
    error: Option<String>,
    /// The last successful action, if any.
    status: Option<String>,
}

impl Gui {
    fn new() -> Self {
        Self {
            archive: String::new(),
            loaded: false,
            entries: Vec::new(),
            folder: None,
            selected: None,
            algorithm: AlgorithmId::Rle,
            force: false,
            error: None,
            status: None,
        }
    }

    /// (Re)opens the archive named in the address bar.
    fn reload(&mut self) {
        let Some(path) = typed_path(&self.archive) else {
            self.loaded = false;
            self.entries.clear();
            self.folder = None;
            self.selected = None;
            self.error = Some("type or browse for an archive first.".to_string());
            return;
        };

        match ArchiveReader::open(&path, &Registry::new()) {
            Ok(reader) => {
                self.entries = reader.entries().to_vec();
                self.folder = None;
                self.selected = None;
                self.loaded = true;
                self.error = None;
                self.status = Some(format!(
                    "Opened {} ({} entries).",
                    path.display(),
                    self.entries.len()
                ));
            }
            Err(error) => {
                self.loaded = false;
                self.entries.clear();
                self.folder = None;
                self.selected = None;
                self.error = Some(error.to_string());
                self.status = None;
            }
        }
    }

    /// Asks for an archive to open via the file picker.
    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open archive")
            .add_filter("MyCompress archive", &["mcz"])
            .pick_file()
        {
            self.archive = path.display().to_string();
            self.reload();
        }
    }

    /// Compresses a chosen file or folder into a new archive.
    fn add(&mut self, folder: bool) {
        let dialog = rfd::FileDialog::new().set_title("Select what to compress");
        let chosen = if folder {
            dialog.pick_folder()
        } else {
            dialog.pick_file()
        };
        let Some(input) = chosen else {
            return;
        };

        let default_name = input
            .file_name()
            .map(|name| format!("{}.mcz", name.to_string_lossy()))
            .unwrap_or_else(|| "archive.mcz".to_string());
        let Some(output) = rfd::FileDialog::new()
            .set_title("Save archive")
            .set_file_name(&default_name)
            .add_filter("MyCompress archive", &["mcz"])
            .save_file()
        else {
            return;
        };

        let options = CompressOptions {
            algorithm: self.algorithm,
            force: self.force,
        };
        match compress(&input, &output, &options, &Registry::new()) {
            Ok(stats) => {
                self.error = None;
                self.status = Some(format!(
                    "Added {} files in {} folders ({}) to {}.",
                    stats.file_count,
                    stats.dir_count,
                    human_bytes(stats.archive_size),
                    output.display()
                ));
                // If the user just wrote over the open archive, refresh it.
                if output.display().to_string() == self.archive {
                    self.reload();
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.status = None;
            }
        }
    }

    /// Extracts the open archive into a chosen folder.
    fn run_extract(&mut self) {
        let Some(source) = typed_path(&self.archive) else {
            self.error = Some("open an archive first.".to_string());
            return;
        };
        let Some(destination) = rfd::FileDialog::new()
            .set_title("Extract to folder")
            .pick_folder()
        else {
            return;
        };

        let options = ExtractOptions { force: self.force };
        match extract(&source, &destination, &options, &Registry::new()) {
            Ok(stats) => {
                self.error = None;
                self.status = Some(format!(
                    "Extracted {} files in {} folders ({}) to {}.",
                    stats.files_written,
                    stats.dirs_created,
                    human_bytes(stats.bytes_written),
                    destination.display()
                ));
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.status = None;
            }
        }
    }

    /// Decompresses and verifies every file in the open archive.
    fn run_test(&mut self) {
        let Some(path) = typed_path(&self.archive) else {
            self.error = Some("open an archive first.".to_string());
            return;
        };

        match ArchiveReader::open(&path, &Registry::new()) {
            Ok(mut reader) => {
                let mut tested = 0u32;
                for index in 0..reader.entry_count() {
                    if reader.entry(index).kind != EntryKind::File {
                        continue;
                    }
                    if let Err(error) = reader.read_entry_data(index) {
                        self.error = Some(error.to_string());
                        self.status = None;
                        return;
                    }
                    tested += 1;
                }
                self.error = None;
                self.status = Some(format!("Tested {tested} files: no errors."));
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.status = None;
            }
        }
    }

    /// Every directory that appears in the archive, sorted by path.
    fn folders(&self) -> Vec<String> {
        let mut dirs = BTreeSet::new();
        for entry in &self.entries {
            if entry.kind == EntryKind::Directory {
                dirs.insert(entry.path.clone());
            }
            // Also record the ancestors of every entry, in case the archive
            // does not store its directories explicitly.
            let mut path = entry.path.as_str();
            while let Some(index) = path.rfind('/') {
                path = &path[..index];
                if !path.is_empty() {
                    dirs.insert(path.to_string());
                }
            }
        }
        dirs.into_iter().collect()
    }

    /// The rows to show for the folder currently selected in the left panel.
    fn visible(&self) -> Vec<Row> {
        let prefix = self.folder.as_ref().map(|folder| format!("{folder}/"));
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| match &prefix {
                Some(prefix) => entry.path.starts_with(prefix.as_str()),
                None => true,
            })
            .map(|(index, entry)| {
                let name = match &prefix {
                    Some(prefix) => entry
                        .path
                        .strip_prefix(prefix.as_str())
                        .unwrap_or(&entry.path),
                    None => entry.path.as_str(),
                };
                Row {
                    index,
                    name: name.to_string(),
                    kind: entry.kind,
                    size: entry.uncompressed_size,
                    packed: entry.compressed_size,
                    mtime: entry.mtime,
                }
            })
            .collect()
    }

    /// Totals for the status bar: (files, folders, unpacked, packed).
    fn totals(&self) -> (usize, usize, String, String) {
        let mut files = 0;
        let mut folders = 0;
        let mut unpacked = 0u64;
        let mut packed = 0u64;
        for entry in &self.entries {
            match entry.kind {
                EntryKind::File => {
                    files += 1;
                    unpacked += entry.uncompressed_size;
                    packed += entry.compressed_size;
                }
                EntryKind::Directory => folders += 1,
            }
        }
        (files, folders, human_bytes(unpacked), human_bytes(packed))
    }

    /// The toolbar row: the main actions plus the archive options.
    fn show_toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open").clicked() {
                self.open_dialog();
            }
            ui.menu_button("Add", |ui| {
                if ui.button("File...").clicked() {
                    ui.close_menu();
                    self.add(false);
                }
                if ui.button("Folder...").clicked() {
                    ui.close_menu();
                    self.add(true);
                }
            });
            if ui
                .add_enabled(self.loaded, egui::Button::new("Extract"))
                .clicked()
            {
                self.run_extract();
            }
            if ui
                .add_enabled(self.loaded, egui::Button::new("Test"))
                .clicked()
            {
                self.run_test();
            }
            if ui
                .add_enabled(self.loaded, egui::Button::new("Refresh"))
                .clicked()
            {
                self.reload();
            }

            ui.separator();
            ui.checkbox(&mut self.force, "Overwrite");
            ui.label("Algorithm:");
            ComboBox::from_id_source("algorithm")
                .selected_text(self.algorithm.to_string())
                .width(90.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.algorithm, AlgorithmId::Rle, "rle");
                    ui.selectable_value(&mut self.algorithm, AlgorithmId::Huffman, "huffman");
                });
        });
    }

    /// The address bar showing (and editing) the open archive's path.
    fn show_address_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Archive:").strong());
            let width = ui.available_width();
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.archive)
                    .hint_text("path to a .mcz archive, then press Enter")
                    .desired_width(width),
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                self.reload();
            }
        });
    }

    /// The left panel listing the archive's folders.
    fn show_folder_panel(&mut self, ui: &mut Ui) {
        ui.add_space(2.0);
        ui.label(RichText::new("Folders").strong());
        ui.separator();

        let folders = self.folders();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ui
                    .selectable_label(self.folder.is_none(), "All files")
                    .clicked()
                {
                    self.folder = None;
                    self.selected = None;
                }
                for folder in &folders {
                    let depth = folder.matches('/').count();
                    let name = folder.rsplit('/').next().unwrap_or(folder.as_str());
                    let label = format!("{}{}", "    ".repeat(depth), name);
                    let selected = self.folder.as_deref() == Some(folder.as_str());
                    if ui.selectable_label(selected, label).clicked() {
                        self.folder = Some(folder.clone());
                        self.selected = None;
                    }
                }
            });
    }

    /// The middle panel: the archive contents table.
    fn show_file_table(&mut self, ui: &mut Ui) {
        if !self.loaded {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Open an .mcz archive to view its contents.").color(MUTED));
            });
            return;
        }

        let rows = self.visible();
        if rows.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("This folder is empty.").color(MUTED));
            return;
        }

        // Row 0 of the grid is the header; data rows follow in order. Give the
        // colour callback what it needs without borrowing `self`.
        let selected = self.selected;
        let entry_of_row: Vec<usize> = rows.iter().map(|row| row.index).collect();

        let mut clicked = None;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Grid::new("file-table")
                    .num_columns(5)
                    .min_col_width(70.0)
                    .min_row_height(20.0)
                    .spacing(Vec2::new(12.0, 4.0))
                    .with_row_color(move |row, style| {
                        if row == 0 {
                            return Some(HEADER);
                        }
                        match entry_of_row.get(row - 1) {
                            Some(&entry) if Some(entry) == selected => Some(ACCENT),
                            _ if row % 2 == 1 => Some(style.visuals.faint_bg_color),
                            _ => None,
                        }
                    })
                    .show(ui, |ui| {
                        header_label(ui, "Name");
                        right_header(ui, "Size");
                        right_header(ui, "Packed");
                        header_label(ui, "Modified");
                        fill_cell(ui);
                        ui.end_row();

                        for row in &rows {
                            let is_selected = self.selected == Some(row.index);
                            // White text reads well on the blue selection bar.
                            let text_color = if is_selected { Color32::WHITE } else { MUTED };
                            let name = match row.kind {
                                EntryKind::Directory => format!("{}/", row.name),
                                EntryKind::File => row.name.clone(),
                            };
                            let name_color = if is_selected {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(0x1f, 0x23, 0x28)
                            };
                            let response = ui.add(
                                egui::Label::new(RichText::new(name).monospace().color(name_color))
                                    .sense(Sense::click()),
                            );
                            if response.clicked() {
                                clicked = Some(row.index);
                            }

                            match row.kind {
                                EntryKind::Directory => {
                                    right_label(ui, "", text_color);
                                    right_label(ui, "", text_color);
                                }
                                EntryKind::File => {
                                    right_label(ui, &human_bytes(row.size), text_color);
                                    right_label(ui, &human_bytes(row.packed), text_color);
                                }
                            }
                            ui.label(
                                RichText::new(format_time(row.mtime))
                                    .monospace()
                                    .color(text_color),
                            );
                            fill_cell(ui);
                            ui.end_row();
                        }
                    });
            });

        if let Some(index) = clicked {
            self.selected = Some(index);
        }
    }

    /// The status bar at the bottom of the window.
    fn show_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(error) = &self.error {
                ui.label(RichText::new(format!("Error: {error}")).color(ERROR));
            } else if let Some(status) = &self.status {
                ui.label(RichText::new(status.as_str()).color(MUTED));
            } else {
                ui.label(RichText::new("Ready.").color(MUTED));
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.loaded {
                    let (files, folders, unpacked, packed) = self.totals();
                    ui.label(
                        RichText::new(format!(
                            "{files} files, {folders} folders    {unpacked} -> {packed}"
                        ))
                        .color(MUTED),
                    );
                }
            });
        });
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

/// A right-aligned table cell.
fn right_label(ui: &mut Ui, text: &str, color: Color32) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(text).monospace().color(color));
    });
}

/// A left-aligned, bold column header.
fn header_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

/// A right-aligned, bold column header.
fn right_header(ui: &mut Ui, text: &str) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(text).strong());
    });
}

/// An empty cell that stretches to the end of the row so the table fills the
/// panel (and its background stripes cover the full width).
fn fill_cell(ui: &mut Ui) {
    ui.allocate_space(Vec2::new(ui.available_width(), 0.0));
}

/// Applies the app's palette, spacing, and rounding to the default style.
fn configure_style(ctx: &Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();

    // Classic archiver look: grey chrome around a white file list.
    style.visuals.panel_fill = CHROME;
    style.visuals.window_fill = CHROME;
    style.visuals.extreme_bg_color = Color32::WHITE;
    style.visuals.faint_bg_color = STRIPE;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);

    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.interact_size.y = 26.0;

    let rounding = Rounding::same(4.0);
    style.visuals.widgets.noninteractive.rounding = rounding;
    style.visuals.widgets.inactive.rounding = rounding;
    style.visuals.widgets.hovered.rounding = rounding;
    style.visuals.widgets.active.rounding = rounding;
    style.visuals.widgets.open.rounding = rounding;

    // egui's light theme draws buttons in the same grey as the panel with no
    // border, so a toolbar of them would be invisible. Give buttons a raised
    // body, a border, and obvious hover/press states.
    style.visuals.widgets.inactive.weak_bg_fill = BUTTON;
    style.visuals.widgets.inactive.bg_fill = BUTTON;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.fg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgb(0x28, 0x2c, 0x31));

    style.visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.hovered.bg_fill = Color32::WHITE;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.hovered.fg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgb(0x10, 0x14, 0x18));

    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0xd6, 0xdd, 0xe8);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(0xd6, 0xdd, 0xe8);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.fg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgb(0x10, 0x14, 0x18));

    ctx.set_style(style);
}

/// Formats a byte count using binary (1024-based) units.
fn human_bytes(bytes: u64) -> String {
    const UNIT: f64 = 1024.0;
    const NAMES: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if (bytes as f64) < UNIT {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= UNIT && unit + 1 < NAMES.len() {
        value /= UNIT;
        unit += 1;
    }
    format!("{value:.1} {}", NAMES[unit])
}

/// Formats a Unix timestamp as `YYYY-MM-DD HH:MM`, or `-` when unspecified.
fn format_time(secs: u64) -> String {
    if secs == 0 {
        return "-".to_string();
    }
    let days = (secs / 86_400) as i64;
    let time = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        time / 3600,
        (time % 3600) / 60
    )
}

/// Converts days since the Unix epoch to a civil `(year, month, day)`.
///
/// This is Howard Hinnant's `civil_from_days` algorithm, which is valid for
/// the entire range of `i64` days.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

impl App for Gui {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.show_toolbar(ui);
            ui.add_space(4.0);
            self.show_address_bar(ui);
            ui.add_space(4.0);
        });

        TopBottomPanel::bottom("status-bar").show(ctx, |ui| {
            ui.add_space(3.0);
            self.show_status_bar(ui);
            ui.add_space(3.0);
        });

        SidePanel::left("folder-panel")
            .resizable(true)
            .default_width(220.0)
            .min_width(150.0)
            .max_width(420.0)
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .fill(CHROME)
                    .stroke(Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| self.show_folder_panel(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::same(4.0)),
            )
            .show(ctx, |ui| self.show_file_table(ui));
    }
}

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1020.0, 680.0])
            .with_min_inner_size([720.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MyCompress",
        options,
        Box::new(|cc| {
            configure_style(&cc.egui_ctx);
            let mut gui = Gui::new();
            // Like 7-Zip, open the archive named on the command line, if any.
            if let Some(path) = std::env::args_os().nth(1) {
                gui.archive = PathBuf::from(path).display().to_string();
                gui.reload();
            }
            Box::new(gui)
        }),
    )
}
