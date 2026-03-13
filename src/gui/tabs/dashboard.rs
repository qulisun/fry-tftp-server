use egui::{Color32, RichText, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crate::core::state::*;
use crate::gui::theme::Theme;

struct BandwidthSample {
    time_secs: f64,
    tx_bps: f64,
    rx_bps: f64,
}

pub struct DashboardState {
    samples: VecDeque<BandwidthSample>,
    last_sample: Instant,
    prev_tx: u64,
    prev_rx: u64,
    pub current_tx_rate: f64,
    pub current_rx_rate: f64,
    start_time: Instant,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self::new()
    }
}

impl DashboardState {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(300),
            last_sample: Instant::now(),
            prev_tx: 0,
            prev_rx: 0,
            current_tx_rate: 0.0,
            current_rx_rate: 0.0,
            start_time: Instant::now(),
        }
    }

    pub fn update(&mut self, state: &Arc<AppState>) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_sample);

        if elapsed.as_millis() >= 1000 {
            let tx = state.total_bytes_tx.load(Ordering::Relaxed);
            let rx = state.total_bytes_rx.load(Ordering::Relaxed);
            let dt = elapsed.as_secs_f64();

            self.current_tx_rate = (tx.saturating_sub(self.prev_tx)) as f64 / dt;
            self.current_rx_rate = (rx.saturating_sub(self.prev_rx)) as f64 / dt;

            let time_secs = now.duration_since(self.start_time).as_secs_f64();
            self.samples.push_back(BandwidthSample {
                time_secs,
                tx_bps: self.current_tx_rate,
                rx_bps: self.current_rx_rate,
            });

            if self.samples.len() > 300 {
                self.samples.pop_front();
            }

            self.prev_tx = tx;
            self.prev_rx = rx;
            self.last_sample = now;
        }
    }
}

fn format_bytes_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000_000.0 {
        format!("{:.1} GB/s", bytes_per_sec / 1_000_000_000.0)
    } else if bytes_per_sec >= 1_000_000.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1_000.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
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

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

pub fn draw(
    ui: &mut Ui,
    state: &Arc<AppState>,
    dashboard: &mut DashboardState,
    theme: &Theme,
    active_sessions: &[SessionInfo],
) {
    dashboard.update(state);

    // Status cards
    let spacing = ui.spacing().item_spacing.x;
    let inner_margin = 12.0;
    let total_available = ui.available_width();
    let card_width = ((total_available - spacing * 2.0) / 3.0 - inner_margin * 2.0).max(60.0);

    ui.horizontal(|ui| {
        // Active Sessions
        egui::Frame::new()
            .fill(theme.sidebar_bg())
            .corner_radius(8.0)
            .inner_margin(inner_margin)
            .show(ui, |ui| {
                ui.set_width(card_width);
                ui.label(RichText::new("Active Sessions").small());
                ui.label(
                    RichText::new(active_sessions.len().to_string())
                        .size(28.0)
                        .strong(),
                );
            });

        // TX Rate
        egui::Frame::new()
            .fill(theme.sidebar_bg())
            .corner_radius(8.0)
            .inner_margin(inner_margin)
            .show(ui, |ui| {
                ui.set_width(card_width);
                ui.label(RichText::new("TX Rate").small());
                ui.label(
                    RichText::new(format_bytes_rate(dashboard.current_tx_rate))
                        .size(28.0)
                        .strong(),
                );
            });

        // RX Rate
        egui::Frame::new()
            .fill(theme.sidebar_bg())
            .corner_radius(8.0)
            .inner_margin(inner_margin)
            .show(ui, |ui| {
                ui.set_width(card_width);
                ui.label(RichText::new("RX Rate").small());
                ui.label(
                    RichText::new(format_bytes_rate(dashboard.current_rx_rate))
                        .size(28.0)
                        .strong(),
                );
            });
    });

    ui.add_space(12.0);

    // Active transfers table
    ui.heading("Active Transfers");

    if active_sessions.is_empty() {
        ui.label("No active transfers");
    } else {
        egui::ScrollArea::horizontal().show(ui, |ui| {
            egui::Grid::new("active_transfers")
                .striped(true)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    ui.strong("Client");
                    ui.strong("File");
                    ui.strong("Dir");
                    ui.strong("Progress");
                    ui.strong("Speed");
                    ui.strong("Duration");
                    ui.strong("Blksize");
                    ui.strong("Window");
                    ui.end_row();

                    for session in active_sessions {
                        ui.label(session.client_addr.to_string());
                        ui.label(&session.filename);
                        ui.label(match session.direction {
                            Direction::Read => "Download",
                            Direction::Write => "Upload",
                        });

                        if let Some(tsize) = session.tsize {
                            if tsize > 0 {
                                let progress = session.bytes_transferred as f32 / tsize as f32;
                                ui.add(egui::ProgressBar::new(progress).text(format!(
                                    "{} / {}",
                                    format_bytes(session.bytes_transferred),
                                    format_bytes(tsize)
                                )));
                            } else {
                                ui.label(format_bytes(session.bytes_transferred));
                            }
                        } else {
                            ui.label(format_bytes(session.bytes_transferred));
                        }

                        let elapsed = session.started_at.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            session.bytes_transferred as f64 / elapsed
                        } else {
                            0.0
                        };
                        ui.label(format_bytes_rate(speed));
                        ui.label(format_duration(session.started_at.elapsed()));
                        ui.label(session.blksize.to_string());
                        ui.label(session.windowsize.to_string());
                        ui.end_row();
                    }
                });
        });
    }

    // Bandwidth graph (configurable)
    let config = state.config();
    if config.gui.show_bandwidth_chart {
        ui.add_space(12.0);
        ui.heading("Bandwidth");

        let tx_points: PlotPoints = dashboard
            .samples
            .iter()
            .map(|s| [s.time_secs, s.tx_bps / 1_000_000.0])
            .collect();
        let rx_points: PlotPoints = dashboard
            .samples
            .iter()
            .map(|s| [s.time_secs, s.rx_bps / 1_000_000.0])
            .collect();

        let tx_line = Line::new(tx_points)
            .name("TX (MB/s)")
            .color(Color32::from_rgb(0x42, 0xa5, 0xf5));
        let rx_line = Line::new(rx_points)
            .name("RX (MB/s)")
            .color(Color32::from_rgb(0x66, 0xbb, 0x6a));

        let now_secs = dashboard.start_time.elapsed().as_secs_f64();
        let x_min = (now_secs - 300.0).max(0.0);
        let x_range = now_secs - x_min;
        let x_pad = (x_range * 0.05).max(2.0);

        let plot_width = ui.available_width();
        Plot::new("bandwidth_plot")
            .height(200.0)
            .width(plot_width)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show_axes(true)
            .include_x(x_min)
            .include_x(now_secs + x_pad)
            .include_y(0.0)
            .legend(Legend::default().position(egui_plot::Corner::LeftTop))
            .show(ui, |plot_ui| {
                plot_ui.line(tx_line);
                plot_ui.line(rx_line);
            });
    }
}
