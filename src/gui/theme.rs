use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn apply(&self, ctx: &egui::Context) {
        match self {
            Theme::Dark => {
                let mut visuals = Visuals::dark();
                // Main content area — very dark grey
                visuals.panel_fill = Color32::from_rgb(0x1e, 0x1e, 0x1e);
                visuals.window_fill = Color32::from_rgb(0x1e, 0x1e, 0x1e);
                visuals.extreme_bg_color = Color32::from_rgb(0x16, 0x16, 0x16);

                // Text colors — bright white for readability
                visuals.override_text_color = Some(Color32::from_rgb(0xec, 0xec, 0xec));

                // Widgets — subtle rounded style
                let widget_rounding = CornerRadius::same(6);
                visuals.widgets.inactive.corner_radius = widget_rounding;
                visuals.widgets.hovered.corner_radius = widget_rounding;
                visuals.widgets.active.corner_radius = widget_rounding;
                visuals.widgets.open.corner_radius = widget_rounding;
                visuals.widgets.noninteractive.corner_radius = widget_rounding;

                // Inactive widgets — slightly lighter than background so buttons are visible
                visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x38, 0x38, 0x38);
                visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x38, 0x38, 0x38);
                visuals.widgets.inactive.bg_stroke =
                    Stroke::new(1.0, Color32::from_rgb(0x45, 0x45, 0x45));

                // Hovered widgets — brighter highlight
                visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x48, 0x48, 0x48);
                visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x48, 0x48, 0x48);
                visuals.widgets.hovered.bg_stroke =
                    Stroke::new(1.0, Color32::from_rgb(0x58, 0x58, 0x58));

                // Active/selected widgets — brightest
                visuals.widgets.active.bg_fill = Color32::from_rgb(0x50, 0x50, 0x50);
                visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0x50, 0x50, 0x50);

                // Open widgets (combo boxes, etc.)
                visuals.widgets.open.bg_fill = Color32::from_rgb(0x3a, 0x3a, 0x3a);
                visuals.widgets.open.weak_bg_fill = Color32::from_rgb(0x3a, 0x3a, 0x3a);

                // Selection highlight
                visuals.selection.bg_fill = Color32::from_rgb(0x3a, 0x3a, 0x3a);
                visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x60, 0x60, 0x60));

                ctx.set_visuals(visuals);
            }
            Theme::Light => {
                ctx.set_visuals(Visuals::light());
            }
        }
    }

    pub fn sidebar_bg(&self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(0x2f, 0x2f, 0x2f),
            Theme::Light => Color32::from_rgb(0xf0, 0xf0, 0xf5),
        }
    }

    pub fn sidebar_text(&self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(0xd0, 0xd0, 0xd0),
            Theme::Light => Color32::from_rgb(0x33, 0x33, 0x33),
        }
    }

    pub fn sidebar_hover_bg(&self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(0x3a, 0x3a, 0x3a),
            Theme::Light => Color32::from_rgb(0xe8, 0xe8, 0xf0),
        }
    }

    pub fn sidebar_selected_bg(&self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(0x44, 0x44, 0x44),
            Theme::Light => Color32::from_rgb(0xe0, 0xe0, 0xe8),
        }
    }

    pub fn sidebar_selected_text(&self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(0xff, 0xff, 0xff),
            Theme::Light => Color32::from_rgb(0x11, 0x11, 0x11),
        }
    }

    pub fn accent(&self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(0x6e, 0xa8, 0xfe),
            Theme::Light => Color32::from_rgb(0x19, 0x76, 0xd2),
        }
    }

    pub fn status_running(&self) -> Color32 {
        Color32::from_rgb(0x4c, 0xaf, 0x50)
    }

    pub fn status_stopped(&self) -> Color32 {
        Color32::from_rgb(0x9e, 0x9e, 0x9e)
    }

    pub fn status_error(&self) -> Color32 {
        Color32::from_rgb(0xf4, 0x43, 0x36)
    }

    pub fn log_color(&self, level: &tracing::Level) -> Color32 {
        match *level {
            tracing::Level::TRACE => Color32::from_rgb(0x90, 0x90, 0x90),
            tracing::Level::DEBUG => Color32::from_rgb(0x00, 0xbc, 0xd4),
            tracing::Level::INFO => Color32::from_rgb(0x4c, 0xaf, 0x50),
            tracing::Level::WARN => Color32::from_rgb(0xff, 0xc1, 0x07),
            tracing::Level::ERROR => Color32::from_rgb(0xf4, 0x43, 0x36),
        }
    }
}
