use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct TrayState {
    pub tray_icon: TrayIcon,
    pub show_id: tray_icon::menu::MenuId,
    pub stop_id: tray_icon::menu::MenuId,
    pub quit_id: tray_icon::menu::MenuId,
}

/// Embedded 32x32 PNG tray icon
const TRAY_ICON_PNG: &[u8] = include_bytes!("tray_icon_32.png");

fn load_icon_from_png() -> Icon {
    let decoder = png::Decoder::new(std::io::Cursor::new(TRAY_ICON_PNG));
    let mut reader = decoder.read_info().expect("failed to read tray icon PNG");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .expect("failed to decode tray icon PNG");
    let raw = &buf[..info.buffer_size()];

    // Convert to RGBA if needed (PNG may be RGB without alpha)
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.chunks(3) {
                out.extend_from_slice(chunk);
                out.push(255); // fully opaque
            }
            out
        }
        _ => raw.to_vec(), // fallback
    };

    Icon::from_rgba(rgba, info.width, info.height).expect("failed to create tray icon from PNG")
}

pub fn create_tray() -> anyhow::Result<TrayState> {
    let menu = Menu::new();

    let show_item = MenuItem::new("Show", true, None);
    let stop_item = MenuItem::new("Stop Server", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    let show_id = show_item.id().clone();
    let stop_id = stop_item.id().clone();
    let quit_id = quit_item.id().clone();

    menu.append(&show_item)?;
    menu.append(&stop_item)?;
    menu.append(&quit_item)?;

    let icon = load_icon_from_png();

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Fry TFTP Server - Running")
        .with_icon(icon)
        .build()?;

    Ok(TrayState {
        tray_icon,
        show_id,
        stop_id,
        quit_id,
    })
}

/// Tray visual state: Running (green), Stopped (grey), Error (red)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrayVisualState {
    Running,
    Stopped,
    Error,
}

pub fn update_tray_icon(tray: &TrayState, visual: TrayVisualState) {
    let tooltip = match visual {
        TrayVisualState::Running => "Fry TFTP Server - Running",
        TrayVisualState::Stopped => "Fry TFTP Server - Stopped",
        TrayVisualState::Error => "Fry TFTP Server - Error",
    };
    // Always use the same icon (custom PNG), just update tooltip
    let _ = tray.tray_icon.set_tooltip(Some(tooltip));
}

/// Poll for menu events, returns action if any
pub enum TrayAction {
    Show,
    Stop,
    Quit,
}

pub fn poll_tray_events(tray: &TrayState) -> Option<TrayAction> {
    if let Ok(event) = MenuEvent::receiver().try_recv() {
        if event.id == tray.show_id {
            return Some(TrayAction::Show);
        } else if event.id == tray.stop_id {
            return Some(TrayAction::Stop);
        } else if event.id == tray.quit_id {
            return Some(TrayAction::Quit);
        }
    }
    None
}
