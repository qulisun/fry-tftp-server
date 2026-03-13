//! IPC management interface for the TFTP server.
//!
//! Provides a line-based control protocol over:
//! - Unix domain socket (`/run/fry-tftp-server.sock` or `$XDG_RUNTIME_DIR/fry-tftp-server.sock`)
//! - Windows Named Pipe (`\\.\pipe\fry-tftp-server-control`)
//!
//! Commands: `reload`, `stop`, `status`

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Maximum concurrent IPC connections to prevent resource exhaustion.
const MAX_IPC_CONNECTIONS: usize = 50;
/// Per-connection read timeout to prevent idle connections from holding slots.
const IPC_CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

use crate::core::state::AppState;

/// Build a JSON status response from the current server state.
fn build_status_json(state: &AppState) -> String {
    let server_state = state.get_server_state();
    let config = state.config();
    let bw = state.get_bandwidth();

    serde_json::json!({
        "ok": true,
        "status": format!("{:?}", server_state),
        "active_sessions": state.active_sessions.try_read().map(|s| s.len()).unwrap_or(0),
        "total_sessions": state.total_sessions.load(Ordering::Relaxed),
        "total_bytes_tx": state.total_bytes_tx.load(Ordering::Relaxed),
        "total_bytes_rx": state.total_bytes_rx.load(Ordering::Relaxed),
        "total_errors": state.total_errors.load(Ordering::Relaxed),
        "bandwidth_tx_bps": bw.tx_bps,
        "bandwidth_rx_bps": bw.rx_bps,
        "bind_address": format!("{}:{}", config.server.bind_address, config.server.port),
        "root": config.server.root.display().to_string(),
        "buffer_pool_hits": state.buffer_pool.hits.load(Ordering::Relaxed),
        "buffer_pool_misses": state.buffer_pool.misses.load(Ordering::Relaxed),
    })
    .to_string()
}

/// Process a single IPC command and return the response JSON.
fn handle_command(cmd: &str, state: &Arc<AppState>) -> String {
    let cmd = cmd.trim();
    match cmd {
        "status" => build_status_json(state),
        "reload" => match state.reload_config() {
            Ok(()) => {
                tracing::info!("config reloaded via IPC");
                r#"{"ok":true,"message":"config reloaded"}"#.to_string()
            }
            Err(e) => {
                tracing::error!(error=%e, "IPC reload failed");
                serde_json::json!({"ok": false, "error": format!("config reload failed: {}", e)})
                    .to_string()
            }
        },
        "stop" => {
            tracing::info!("shutdown requested via IPC");
            state.cancel_shutdown();
            r#"{"ok":true,"message":"shutdown initiated"}"#.to_string()
        }
        _ => serde_json::json!({"ok": false, "error": format!("unknown command: {}", cmd)})
            .to_string(),
    }
}

// ─── Unix Domain Socket ─────────────────────────────────────────────────────

#[cfg(unix)]
mod unix_ipc {
    use super::*;
    use tokio::net::UnixListener;

    /// Determine the socket path.
    fn socket_path() -> std::path::PathBuf {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            std::path::PathBuf::from(runtime_dir).join("fry-tftp-server.sock")
        } else {
            std::path::PathBuf::from("/run/fry-tftp-server.sock")
        }
    }

    pub async fn spawn_ipc_listener(
        state: Arc<AppState>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<()> {
        let path = socket_path();

        // Remove stale socket file if it exists
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                // Try /tmp as fallback
                let fallback = std::path::PathBuf::from("/tmp/fry-tftp-server.sock");
                return spawn_listener_at(fallback, state, shutdown).await;
            }
        }

        spawn_listener_at(path, state, shutdown).await
    }

    async fn spawn_listener_at(
        path: std::path::PathBuf,
        state: Arc<AppState>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<()> {
        // Remove stale socket
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }

        let listener = UnixListener::bind(&path)?;
        tracing::info!(path=%path.display(), "IPC listener started (Unix socket)");

        let cleanup_path = path.clone();
        let semaphore = Arc::new(Semaphore::new(MAX_IPC_CONNECTIONS));
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        tracing::debug!("IPC listener shutting down");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _)) => {
                                let state = state.clone();
                                let permit = match semaphore.clone().try_acquire_owned() {
                                    Ok(p) => p,
                                    Err(_) => {
                                        tracing::warn!("IPC connection limit reached, dropping connection");
                                        drop(stream);
                                        continue;
                                    }
                                };
                                tokio::spawn(async move {
                                    let _permit = permit; // held until task completes
                                    let result = tokio::time::timeout(IPC_CONNECTION_TIMEOUT, async {
                                        let (reader, mut writer) = stream.into_split();
                                        let mut reader = BufReader::new(reader);
                                        let mut line = String::new();

                                        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                                            let response = handle_command(&line, &state);
                                            let _ = writer.write_all(response.as_bytes()).await;
                                            let _ = writer.write_all(b"\n").await;
                                            let _ = writer.flush().await;
                                            line.clear();
                                        }
                                    }).await;
                                    if result.is_err() {
                                        tracing::debug!("IPC connection timed out");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!(error=%e, "IPC accept error");
                            }
                        }
                    }
                }
            }

            // Cleanup socket file
            let _ = std::fs::remove_file(&cleanup_path);
        });

        Ok(())
    }
}

// ─── Windows Named Pipe ─────────────────────────────────────────────────────

#[cfg(windows)]
mod windows_ipc {
    use super::*;
    use tokio::net::windows::named_pipe::ServerOptions;

    const PIPE_NAME: &str = r"\\.\pipe\fry-tftp-server-control";

    pub async fn spawn_ipc_listener(
        state: Arc<AppState>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<()> {
        tracing::info!(pipe=%PIPE_NAME, "IPC listener started (Named Pipe)");

        let semaphore = Arc::new(Semaphore::new(MAX_IPC_CONNECTIONS));
        tokio::spawn(async move {
            loop {
                // Create a new pipe instance for each connection
                let server = match ServerOptions::new()
                    .first_pipe_instance(false)
                    .create(PIPE_NAME)
                {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error=%e, "failed to create named pipe instance");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                };

                tokio::select! {
                    _ = shutdown.cancelled() => {
                        tracing::debug!("IPC listener shutting down");
                        break;
                    }
                    result = server.connect() => {
                        match result {
                            Ok(()) => {
                                let state = state.clone();
                                let permit = match semaphore.clone().try_acquire_owned() {
                                    Ok(p) => p,
                                    Err(_) => {
                                        tracing::warn!("IPC connection limit reached, dropping connection");
                                        continue;
                                    }
                                };
                                tokio::spawn(async move {
                                    let _permit = permit;
                                    let result = tokio::time::timeout(IPC_CONNECTION_TIMEOUT, async {
                                        let (reader, mut writer) = tokio::io::split(server);
                                        let mut reader = BufReader::new(reader);
                                        let mut line = String::new();

                                        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                                            let response = handle_command(&line, &state);
                                            let _ = writer.write_all(response.as_bytes()).await;
                                            let _ = writer.write_all(b"\n").await;
                                            let _ = writer.flush().await;
                                            line.clear();
                                        }
                                    }).await;
                                    if result.is_err() {
                                        tracing::debug!("IPC connection timed out");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!(error=%e, "IPC pipe connect error");
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Spawn the IPC listener appropriate for the current platform.
pub async fn spawn_ipc_listener(
    state: Arc<AppState>,
    shutdown: Arc<CancellationToken>,
) -> anyhow::Result<()> {
    let token = (*shutdown).clone();

    #[cfg(unix)]
    {
        unix_ipc::spawn_ipc_listener(state, token).await
    }

    #[cfg(windows)]
    {
        windows_ipc::spawn_ipc_listener(state, token).await
    }

    #[cfg(not(any(unix, windows)))]
    {
        tracing::warn!("IPC not supported on this platform");
        let _ = (state, token);
        Ok(())
    }
}
