use egui::Ui;
use std::sync::Arc;

use crate::core::config::Config;
use crate::core::state::AppState;

pub struct ConfigState {
    pub port: String,
    pub bind_address: String,
    pub root: String,
    pub ip_version: String,
    pub log_level: String,
    pub max_log_lines: String,

    pub allow_write: bool,
    pub default_blksize: String,
    pub max_blksize: String,
    pub default_windowsize: String,
    pub max_windowsize: String,
    pub default_timeout: String,

    pub max_sessions: String,
    pub max_retries: String,
    pub exponential_backoff: bool,
    pub session_timeout: String,

    pub per_ip_max_sessions: String,
    pub per_ip_rate_limit: String,
    pub rate_limit_window: String,

    pub max_file_size: String,
    pub allow_overwrite: bool,
    pub create_dirs: bool,
    pub follow_symlinks: bool,

    pub show_bandwidth_chart: bool,

    pub dirty: bool,
    pub status_message: String,
}

impl ConfigState {
    pub fn from_config(config: &Config) -> Self {
        Self {
            port: config.server.port.to_string(),
            bind_address: config.server.bind_address.clone(),
            root: config.server.root.to_string_lossy().to_string(),
            ip_version: config.network.ip_version.clone(),
            log_level: config.server.log_level.clone(),
            max_log_lines: config.server.max_log_lines.to_string(),

            allow_write: config.protocol.allow_write,
            default_blksize: config.protocol.default_blksize.to_string(),
            max_blksize: config.protocol.max_blksize.to_string(),
            default_windowsize: config.protocol.default_windowsize.to_string(),
            max_windowsize: config.protocol.max_windowsize.to_string(),
            default_timeout: config.protocol.default_timeout.to_string(),

            max_sessions: config.session.max_sessions.to_string(),
            max_retries: config.session.max_retries.to_string(),
            exponential_backoff: config.session.exponential_backoff,
            session_timeout: config.session.session_timeout.to_string(),

            per_ip_max_sessions: config.security.per_ip_max_sessions.to_string(),
            per_ip_rate_limit: config.security.per_ip_rate_limit.to_string(),
            rate_limit_window: config.security.rate_limit_window_seconds.to_string(),

            max_file_size: config.filesystem.max_file_size.clone(),
            allow_overwrite: config.filesystem.allow_overwrite,
            create_dirs: config.filesystem.create_dirs,
            follow_symlinks: config.filesystem.follow_symlinks,

            show_bandwidth_chart: config.gui.show_bandwidth_chart,

            dirty: false,
            status_message: String::new(),
        }
    }

    fn apply_to_config(&self, config: &mut Config) -> Result<(), String> {
        config.server.port = self.port.parse().map_err(|_| "Invalid port")?;
        config.server.bind_address = self.bind_address.clone();
        config.server.root = self.root.clone().into();
        config.network.ip_version = self.ip_version.clone();
        config.server.log_level = self.log_level.clone();
        config.server.max_log_lines = self
            .max_log_lines
            .parse()
            .map_err(|_| "Invalid max log lines")?;

        config.protocol.allow_write = self.allow_write;
        config.protocol.default_blksize = self
            .default_blksize
            .parse()
            .map_err(|_| "Invalid blksize")?;
        config.protocol.max_blksize = self
            .max_blksize
            .parse()
            .map_err(|_| "Invalid max blksize")?;
        config.protocol.default_windowsize = self
            .default_windowsize
            .parse()
            .map_err(|_| "Invalid windowsize")?;
        config.protocol.max_windowsize = self
            .max_windowsize
            .parse()
            .map_err(|_| "Invalid max windowsize")?;
        config.protocol.default_timeout = self
            .default_timeout
            .parse()
            .map_err(|_| "Invalid timeout")?;

        config.session.max_sessions = self
            .max_sessions
            .parse()
            .map_err(|_| "Invalid max sessions")?;
        config.session.max_retries = self
            .max_retries
            .parse()
            .map_err(|_| "Invalid max retries")?;
        config.session.exponential_backoff = self.exponential_backoff;
        config.session.session_timeout = self
            .session_timeout
            .parse()
            .map_err(|_| "Invalid session timeout")?;

        config.security.per_ip_max_sessions = self
            .per_ip_max_sessions
            .parse()
            .map_err(|_| "Invalid per-IP max sessions")?;
        config.security.per_ip_rate_limit = self
            .per_ip_rate_limit
            .parse()
            .map_err(|_| "Invalid rate limit")?;
        config.security.rate_limit_window_seconds = self
            .rate_limit_window
            .parse()
            .map_err(|_| "Invalid rate limit window")?;

        config.filesystem.max_file_size = self.max_file_size.clone();
        config.filesystem.allow_overwrite = self.allow_overwrite;
        config.filesystem.create_dirs = self.create_dirs;
        config.filesystem.follow_symlinks = self.follow_symlinks;

        config.gui.show_bandwidth_chart = self.show_bandwidth_chart;

        Ok(())
    }
}

pub fn draw(ui: &mut Ui, state: &Arc<AppState>, cs: &mut ConfigState) {
    ui.heading("Configuration");

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.collapsing("Server", |ui| {
            ui.label(
                egui::RichText::new(
                    "* Port, Bind Address and IP Version require restart to take effect",
                )
                .small()
                .weak(),
            );
            egui::Grid::new("server_cfg").show(ui, |ui| {
                ui.label("Port *:");
                if ui.text_edit_singleline(&mut cs.port).changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                ui.label("Bind Address *:");
                if ui.text_edit_singleline(&mut cs.bind_address).changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                ui.label("Root Directory:");
                ui.horizontal(|ui| {
                    if ui.text_edit_singleline(&mut cs.root).changed() {
                        cs.dirty = true;
                    }
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            cs.root = path.to_string_lossy().to_string();
                            cs.dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label("IP Version *:");
                egui::ComboBox::from_id_salt("cfg_ip_version")
                    .selected_text(&cs.ip_version)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut cs.ip_version, "dual".to_string(), "Dual Stack")
                            .changed()
                        {
                            cs.dirty = true;
                        }
                        if ui
                            .selectable_value(&mut cs.ip_version, "v4".to_string(), "IPv4 Only")
                            .changed()
                        {
                            cs.dirty = true;
                        }
                        if ui
                            .selectable_value(&mut cs.ip_version, "v6".to_string(), "IPv6 Only")
                            .changed()
                        {
                            cs.dirty = true;
                        }
                    });
                ui.end_row();

                ui.label("Log Level:");
                egui::ComboBox::from_id_salt("cfg_log_level")
                    .selected_text(&cs.log_level)
                    .show_ui(ui, |ui| {
                        for level in &["trace", "debug", "info", "warn", "error"] {
                            if ui
                                .selectable_value(&mut cs.log_level, level.to_string(), *level)
                                .changed()
                            {
                                cs.dirty = true;
                            }
                        }
                    });
                ui.end_row();

                ui.label("Max Log Lines:");
                ui.horizontal(|ui| {
                    if ui.text_edit_singleline(&mut cs.max_log_lines).changed() {
                        cs.dirty = true;
                    }
                    ui.weak("0 = unlimited");
                });
                ui.end_row();
            });
        });

        ui.collapsing("Protocol", |ui| {
            egui::Grid::new("protocol_cfg").show(ui, |ui| {
                ui.label("Allow Write:");
                if ui.checkbox(&mut cs.allow_write, "").changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                for (label, value) in [
                    ("Default Blksize:", &mut cs.default_blksize as &mut String),
                    ("Max Blksize:", &mut cs.max_blksize),
                    ("Default Windowsize:", &mut cs.default_windowsize),
                    ("Max Windowsize:", &mut cs.max_windowsize),
                    ("Default Timeout:", &mut cs.default_timeout),
                ] {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }
            });
        });

        ui.collapsing("Session", |ui| {
            egui::Grid::new("session_cfg").show(ui, |ui| {
                for (label, value) in [
                    ("Max Sessions:", &mut cs.max_sessions as &mut String),
                    ("Max Retries:", &mut cs.max_retries),
                    ("Session Timeout (s):", &mut cs.session_timeout),
                ] {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }

                ui.label("Exponential Backoff:");
                if ui.checkbox(&mut cs.exponential_backoff, "").changed() {
                    cs.dirty = true;
                }
                ui.end_row();
            });
        });

        ui.collapsing("Security", |ui| {
            egui::Grid::new("security_cfg").show(ui, |ui| {
                for (label, value) in [
                    (
                        "Per-IP Max Sessions:",
                        &mut cs.per_ip_max_sessions as &mut String,
                    ),
                    ("Per-IP Rate Limit:", &mut cs.per_ip_rate_limit),
                    ("Rate Limit Window (s):", &mut cs.rate_limit_window),
                ] {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }
            });
        });

        ui.collapsing("Dashboard", |ui| {
            egui::Grid::new("dashboard_cfg").show(ui, |ui| {
                ui.label("Show Bandwidth Chart:");
                if ui.checkbox(&mut cs.show_bandwidth_chart, "").changed() {
                    cs.dirty = true;
                }
                ui.end_row();
            });
        });

        ui.collapsing("Filesystem", |ui| {
            egui::Grid::new("fs_cfg").show(ui, |ui| {
                ui.label("Max File Size:");
                if ui.text_edit_singleline(&mut cs.max_file_size).changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                for (label, value) in [
                    ("Allow Overwrite:", &mut cs.allow_overwrite as &mut bool),
                    ("Create Directories:", &mut cs.create_dirs),
                    ("Follow Symlinks:", &mut cs.follow_symlinks),
                ] {
                    ui.label(label);
                    if ui.checkbox(value, "").changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }
            });
        });
    });

    ui.separator();

    ui.horizontal(|ui| {
        let apply_btn = ui.add_enabled(cs.dirty, egui::Button::new("Apply"));
        if apply_btn.clicked() {
            let old_config = state.config();
            let mut new_config = (*old_config).clone();
            match cs.apply_to_config(&mut new_config) {
                Ok(()) => {
                    // Detect if restart-requiring settings changed
                    let needs_restart = new_config.server.port != old_config.server.port
                        || new_config.server.bind_address != old_config.server.bind_address
                        || new_config.network.ip_version != old_config.network.ip_version;

                    let save_result = new_config.save();
                    state.config.store(Arc::new(new_config));
                    cs.dirty = false;

                    let restart_note = if needs_restart {
                        " (Port/Bind/IP changes require server restart)"
                    } else {
                        ""
                    };

                    match save_result {
                        Ok(path) => {
                            cs.status_message = format!(
                                "Config applied & saved to {}{}",
                                path.display(),
                                restart_note
                            );
                        }
                        Err(e) => {
                            cs.status_message =
                                format!("Config applied (save failed: {}){}", e, restart_note);
                        }
                    }
                }
                Err(e) => {
                    cs.status_message = format!("Error: {}", e);
                }
            }
        }

        if ui.button("Reset to Current").clicked() {
            *cs = ConfigState::from_config(&state.config());
            cs.status_message = "Reset to current running config".to_string();
        }

        if ui.button("Reset to Defaults").clicked() {
            *cs = ConfigState::from_config(&Config::default());
            cs.dirty = true;
            cs.status_message = "Reset to defaults (click Apply to activate)".to_string();
        }

        if ui.button("Import TOML...").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("TOML", &["toml"])
                .pick_file()
            {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<Config>(&content) {
                        Ok(imported) => {
                            *cs = ConfigState::from_config(&imported);
                            cs.dirty = true;
                            cs.status_message = format!(
                                "Imported from {} (click Apply to activate)",
                                path.display()
                            );
                        }
                        Err(e) => {
                            cs.status_message = format!("Parse error: {}", e);
                        }
                    },
                    Err(e) => {
                        cs.status_message = format!("Read error: {}", e);
                    }
                }
            }
        }

        if ui.button("Export TOML...").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("TOML", &["toml"])
                .save_file()
            {
                let config = state.config();
                match toml::to_string_pretty(&*config) {
                    Ok(content) => {
                        if let Err(e) = std::fs::write(&path, &content) {
                            cs.status_message = format!("Write error: {}", e);
                        } else {
                            cs.status_message = "Config exported".to_string();
                        }
                    }
                    Err(e) => {
                        cs.status_message = format!("Serialize error: {}", e);
                    }
                }
            }
        }
    });

    if !cs.status_message.is_empty() {
        ui.label(&cs.status_message);
    }
}
