use egui::Ui;

use crate::core::state::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Client,
    File,
    Direction,
    Size,
    Duration,
    Speed,
    Status,
    Retransmits,
}

pub struct TransfersState {
    pub filter_ip: String,
    pub filter_filename: String,
    pub filter_status: String,
    pub sort_column: SortColumn,
    pub sort_ascending: bool,
}

impl Default for TransfersState {
    fn default() -> Self {
        Self::new()
    }
}

impl TransfersState {
    pub fn new() -> Self {
        Self {
            filter_ip: String::new(),
            filter_filename: String::new(),
            filter_status: "all".to_string(),
            sort_column: SortColumn::Duration,
            sort_ascending: false,
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}

fn sort_header(
    ui: &mut Ui,
    label: &str,
    col: SortColumn,
    current: SortColumn,
    ascending: bool,
) -> bool {
    let arrow = if current == col {
        if ascending {
            " [A]"
        } else {
            " [D]"
        }
    } else {
        ""
    };
    let text = format!("{}{}", label, arrow);
    ui.strong(text).clicked()
}

fn export_json(records: &[&TransferRecord]) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Export transfers as JSON")
        .add_filter("JSON", &["json"])
        .set_file_name("transfers.json")
        .save_file()
    {
        let entries: Vec<serde_json::Value> = records
            .iter()
            .map(|r| {
                serde_json::json!({
                    "client": r.client_addr.to_string(),
                    "file": r.filename,
                    "direction": match r.direction {
                        Direction::Read => "Download",
                        Direction::Write => "Upload",
                    },
                    "bytes": r.bytes_transferred,
                    "duration_ms": r.duration_ms,
                    "speed_mbps": r.speed_mbps,
                    "status": match r.status {
                        SessionStatus::Completed => "Completed",
                        SessionStatus::Failed => "Failed",
                        SessionStatus::Cancelled => "Cancelled",
                        _ => "Unknown",
                    },
                    "retransmits": r.retransmits,
                    "elapsed_secs": r.timestamp.elapsed().as_secs(),
                })
            })
            .collect();
        match serde_json::to_string_pretty(&entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::error!(error=%e, "failed to export JSON");
                }
            }
            Err(e) => {
                tracing::error!(error=%e, "failed to serialize JSON");
            }
        }
    }
}

fn export_csv(records: &[&TransferRecord]) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Export transfers as CSV")
        .add_filter("CSV", &["csv"])
        .set_file_name("transfers.csv")
        .save_file()
    {
        let mut csv =
            String::from("Client,File,Direction,Bytes,Duration_ms,Speed_Mbps,Status,Retransmits\n");
        for r in records {
            let dir = match r.direction {
                Direction::Read => "Download",
                Direction::Write => "Upload",
            };
            let status = match r.status {
                SessionStatus::Completed => "Completed",
                SessionStatus::Failed => "Failed",
                SessionStatus::Cancelled => "Cancelled",
                _ => "Unknown",
            };
            csv.push_str(&format!(
                "{},{},{},{},{},{:.2},{},{}\n",
                r.client_addr,
                r.filename,
                dir,
                r.bytes_transferred,
                r.duration_ms,
                r.speed_mbps,
                status,
                r.retransmits
            ));
        }
        if let Err(e) = std::fs::write(&path, csv) {
            tracing::error!(error=%e, "failed to export CSV");
        }
    }
}

pub fn draw(ui: &mut Ui, history: &[TransferRecord], transfers: &mut TransfersState) {
    ui.heading("Transfer History");

    // Filters + Export
    ui.horizontal(|ui| {
        ui.label("IP:");
        ui.add(egui::TextEdit::singleline(&mut transfers.filter_ip).desired_width(120.0));
        ui.label("File:");
        ui.add(egui::TextEdit::singleline(&mut transfers.filter_filename).desired_width(120.0));
        ui.label("Status:");
        egui::ComboBox::from_id_salt("status_filter")
            .selected_text(&transfers.filter_status)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut transfers.filter_status, "all".to_string(), "All");
                ui.selectable_value(
                    &mut transfers.filter_status,
                    "completed".to_string(),
                    "Completed",
                );
                ui.selectable_value(&mut transfers.filter_status, "failed".to_string(), "Failed");
            });
    });

    ui.separator();

    let mut filtered: Vec<&TransferRecord> = history
        .iter()
        .filter(|r| {
            if !transfers.filter_ip.is_empty()
                && !r
                    .client_addr
                    .ip()
                    .to_string()
                    .contains(&transfers.filter_ip)
            {
                return false;
            }
            if !transfers.filter_filename.is_empty()
                && !r.filename.contains(&transfers.filter_filename)
            {
                return false;
            }
            match transfers.filter_status.as_str() {
                "completed" => r.status == SessionStatus::Completed,
                "failed" => r.status == SessionStatus::Failed,
                _ => true,
            }
        })
        .collect();

    // Sort
    let asc = transfers.sort_ascending;
    match transfers.sort_column {
        SortColumn::Client => filtered.sort_by(|a, b| {
            let c = a.client_addr.to_string().cmp(&b.client_addr.to_string());
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::File => filtered.sort_by(|a, b| {
            let c = a.filename.cmp(&b.filename);
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::Direction => filtered.sort_by(|a, b| {
            let c = (a.direction as u8).cmp(&(b.direction as u8));
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::Size => filtered.sort_by(|a, b| {
            let c = a.bytes_transferred.cmp(&b.bytes_transferred);
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::Duration => filtered.sort_by(|a, b| {
            let c = a.duration_ms.cmp(&b.duration_ms);
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::Speed => filtered.sort_by(|a, b| {
            let c = a
                .speed_mbps
                .partial_cmp(&b.speed_mbps)
                .unwrap_or(std::cmp::Ordering::Equal);
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::Status => filtered.sort_by(|a, b| {
            let c = (a.status as u8).cmp(&(b.status as u8));
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
        SortColumn::Retransmits => filtered.sort_by(|a, b| {
            let c = a.retransmits.cmp(&b.retransmits);
            if asc {
                c
            } else {
                c.reverse()
            }
        }),
    }

    ui.horizontal(|ui| {
        ui.label(format!("{} records", filtered.len()));
        if ui.button("Export CSV").clicked() {
            export_csv(&filtered);
        }
        if ui.button("Export JSON").clicked() {
            export_json(&filtered);
        }
    });

    egui::ScrollArea::both().show(ui, |ui| {
        egui::Grid::new("transfers_grid")
            .striped(true)
            .min_col_width(80.0)
            .show(ui, |ui| {
                let cols = [
                    ("Client", SortColumn::Client),
                    ("File", SortColumn::File),
                    ("Dir", SortColumn::Direction),
                    ("Size", SortColumn::Size),
                    ("Duration", SortColumn::Duration),
                    ("Speed", SortColumn::Speed),
                    ("Status", SortColumn::Status),
                    ("Retransmits", SortColumn::Retransmits),
                ];
                for (label, col) in &cols {
                    if sort_header(
                        ui,
                        label,
                        *col,
                        transfers.sort_column,
                        transfers.sort_ascending,
                    ) {
                        if transfers.sort_column == *col {
                            transfers.sort_ascending = !transfers.sort_ascending;
                        } else {
                            transfers.sort_column = *col;
                            transfers.sort_ascending = true;
                        }
                    }
                }
                ui.end_row();

                for record in &filtered {
                    ui.label(record.client_addr.to_string());
                    ui.label(&record.filename);
                    ui.label(match record.direction {
                        Direction::Read => "Download",
                        Direction::Write => "Upload",
                    });
                    ui.label(format_bytes(record.bytes_transferred));
                    ui.label(format!("{}ms", record.duration_ms));
                    ui.label(format!("{:.2} Mbps", record.speed_mbps));
                    ui.label(match record.status {
                        SessionStatus::Completed => "OK",
                        SessionStatus::Failed => "FAIL",
                        SessionStatus::Cancelled => "Cancelled",
                        _ => "?",
                    });
                    ui.label(record.retransmits.to_string());
                    ui.end_row();
                }
            });
    });
}
