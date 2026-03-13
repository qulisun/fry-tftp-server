use egui::Ui;
use std::path::PathBuf;
use std::sync::Arc;

use crate::core::state::AppState;

pub struct FilesState {
    current_root: PathBuf,
    entries: Vec<DirEntry>,
    selected: Option<usize>,
    needs_refresh: bool,
}

struct DirEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

impl FilesState {
    pub fn new(root: PathBuf) -> Self {
        let mut s = Self {
            current_root: root,
            entries: Vec::new(),
            selected: None,
            needs_refresh: true,
        };
        s.refresh();
        s
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        if let Ok(rd) = std::fs::read_dir(&self.current_root) {
            let mut entries: Vec<DirEntry> = rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    let meta = e.metadata().ok();
                    DirEntry {
                        name: e.file_name().to_string_lossy().to_string(),
                        path: e.path(),
                        is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
                        size: meta.as_ref().map_or(0, |m| m.len()),
                    }
                })
                .collect();
            entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
            self.entries = entries;
        }
        self.needs_refresh = false;
    }
}

fn format_size(size: u64) -> String {
    if size >= 1_000_000_000 {
        format!("{:.1} GB", size as f64 / 1_000_000_000.0)
    } else if size >= 1_000_000 {
        format!("{:.1} MB", size as f64 / 1_000_000.0)
    } else if size >= 1_000 {
        format!("{:.1} KB", size as f64 / 1_000.0)
    } else {
        format!("{} B", size)
    }
}

pub fn draw(ui: &mut Ui, _state: &Arc<AppState>, files: &mut FilesState) {
    ui.horizontal(|ui| {
        ui.heading("Files");
        if ui.button("Refresh").clicked() {
            files.refresh();
        }
        if ui.button("Change Root...").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&files.current_root)
                .pick_folder()
            {
                files.current_root = path;
                files.refresh();
            }
        }
        if files.current_root.parent().is_some() && ui.button("Up").clicked() {
            if let Some(parent) = files.current_root.parent() {
                files.current_root = parent.to_path_buf();
                files.needs_refresh = true;
            }
        }
    });

    ui.separator();
    ui.label(format!("Path: {}", files.current_root.display()));
    ui.add_space(4.0);

    if files.needs_refresh {
        files.refresh();
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("files_grid")
            .striped(true)
            .min_col_width(100.0)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Size");
                ui.strong("Type");
                ui.end_row();

                for (i, entry) in files.entries.iter().enumerate() {
                    let label = if entry.is_dir {
                        format!("[DIR] {}", entry.name)
                    } else {
                        entry.name.clone()
                    };

                    let selected = files.selected == Some(i);
                    if ui.selectable_label(selected, &label).clicked() {
                        if entry.is_dir {
                            files.current_root = entry.path.clone();
                            files.needs_refresh = true;
                        } else {
                            files.selected = Some(i);
                        }
                    }

                    if entry.is_dir {
                        ui.label("-");
                        ui.label("Directory");
                    } else {
                        ui.label(format_size(entry.size));
                        ui.label("File");
                    }
                    ui.end_row();
                }
            });
    });
}
