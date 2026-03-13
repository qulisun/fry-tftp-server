use egui::{RichText, ScrollArea, Ui};

use crate::gui::log_layer::{LogBuffer, LogEntry};
use crate::gui::theme::Theme;

pub struct LogState {
    pub entries: Vec<LogEntry>,
    pub filter_level: tracing::Level,
    pub filter_text: String,
    pub auto_scroll: bool,
    max_entries: usize,
    pub copy_status: String,
}

impl Default for LogState {
    fn default() -> Self {
        Self::new()
    }
}

impl LogState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            filter_level: tracing::Level::TRACE,
            filter_text: String::new(),
            auto_scroll: true,
            max_entries: 10_000,
            copy_status: String::new(),
        }
    }

    pub fn update(&mut self, buffer: &LogBuffer) {
        if let Ok(mut buf) = buffer.lock() {
            while let Some(entry) = buf.pop_front() {
                self.entries.push(entry);
            }
            if self.entries.len() > self.max_entries {
                let excess = self.entries.len() - self.max_entries;
                self.entries.drain(0..excess);
            }
        }
    }
}

fn format_timestamp(ts: std::time::SystemTime) -> String {
    let dur = ts.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let millis = dur.subsec_millis();
    format!("{:02}:{:02}:{:02}.{:03}", hours, mins, secs, millis)
}

fn format_entry(entry: &LogEntry) -> String {
    let ts = format_timestamp(entry.timestamp);
    format!(
        "{} {:5} [{}] {}",
        ts, entry.level, entry.target, entry.message
    )
}

fn filtered_text(entries: &[LogEntry], filter_level: tracing::Level, filter_text: &str) -> String {
    entries
        .iter()
        .filter(|e| {
            e.level <= filter_level
                && (filter_text.is_empty()
                    || e.message.contains(filter_text)
                    || e.target.contains(filter_text))
        })
        .map(format_entry)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn draw(ui: &mut Ui, log_state: &mut LogState, buffer: &LogBuffer, theme: &Theme) {
    log_state.update(buffer);

    ui.horizontal(|ui| {
        ui.heading("Log");

        ui.label("Level:");
        egui::ComboBox::from_id_salt("log_level_filter")
            .selected_text(format!("{}", log_state.filter_level))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut log_state.filter_level, tracing::Level::TRACE, "TRACE");
                ui.selectable_value(&mut log_state.filter_level, tracing::Level::DEBUG, "DEBUG");
                ui.selectable_value(&mut log_state.filter_level, tracing::Level::INFO, "INFO");
                ui.selectable_value(&mut log_state.filter_level, tracing::Level::WARN, "WARN");
                ui.selectable_value(&mut log_state.filter_level, tracing::Level::ERROR, "ERROR");
            });

        ui.label("Filter:");
        ui.add(egui::TextEdit::singleline(&mut log_state.filter_text).desired_width(150.0));

        ui.checkbox(&mut log_state.auto_scroll, "Auto-scroll");

        if ui.button("Clear").clicked() {
            log_state.entries.clear();
            log_state.copy_status.clear();
        }

        if ui.button("Copy All").clicked() {
            let text = filtered_text(
                &log_state.entries,
                log_state.filter_level,
                &log_state.filter_text,
            );
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                Ok(_) => log_state.copy_status = "Copied to clipboard".to_string(),
                Err(e) => log_state.copy_status = format!("Copy failed: {}", e),
            }
        }

        if ui.button("Export").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Export logs")
                .add_filter("Log", &["log", "txt"])
                .set_file_name("server.log")
                .save_file()
            {
                let text = filtered_text(
                    &log_state.entries,
                    log_state.filter_level,
                    &log_state.filter_text,
                );
                match std::fs::write(&path, text) {
                    Ok(_) => log_state.copy_status = format!("Exported to {}", path.display()),
                    Err(e) => log_state.copy_status = format!("Export failed: {}", e),
                }
            }
        }

        if !log_state.copy_status.is_empty() {
            ui.label(&log_state.copy_status);
        }
    });

    ui.separator();

    let filtered: Vec<&LogEntry> = log_state
        .entries
        .iter()
        .filter(|e| {
            e.level <= log_state.filter_level
                && (log_state.filter_text.is_empty()
                    || e.message.contains(&log_state.filter_text)
                    || e.target.contains(&log_state.filter_text))
        })
        .collect();

    let scroll = ScrollArea::vertical().auto_shrink([false; 2]);
    let scroll = if log_state.auto_scroll {
        scroll.stick_to_bottom(true)
    } else {
        scroll
    };

    scroll.show(ui, |ui| {
        for entry in &filtered {
            let color = theme.log_color(&entry.level);
            let text = format_entry(entry);
            ui.label(RichText::new(text).color(color).monospace().size(12.0));
        }
    });
}
