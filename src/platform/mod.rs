#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub mod windows_service;

#[cfg(windows)]
pub mod windows_eventlog;

use crate::core::state::AppState;
use std::sync::Arc;

/// Register platform-specific signal handlers for graceful shutdown and config reload
pub async fn register_signals(
    shutdown_token: Arc<tokio_util::sync::CancellationToken>,
    state: Option<Arc<AppState>>,
) {
    // Clone the inner token for platform handlers
    let token = (*shutdown_token).clone();

    #[cfg(unix)]
    unix::register_signals(token, state).await;

    #[cfg(windows)]
    {
        let _ = state; // Windows doesn't use state for SIGHUP
        windows::register_signals(token).await;
    }
}
