#[cfg(feature = "gui")]
pub mod app;
#[cfg(feature = "gui")]
pub mod log_layer;
#[cfg(feature = "gui")]
pub mod tabs;
#[cfg(feature = "gui")]
pub mod theme;
#[cfg(feature = "gui")]
pub mod tray;

#[cfg(feature = "gui")]
pub async fn run(
    state: std::sync::Arc<crate::core::state::AppState>,
    log_buffer: log_layer::LogBuffer,
) -> anyhow::Result<()> {
    use crate::platform;

    // Register platform-specific signal handlers
    platform::register_signals(state.get_shutdown_token(), Some(state.clone())).await;

    // Spawn the TFTP server in background
    let server_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::core::run_server(server_state.clone()).await {
            tracing::error!(error = %e, "server error");
            server_state.set_server_state(crate::core::state::ServerState::Error);
        }
    });

    let app_state_for_close = state.clone();

    // Load window/dock icon from embedded 256px PNG
    let icon = {
        let png_bytes = include_bytes!("app_icon_256.png");
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes as &[u8]));
        let mut reader = decoder.read_info().expect("failed to read icon PNG");
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buf)
            .expect("failed to decode icon PNG");
        let raw = &buf[..info.buffer_size()];
        let rgba = if info.color_type == png::ColorType::Rgb {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.chunks(3) {
                out.extend_from_slice(chunk);
                out.push(255);
            }
            out
        } else {
            raw.to_vec()
        };
        egui::IconData {
            rgba,
            width: info.width,
            height: info.height,
        }
    };

    // Run eframe on the current thread (blocks until window is closed)
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_icon(std::sync::Arc::new(icon)),
        ..Default::default()
    };

    let result = tokio::task::block_in_place(|| {
        // Create system tray icon (must be on same thread as event loop)
        let tray_state = tray::create_tray().ok();

        eframe::run_native(
            "Fry TFTP Server",
            options,
            Box::new(move |cc| {
                Ok(Box::new(app::TftpApp::new(
                    cc, state, log_buffer, tray_state,
                )))
            }),
        )
        .map_err(|e| anyhow::anyhow!("eframe error: {}", e))
    });

    // Window closed — shut down server
    app_state_for_close.cancel_shutdown();
    result
}
