use egui::Ui;
use std::sync::Arc;

use crate::core::config::{AclConfig, AclRuleConfig};
use crate::core::state::AppState;

pub struct AclState {
    pub mode: String,
    pub rules: Vec<AclRuleEdit>,
    pub new_action: String,
    pub new_source: String,
    pub new_ops: String,
    pub new_comment: String,
    pub dirty: bool,
    pub status_message: String,
}

#[derive(Clone)]
pub struct AclRuleEdit {
    pub action: String,
    pub source: String,
    pub operations: String,
    pub comment: String,
    pub enabled: bool,
}

impl AclRuleEdit {
    fn is_valid_cidr(&self) -> bool {
        if self.source.is_empty() {
            return false;
        }
        self.source.parse::<ipnet::IpNet>().is_ok()
            || self.source.parse::<std::net::IpAddr>().is_ok()
    }
}

impl AclState {
    pub fn from_config(config: &AclConfig) -> Self {
        let rules = config
            .rules
            .iter()
            .map(|r| AclRuleEdit {
                action: r.action.clone(),
                source: r.source.clone(),
                operations: r.operations.join(", "),
                comment: r.comment.clone(),
                enabled: true,
            })
            .collect();

        Self {
            mode: config.mode.clone(),
            rules,
            new_action: "allow".to_string(),
            new_source: String::new(),
            new_ops: "read".to_string(),
            new_comment: String::new(),
            dirty: false,
            status_message: String::new(),
        }
    }

    fn to_config(&self) -> AclConfig {
        AclConfig {
            mode: self.mode.clone(),
            rules: self
                .rules
                .iter()
                .filter(|r| r.enabled)
                .map(|r| AclRuleConfig {
                    action: r.action.clone(),
                    source: r.source.clone(),
                    operations: r
                        .operations
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    comment: r.comment.clone(),
                })
                .collect(),
        }
    }
}

fn is_valid_cidr_input(source: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    source.parse::<ipnet::IpNet>().is_ok() || source.parse::<std::net::IpAddr>().is_ok()
}

/// Column widths computed from available width.
struct ColWidths {
    num: f32,
    action: f32,
    source: f32,
    ops: f32,
    comment: f32,
    enabled: f32,
    move_btns: f32,
    delete: f32,
}

impl ColWidths {
    fn compute(total: f32) -> Self {
        let num = 24.0;
        let action = 80.0;
        let ops = 110.0;
        let enabled = 28.0;
        let move_btns = 56.0;
        let delete = 24.0;
        let spacing = 7.0 * 6.0; // ~7 columns of spacing at ~6px each
        let fixed = num + action + ops + enabled + move_btns + delete + spacing;
        let flex = (total - fixed).max(200.0);
        let source = (flex * 0.30).max(100.0);
        let comment = (flex - source).max(80.0);
        Self {
            num,
            action,
            source,
            ops,
            comment,
            enabled,
            move_btns,
            delete,
        }
    }
}

fn draw_rule_row(
    ui: &mut Ui,
    i: usize,
    rule_count: usize,
    rule: &mut AclRuleEdit,
    cw: &ColWidths,
    dirty: &mut bool,
) -> (Option<(usize, usize)>, bool) {
    let mut swap: Option<(usize, usize)> = None;
    let mut remove = false;

    ui.horizontal(|ui| {
        // #
        ui.allocate_ui(egui::vec2(cw.num, 20.0), |ui| {
            ui.label((i + 1).to_string());
        });

        // Action
        ui.allocate_ui(egui::vec2(cw.action, 20.0), |ui| {
            let changed = egui::ComboBox::from_id_salt(format!("acl_act_{}", i))
                .selected_text(&rule.action)
                .width(cw.action - 12.0)
                .show_ui(ui, |ui| {
                    let mut c = false;
                    c |= ui
                        .selectable_value(&mut rule.action, "allow".to_string(), "Allow")
                        .changed();
                    c |= ui
                        .selectable_value(&mut rule.action, "deny".to_string(), "Deny")
                        .changed();
                    c
                })
                .inner
                .unwrap_or(false);
            if changed {
                *dirty = true;
            }
        });

        // Source (CIDR) — with validation coloring
        ui.allocate_ui(egui::vec2(cw.source, 20.0), |ui| {
            let valid = rule.is_valid_cidr();
            let color = if rule.source.is_empty() {
                ui.visuals().widgets.inactive.fg_stroke.color
            } else if valid {
                egui::Color32::from_rgb(0x4c, 0xaf, 0x50)
            } else {
                egui::Color32::from_rgb(0xf4, 0x43, 0x36)
            };
            let resp = ui.add(
                egui::TextEdit::singleline(&mut rule.source)
                    .hint_text("192.168.1.0/24")
                    .desired_width(cw.source - 8.0)
                    .text_color(color),
            );
            if resp.changed() {
                *dirty = true;
            }
            if !valid && !rule.source.is_empty() {
                resp.on_hover_text("Invalid CIDR notation");
            }
        });

        // Operations
        ui.allocate_ui(egui::vec2(cw.ops, 20.0), |ui| {
            let changed = egui::ComboBox::from_id_salt(format!("acl_ops_{}", i))
                .selected_text(&rule.operations)
                .width(cw.ops - 12.0)
                .show_ui(ui, |ui| {
                    let mut c = false;
                    for opt in &["read", "write", "read, write"] {
                        c |= ui
                            .selectable_value(&mut rule.operations, opt.to_string(), *opt)
                            .changed();
                    }
                    c
                })
                .inner
                .unwrap_or(false);
            if changed {
                *dirty = true;
            }
        });

        // Comment
        ui.allocate_ui(egui::vec2(cw.comment, 20.0), |ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut rule.comment)
                        .hint_text("Description...")
                        .desired_width(cw.comment - 8.0),
                )
                .changed()
            {
                *dirty = true;
            }
        });

        // Enabled
        ui.allocate_ui(egui::vec2(cw.enabled, 20.0), |ui| {
            if ui.checkbox(&mut rule.enabled, "").changed() {
                *dirty = true;
            }
        });

        // Move Up/Dn
        ui.allocate_ui(egui::vec2(cw.move_btns, 20.0), |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            if ui
                .add_enabled(i > 0, egui::Button::new("Up").small())
                .clicked()
            {
                swap = Some((i, i - 1));
            }
            if ui
                .add_enabled(i + 1 < rule_count, egui::Button::new("Dn").small())
                .clicked()
            {
                swap = Some((i, i + 1));
            }
        });

        // Delete
        ui.allocate_ui(egui::vec2(cw.delete, 20.0), |ui| {
            if ui.small_button("X").clicked() {
                remove = true;
            }
        });
    });

    (swap, remove)
}

pub fn draw(ui: &mut Ui, state: &Arc<AppState>, acl: &mut AclState) {
    let total_width = ui.available_width();
    let cw = ColWidths::compute(total_width);

    ui.heading("Access Control List");

    ui.horizontal(|ui| {
        ui.label("Mode:");
        let changed = egui::ComboBox::from_id_salt("acl_mode")
            .selected_text(&acl.mode)
            .show_ui(ui, |ui| {
                let mut c = false;
                c |= ui
                    .selectable_value(&mut acl.mode, "disabled".to_string(), "Disabled")
                    .changed();
                c |= ui
                    .selectable_value(&mut acl.mode, "whitelist".to_string(), "Whitelist")
                    .changed();
                c |= ui
                    .selectable_value(&mut acl.mode, "blacklist".to_string(), "Blacklist")
                    .changed();
                c
            })
            .inner
            .unwrap_or(false);
        if changed {
            acl.dirty = true;
        }
    });

    ui.separator();

    // Reserve height for bottom panel
    let bottom_height = 80.0;
    let scroll_height = (ui.available_height() - bottom_height).max(60.0);

    let mut all_swaps: Vec<(usize, usize)> = Vec::new();
    let mut all_removes: Vec<usize> = Vec::new();

    // ── Rules list (scrollable) ──
    egui::ScrollArea::vertical()
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if acl.rules.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("No ACL rules configured")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(0x9e, 0x9e, 0x9e)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "It is recommended to add ACL rules if the server is \
                             exposed to the network.\nUse whitelist mode to allow \
                             only trusted IP ranges, or blacklist mode to block \
                             specific addresses.",
                        )
                        .size(13.0)
                        .color(egui::Color32::from_rgb(0x75, 0x75, 0x75)),
                    );
                });
            } else {
                // Header
                ui.horizontal(|ui| {
                    ui.allocate_ui(egui::vec2(cw.num, 16.0), |ui| {
                        ui.strong("#");
                    });
                    ui.allocate_ui(egui::vec2(cw.action, 16.0), |ui| {
                        ui.strong("Action");
                    });
                    ui.allocate_ui(egui::vec2(cw.source, 16.0), |ui| {
                        ui.strong("Source (CIDR)");
                    });
                    ui.allocate_ui(egui::vec2(cw.ops, 16.0), |ui| {
                        ui.strong("Operations");
                    });
                    ui.allocate_ui(egui::vec2(cw.comment, 16.0), |ui| {
                        ui.strong("Comment");
                    });
                    ui.allocate_ui(egui::vec2(cw.enabled, 16.0), |ui| {
                        ui.strong("On");
                    });
                    ui.allocate_ui(egui::vec2(cw.move_btns, 16.0), |ui| {
                        ui.strong("Move");
                    });
                });

                ui.add_space(2.0);

                let rule_count = acl.rules.len();
                for i in 0..rule_count {
                    // Alternate row background
                    if i % 2 == 1 {
                        let rect = ui.available_rect_before_wrap();
                        let row_rect =
                            egui::Rect::from_min_size(rect.min, egui::vec2(total_width, 24.0));
                        ui.painter().rect_filled(
                            row_rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(255, 255, 255, 6),
                        );
                    }

                    let (swap, remove) =
                        draw_rule_row(ui, i, rule_count, &mut acl.rules[i], &cw, &mut acl.dirty);
                    if let Some(s) = swap {
                        all_swaps.push(s);
                    }
                    if remove {
                        all_removes.push(i);
                    }
                }
            }
        });

    // Apply pending mutations
    for (a, b) in all_swaps {
        acl.rules.swap(a, b);
        acl.dirty = true;
    }
    for idx in all_removes.into_iter().rev() {
        acl.rules.remove(idx);
        acl.dirty = true;
    }

    // ── Bottom: pinned Add + Apply/Reset ──

    ui.separator();

    // Add new rule
    ui.horizontal(|ui| {
        ui.label("Add:");

        egui::ComboBox::from_id_salt("new_acl_action")
            .selected_text(&acl.new_action)
            .width(cw.action - 12.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut acl.new_action, "allow".to_string(), "Allow");
                ui.selectable_value(&mut acl.new_action, "deny".to_string(), "Deny");
            });

        let new_valid = is_valid_cidr_input(&acl.new_source);
        let new_color = if acl.new_source.is_empty() {
            ui.visuals().widgets.inactive.fg_stroke.color
        } else if new_valid {
            egui::Color32::from_rgb(0x4c, 0xaf, 0x50)
        } else {
            egui::Color32::from_rgb(0xf4, 0x43, 0x36)
        };
        let resp = ui.add(
            egui::TextEdit::singleline(&mut acl.new_source)
                .hint_text("192.168.1.0/24")
                .desired_width(cw.source - 8.0)
                .text_color(new_color),
        );
        if !new_valid && !acl.new_source.is_empty() {
            resp.on_hover_text("Invalid CIDR notation");
        }

        egui::ComboBox::from_id_salt("new_acl_ops")
            .selected_text(&acl.new_ops)
            .width(cw.ops - 12.0)
            .show_ui(ui, |ui| {
                for opt in &["read", "write", "read, write"] {
                    ui.selectable_value(&mut acl.new_ops, opt.to_string(), *opt);
                }
            });

        ui.add(
            egui::TextEdit::singleline(&mut acl.new_comment)
                .hint_text("Description...")
                .desired_width(cw.comment - 8.0),
        );

        let can_add = !acl.new_source.is_empty() && new_valid;
        if ui
            .add_enabled(can_add, egui::Button::new("Add Rule"))
            .clicked()
        {
            acl.rules.push(AclRuleEdit {
                action: acl.new_action.clone(),
                source: acl.new_source.clone(),
                operations: acl.new_ops.clone(),
                comment: acl.new_comment.clone(),
                enabled: true,
            });
            acl.new_source.clear();
            acl.new_comment.clear();
            acl.dirty = true;
        }
    });

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let apply = ui.add_enabled(acl.dirty, egui::Button::new("Apply"));
        if apply.clicked() {
            let new_acl_config = acl.to_config();
            let mut config = (*state.config()).clone();
            config.acl = new_acl_config;
            let save_result = config.save();
            state.config.store(Arc::new(config));
            acl.dirty = false;
            match save_result {
                Ok(path) => {
                    acl.status_message = format!("ACL applied & saved to {}", path.display());
                }
                Err(e) => {
                    acl.status_message = format!("ACL applied (save failed: {})", e);
                }
            }
        }
        if ui.button("Reset").clicked() {
            *acl = AclState::from_config(&state.config().acl);
        }

        if !acl.status_message.is_empty() {
            ui.label(&acl.status_message);
        }
    });
}
