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

    // Use IconData::default() to disable eframe's built-in "e" icon on macOS,
    // so the system uses AppIcon.icns from the .app bundle (with proper rounded corners).
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 600.0])
            .with_min_inner_size([800.0, 500.0])
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
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
