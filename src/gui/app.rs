use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::core::i18n::{I18n, Lang};
use crate::core::state::*;
use crate::gui::log_layer::LogBuffer;
use crate::gui::tabs::acl_tab::AclState;
use crate::gui::tabs::config_tab::ConfigState;
use crate::gui::tabs::dashboard::DashboardState;
use crate::gui::tabs::files::FilesState;
use crate::gui::tabs::help_tab::HelpState;
use crate::gui::tabs::log_tab::LogState;
use crate::gui::tabs::transfers::TransfersState;
use crate::gui::tabs::{self, Tab};
use crate::gui::theme::Theme;
use crate::gui::tray::{self, TrayState, TrayVisualState};

fn format_bytes_short(bytes: u64) -> String {
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

pub struct TftpApp {
    state: Arc<AppState>,
    current_tab: Tab,
    theme: Theme,
    log_buffer: LogBuffer,
    tray_state: Option<TrayState>,
    last_tray_visual: TrayVisualState,
    show_about: bool,
    pub i18n: I18n,
    /// Tokio runtime handle for spawning server restart tasks
    rt_handle: tokio::runtime::Handle,

    dashboard: DashboardState,
    files: FilesState,
    transfers: TransfersState,
    log_state: LogState,
    config_state: ConfigState,
    acl_state: AclState,
    help_state: HelpState,
}

impl TftpApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        state: Arc<AppState>,
        log_buffer: LogBuffer,
        tray_state: Option<TrayState>,
    ) -> Self {
        let config = state.config();
        let root = config.server.root.clone();

        Self {
            state,
            current_tab: Tab::Dashboard,
            theme: Theme::Dark,
            log_buffer,
            tray_state,
            last_tray_visual: TrayVisualState::Running,
            show_about: false,
            i18n: I18n::new(Lang::parse(&config.gui.language)),
            rt_handle: tokio::runtime::Handle::current(),
            dashboard: DashboardState::new(),
            files: FilesState::new(root),
            transfers: TransfersState::new(),
            log_state: LogState::new(),
            config_state: ConfigState::from_config(&config),
            acl_state: AclState::from_config(&config.acl),
            help_state: HelpState::new(),
        }
    }

    fn current_tray_visual(&self) -> TrayVisualState {
        let server_state = self.state.get_server_state();
        let errors = self.state.total_errors.load(Ordering::Relaxed);
        if server_state == ServerState::Error || errors > 0 {
            TrayVisualState::Error
        } else if server_state == ServerState::Running {
            TrayVisualState::Running
        } else {
            TrayVisualState::Stopped
        }
    }
}

impl eframe::App for TftpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.theme.apply(ctx);
        ctx.request_repaint_after(Duration::from_millis(250));

        // Sync language from config
        let cfg_lang = Lang::parse(&self.state.config().gui.language);
        if cfg_lang != self.i18n.lang() {
            self.i18n.set_lang(cfg_lang);
        }

        // Handle system tray events
        if let Some(ref tray) = self.tray_state {
            let visual = self.current_tray_visual();
            if visual != self.last_tray_visual {
                tray::update_tray_icon(tray, visual);
                self.last_tray_visual = visual;
            }

            if let Some(action) = tray::poll_tray_events(tray) {
                match action {
                    tray::TrayAction::Show => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    tray::TrayAction::Stop => {
                        self.state.cancel_shutdown();
                    }
                    tray::TrayAction::Quit => {
                        self.state.cancel_shutdown();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }

        // Close window = quit app (shutdown server and exit)

        // Collect state snapshots outside of UI rendering
        let server_state = self.state.get_server_state();
        let active_sessions: Vec<SessionInfo> = self
            .state
            .active_sessions
            .try_read()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default();
        let transfer_history: Vec<TransferRecord> = self
            .state
            .transfer_history
            .try_read()
            .map(|h| h.clone())
            .unwrap_or_default();

        // Top panel
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let t = &self.i18n;
                let (status_key, status_color) = match server_state {
                    ServerState::Running => ("running", self.theme.status_running()),
                    ServerState::Starting => ("starting", self.theme.accent()),
                    ServerState::Stopping => ("stopping", self.theme.accent()),
                    ServerState::Stopped => ("stopped", self.theme.status_stopped()),
                    ServerState::Error => ("error", self.theme.status_error()),
                };
                ui.label(t.t("status"));
                ui.colored_label(status_color, t.t(status_key));

                ui.separator();
                let config = self.state.config();
                ui.label(format!(
                    "{} {}:{}",
                    t.t("listening"),
                    config.server.bind_address,
                    config.server.port
                ));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t.t("about")).clicked() {
                        self.show_about = !self.show_about;
                    }

                    let theme_label = match self.theme {
                        Theme::Dark => t.t("light_mode"),
                        Theme::Light => t.t("dark_mode"),
                    };
                    if ui.button(theme_label).clicked() {
                        self.theme = match self.theme {
                            Theme::Dark => Theme::Light,
                            Theme::Light => Theme::Dark,
                        };
                    }

                    match server_state {
                        ServerState::Running => {
                            if ui.button(t.t("stop_server")).clicked() {
                                self.state.cancel_shutdown();
                            }
                        }
                        ServerState::Stopped | ServerState::Error => {
                            if ui.button(t.t("start_server")).clicked() {
                                let state = self.state.clone();
                                self.dashboard = DashboardState::new();
                                self.rt_handle.spawn(async move {
                                    let new_config = state
                                        .reload_config()
                                        .map(|()| (*state.config()).clone())
                                        .unwrap_or_default();
                                    state.reset_for_restart(new_config).await;
                                    if let Err(e) = crate::core::run_server(state.clone()).await {
                                        tracing::error!(error=%e, "server start failed");
                                        state.set_server_state(ServerState::Error);
                                    }
                                });
                            }
                        }
                        _ => {}
                    }
                });
            });
        });

        // About panel (right-side, between top and bottom headers)
        if self.show_about {
            let t = &self.i18n;
            egui::SidePanel::right("about_panel")
                .default_width(280.0)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading(t.t("about"));
                    ui.separator();
                    ui.add_space(8.0);

                    ui.label(egui::RichText::new(t.t("about_title")).strong().size(16.0));
                    ui.label(format!("{} {}", t.t("version"), env!("CARGO_PKG_VERSION")));
                    ui.add_space(12.0);

                    egui::Grid::new("about_grid")
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.strong(t.t("author"));
                            ui.label(t.t("author_name"));
                            ui.end_row();

                            ui.strong(t.t("email"));
                            ui.label("qulisun@gmail.com");
                            ui.end_row();

                            ui.strong(t.t("source"));
                            ui.hyperlink_to(
                                "github.com/qulisun/fry-tftp-server",
                                "https://github.com/qulisun/fry-tftp-server",
                            );
                            ui.end_row();

                            ui.strong(t.t("license"));
                            ui.label("MIT");
                            ui.end_row();
                        });

                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(t.t("built_with")).small().weak());

                    // Close button at bottom-right
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                        ui.add_space(4.0);
                        if ui.button(t.t("close")).clicked() {
                            self.show_about = false;
                        }
                    });
                });
        }

        // Left sidebar
        egui::SidePanel::left("sidebar")
            .default_width(160.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(self.theme.sidebar_bg())
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ctx, |ui| {
                ui.add_space(8.0);

                let draw_sidebar_button = |ui: &mut egui::Ui,
                                           tab: &Tab,
                                           current: &mut Tab,
                                           theme: &Theme,
                                           i18n: &I18n| {
                    let selected = *tab == *current;

                    // Reserve space and check hover before drawing
                    let desired_size = egui::vec2(ui.available_width(), 32.0);
                    let (rect, response) =
                        ui.allocate_exact_size(desired_size, egui::Sense::click());

                    let hovered = response.hovered();
                    let bg = if selected {
                        theme.sidebar_selected_bg()
                    } else if hovered {
                        theme.sidebar_hover_bg()
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    let text_color = if selected || hovered {
                        theme.sidebar_selected_text()
                    } else {
                        theme.sidebar_text()
                    };

                    // Draw rounded background
                    ui.painter()
                        .rect_filled(rect, egui::CornerRadius::same(8), bg);

                    // Draw label with icon
                    let label_text = format!("{} {}", tab.icon(), i18n.t(tab.i18n_key()));
                    let galley = ui.painter().layout_no_wrap(
                        label_text,
                        egui::FontId::proportional(14.0),
                        text_color,
                    );
                    let text_pos =
                        egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
                    ui.painter().galley(text_pos, galley, text_color);

                    if response.clicked() {
                        *current = *tab;
                    }
                };

                for tab in Tab::MAIN {
                    draw_sidebar_button(ui, tab, &mut self.current_tab, &self.theme, &self.i18n);
                    ui.add_space(2.0);
                }

                // Help pinned at bottom
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(4.0);
                    draw_sidebar_button(
                        ui,
                        &Tab::Help,
                        &mut self.current_tab,
                        &self.theme,
                        &self.i18n,
                    );
                });
            });

        // Bottom status bar with stats
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .frame(
                egui::Frame::new()
                    .fill(self.theme.sidebar_bg())
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ctx, |ui| {
                let t = &self.i18n;
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {}",
                            t.t("sessions"),
                            active_sessions.len()
                        ))
                        .small(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {}",
                            t.t("total"),
                            self.state.total_sessions.load(Ordering::Relaxed)
                        ))
                        .small(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {}",
                            t.t("errors"),
                            self.state.total_errors.load(Ordering::Relaxed)
                        ))
                        .small(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "TX: {}",
                            format_bytes_short(self.state.total_bytes_tx.load(Ordering::Relaxed))
                        ))
                        .small(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "RX: {}",
                            format_bytes_short(self.state.total_bytes_rx.load(Ordering::Relaxed))
                        ))
                        .small(),
                    );
                });
            });

        // Central panel: tab content
        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            Tab::Dashboard => {
                tabs::dashboard::draw(
                    ui,
                    &self.state,
                    &mut self.dashboard,
                    &self.theme,
                    &active_sessions,
                    &self.i18n,
                );
            }
            Tab::Files => {
                tabs::files::draw(ui, &self.state, &mut self.files, &self.i18n);
            }
            Tab::Transfers => {
                tabs::transfers::draw(ui, &transfer_history, &mut self.transfers, &self.i18n);
            }
            Tab::Log => {
                tabs::log_tab::draw(
                    ui,
                    &mut self.log_state,
                    &self.log_buffer,
                    &self.theme,
                    &self.i18n,
                );
            }
            Tab::Config => {
                tabs::config_tab::draw(ui, &self.state, &mut self.config_state, &self.i18n);
            }
            Tab::Acl => {
                tabs::acl_tab::draw(ui, &self.state, &mut self.acl_state, &self.i18n);
            }
            Tab::Help => {
                tabs::help_tab::draw(ui, &mut self.help_state, &self.i18n);
            }
        });
    }
}
