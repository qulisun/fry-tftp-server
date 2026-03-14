use egui::Ui;
use std::sync::Arc;

use crate::core::config::Config;
use crate::core::i18n::I18n;
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
    pub language: String,

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
            language: config.gui.language.clone(),

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
        config.gui.language = self.language.clone();

        Ok(())
    }
}

pub fn draw(ui: &mut Ui, state: &Arc<AppState>, cs: &mut ConfigState, i18n: &I18n) {
    ui.heading(i18n.t("configuration"));

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.collapsing(i18n.t("server"), |ui| {
            ui.label(
                egui::RichText::new(i18n.t("port_restart_note"))
                    .small()
                    .weak(),
            );
            egui::Grid::new("server_cfg").show(ui, |ui| {
                ui.label(i18n.t("port"));
                if ui.text_edit_singleline(&mut cs.port).changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                ui.label(i18n.t("bind_address"));
                if ui.text_edit_singleline(&mut cs.bind_address).changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                ui.label(i18n.t("root_directory"));
                ui.horizontal(|ui| {
                    if ui.text_edit_singleline(&mut cs.root).changed() {
                        cs.dirty = true;
                    }
                    if ui.button(i18n.t("browse")).clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            cs.root = path.to_string_lossy().to_string();
                            cs.dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(i18n.t("ip_version"));
                egui::ComboBox::from_id_salt("cfg_ip_version")
                    .selected_text(&cs.ip_version)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(
                                &mut cs.ip_version,
                                "dual".to_string(),
                                i18n.t("dual_stack"),
                            )
                            .changed()
                        {
                            cs.dirty = true;
                        }
                        if ui
                            .selectable_value(
                                &mut cs.ip_version,
                                "v4".to_string(),
                                i18n.t("ipv4_only"),
                            )
                            .changed()
                        {
                            cs.dirty = true;
                        }
                        if ui
                            .selectable_value(
                                &mut cs.ip_version,
                                "v6".to_string(),
                                i18n.t("ipv6_only"),
                            )
                            .changed()
                        {
                            cs.dirty = true;
                        }
                    });
                ui.end_row();

                ui.label(i18n.t("log_level"));
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

                ui.label(i18n.t("max_log_lines"));
                ui.horizontal(|ui| {
                    if ui.text_edit_singleline(&mut cs.max_log_lines).changed() {
                        cs.dirty = true;
                    }
                    ui.weak(i18n.t("unlimited"));
                });
                ui.end_row();
            });
        });

        ui.collapsing(i18n.t("protocol"), |ui| {
            egui::Grid::new("protocol_cfg").show(ui, |ui| {
                ui.label(i18n.t("allow_write"));
                if ui.checkbox(&mut cs.allow_write, "").changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                for (label, value) in [
                    (
                        i18n.t("default_blksize"),
                        &mut cs.default_blksize as &mut String,
                    ),
                    (i18n.t("max_blksize"), &mut cs.max_blksize),
                    (i18n.t("default_windowsize"), &mut cs.default_windowsize),
                    (i18n.t("max_windowsize"), &mut cs.max_windowsize),
                    (i18n.t("default_timeout"), &mut cs.default_timeout),
                ] {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }
            });
        });

        ui.collapsing(i18n.t("session"), |ui| {
            egui::Grid::new("session_cfg").show(ui, |ui| {
                for (label, value) in [
                    (i18n.t("max_sessions"), &mut cs.max_sessions as &mut String),
                    (i18n.t("max_retries"), &mut cs.max_retries),
                    (i18n.t("session_timeout"), &mut cs.session_timeout),
                ] {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }

                ui.label(i18n.t("exponential_backoff"));
                if ui.checkbox(&mut cs.exponential_backoff, "").changed() {
                    cs.dirty = true;
                }
                ui.end_row();
            });
        });

        ui.collapsing(i18n.t("security"), |ui| {
            egui::Grid::new("security_cfg").show(ui, |ui| {
                for (label, value) in [
                    (
                        i18n.t("per_ip_max_sessions"),
                        &mut cs.per_ip_max_sessions as &mut String,
                    ),
                    (i18n.t("per_ip_rate_limit"), &mut cs.per_ip_rate_limit),
                    (i18n.t("rate_limit_window"), &mut cs.rate_limit_window),
                ] {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        cs.dirty = true;
                    }
                    ui.end_row();
                }
            });
        });

        ui.collapsing(i18n.t("dashboard_section"), |ui| {
            egui::Grid::new("dashboard_cfg").show(ui, |ui| {
                ui.label(i18n.t("show_bandwidth_chart"));
                if ui.checkbox(&mut cs.show_bandwidth_chart, "").changed() {
                    cs.dirty = true;
                }
                ui.end_row();
            });
        });

        ui.collapsing(i18n.t("language"), |ui| {
            egui::Grid::new("lang_cfg").show(ui, |ui| {
                ui.label(i18n.t("language_label"));
                let prev = cs.language.clone();
                egui::ComboBox::from_id_salt("lang_combo")
                    .selected_text(crate::core::i18n::Lang::parse(&cs.language).name())
                    .show_ui(ui, |ui| {
                        for lang in crate::core::i18n::Lang::ALL {
                            ui.selectable_value(
                                &mut cs.language,
                                lang.code().to_string(),
                                lang.name(),
                            );
                        }
                    });
                if cs.language != prev {
                    cs.dirty = true;
                }
                ui.end_row();
            });
        });

        ui.collapsing(i18n.t("filesystem"), |ui| {
            egui::Grid::new("fs_cfg").show(ui, |ui| {
                ui.label(i18n.t("max_file_size"));
                if ui.text_edit_singleline(&mut cs.max_file_size).changed() {
                    cs.dirty = true;
                }
                ui.end_row();

                for (label, value) in [
                    (
                        i18n.t("allow_overwrite"),
                        &mut cs.allow_overwrite as &mut bool,
                    ),
                    (i18n.t("create_directories"), &mut cs.create_dirs),
                    (i18n.t("follow_symlinks"), &mut cs.follow_symlinks),
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
        let apply_btn = ui.add_enabled(cs.dirty, egui::Button::new(i18n.t("apply")));
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
                        i18n.t("restart_note")
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

        if ui.button(i18n.t("reset_current")).clicked() {
            *cs = ConfigState::from_config(&state.config());
            cs.status_message = "Reset to current running config".to_string();
        }

        if ui.button(i18n.t("reset_defaults")).clicked() {
            *cs = ConfigState::from_config(&Config::default());
            cs.dirty = true;
            cs.status_message = "Reset to defaults (click Apply to activate)".to_string();
        }

        if ui.button(i18n.t("import_toml")).clicked() {
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

        if ui.button(i18n.t("export_toml")).clicked() {
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
