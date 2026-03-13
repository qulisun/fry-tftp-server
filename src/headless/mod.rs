use std::sync::Arc;

use crate::core::state::AppState;
use crate::platform;

/// Run the server in headless (daemon) mode
pub async fn run(state: Arc<AppState>) -> anyhow::Result<()> {
    // Register platform-specific signal handlers
    platform::register_signals(state.get_shutdown_token(), Some(state.clone())).await;

    // Start IPC listener for control commands (reload/stop/status)
    if let Err(e) =
        crate::core::ipc::spawn_ipc_listener(state.clone(), state.get_shutdown_token()).await
    {
        tracing::warn!(error=%e, "IPC listener failed to start (control interface unavailable)");
    }

    // Run the core server loop
    crate::core::run_server(state).await
}
