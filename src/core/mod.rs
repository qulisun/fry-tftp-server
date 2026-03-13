pub mod acl;
pub mod buffer_pool;
pub mod config;
pub mod fs;
pub mod ipc;
pub mod log_buffer;
pub mod net;
pub mod protocol;
pub mod session;
pub mod state;

use std::sync::Arc;

use crate::core::acl::{AclEngine, Operation};
use crate::core::config::Config;
use crate::core::net::create_main_socket;
use crate::core::protocol::packet::*;
use crate::core::state::*;

/// Run the TFTP server core loop
pub async fn run_server(state: Arc<AppState>) -> anyhow::Result<()> {
    let config = state.config();

    // Truncate log file at startup if max_log_lines is set
    if config.server.max_log_lines > 0 && !config.server.log_file.is_empty() {
        let log_path = std::path::PathBuf::from(&config.server.log_file);
        crate::core::log_buffer::truncate_log_file(&log_path, config.server.max_log_lines);
    }

    let main_socket = create_main_socket(&config)?;

    let local_addr = main_socket.local_addr()?;
    tracing::info!(
        bind=%local_addr,
        root=%config.server.root.display(),
        "TFTP server started"
    );

    state.set_server_state(ServerState::Running);

    // Spawn periodic stale session cleanup task
    {
        let cleanup_state = state.clone();
        let cleanup_shutdown = state.get_shutdown_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                tokio::select! {
                    _ = cleanup_shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        let timeout = cleanup_state.config().session.session_timeout;
                        if timeout > 0 {
                            let cleaned = cleanup_state.cleanup_stale_sessions(timeout).await;
                            if cleaned > 0 {
                                tracing::info!(cleaned=%cleaned, "stale sessions cleaned up");
                            }
                        }
                        // Evict expired rate-limiter entries to prevent unbounded growth
                        cleanup_state.cleanup_stale_rate_limits().await;
                    }
                }
            }
        });
    }

    // Spawn bandwidth sampling task (1Hz)
    {
        let bw_state = state.clone();
        let bw_shutdown = state.get_shutdown_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = bw_shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        bw_state.sample_bandwidth();
                    }
                }
            }
        });
    }

    // Spawn periodic log file truncation (every 60 seconds)
    {
        let log_state = state.clone();
        let log_shutdown = state.get_shutdown_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = log_shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        let cfg = log_state.config();
                        if cfg.server.max_log_lines > 0 && !cfg.server.log_file.is_empty() {
                            let log_path = std::path::PathBuf::from(&cfg.server.log_file);
                            crate::core::log_buffer::truncate_log_file(&log_path, cfg.server.max_log_lines);
                        }
                    }
                }
            }
        });
    }

    // Spawn config file watcher for hot-reload
    {
        let watch_state = state.clone();
        let watch_shutdown = state.get_shutdown_token();
        tokio::spawn(async move {
            if let Err(e) = spawn_config_watcher(watch_state, watch_shutdown).await {
                tracing::warn!(error=%e, "config file watcher failed to start");
            }
        });
    }

    let mut buf = vec![0u8; 65536];
    let server_token = state.get_shutdown_token();

    loop {
        tokio::select! {
            _ = server_token.cancelled() => {
                tracing::info!("server shutdown initiated");
                break;
            }

            result = main_socket.recv_from(&mut buf) => {
                let (len, client_addr) = match result {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error=%e, "recv error on main socket");
                        continue;
                    }
                };

                let packet = match parse_packet(&buf[..len]) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(client=%client_addr, error=%e, "malformed packet");
                        continue;
                    }
                };

                // Re-read config on each request for hot-reload support
                let config = state.config();
                let acl = AclEngine::new(&config.acl);

                match packet {
                    Packet::Rrq { filename, mode, options } => {
                        let client_ip = client_addr.ip();

                        // Rate limit check
                        if !state.check_rate_limit(client_ip).await {
                            tracing::warn!(client=%client_ip, "rate limit exceeded");
                            if config.security.rate_limit_action == "error" {
                                let err_pkt = serialize_packet(&Packet::Error {
                                    code: ErrorCode::NotDefined,
                                    message: "Rate limit exceeded".to_string(),
                                });
                                let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            }
                            continue;
                        }

                        // ACL check
                        if !acl.check(client_ip, Operation::Read) {
                            let err_pkt = serialize_packet(&Packet::Error {
                                code: ErrorCode::AccessViolation,
                                message: "Access denied".to_string(),
                            });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            tracing::warn!(client=%client_ip, file=%filename, event="access_denied");
                            continue;
                        }

                        // Per-IP session limit
                        if state.count_sessions_by_ip(client_ip).await >= config.security.per_ip_max_sessions {
                            let err_pkt = serialize_packet(&Packet::Error {
                                code: ErrorCode::NotDefined,
                                message: "Too many sessions".to_string(),
                            });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            continue;
                        }

                        // Global session limit
                        if state.count_sessions().await >= config.session.max_sessions {
                            let err_pkt = serialize_packet(&Packet::Error {
                                code: ErrorCode::NotDefined,
                                message: "Server busy".to_string(),
                            });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            continue;
                        }

                        session::spawn_read_session(
                            state.clone(),
                            client_addr,
                            filename,
                            mode,
                            options,
                            &main_socket,
                        )
                        .await;
                    }

                    Packet::Wrq { filename, mode, options } => {
                        if !config.protocol.allow_write {
                            let err_pkt = serialize_packet(&Packet::Error {
                                code: ErrorCode::AccessViolation,
                                message: "Write not allowed".to_string(),
                            });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            continue;
                        }

                        let client_ip = client_addr.ip();

                        if !state.check_rate_limit(client_ip).await { continue; }
                        if !acl.check(client_ip, Operation::Write) {
                            let err_pkt = serialize_packet(&Packet::Error { code: ErrorCode::AccessViolation, message: "Access denied".to_string() });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            continue;
                        }
                        if state.count_sessions_by_ip(client_ip).await >= config.security.per_ip_max_sessions {
                            let err_pkt = serialize_packet(&Packet::Error { code: ErrorCode::NotDefined, message: "Too many sessions".to_string() });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            continue;
                        }
                        if state.count_sessions().await >= config.session.max_sessions {
                            let err_pkt = serialize_packet(&Packet::Error { code: ErrorCode::NotDefined, message: "Server busy".to_string() });
                            let _ = main_socket.send_to(&err_pkt, client_addr).await;
                            continue;
                        }

                        session::spawn_write_session(
                            state.clone(),
                            client_addr,
                            filename,
                            mode,
                            options,
                            &main_socket,
                        )
                        .await;
                    }

                    _ => {
                        let err_pkt = serialize_packet(&Packet::Error {
                            code: ErrorCode::IllegalOperation,
                            message: "Illegal operation on main socket".to_string(),
                        });
                        let _ = main_socket.send_to(&err_pkt, client_addr).await;
                    }
                }
            }
        }
    }

    state.set_server_state(ServerState::Stopping);

    // Graceful shutdown: wait for active sessions
    let grace = Duration::from_secs(config.session.shutdown_grace_period);
    let deadline = tokio::time::Instant::now() + grace;

    loop {
        if state.count_sessions().await == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("grace period expired, forcing shutdown of remaining sessions");
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    state.set_server_state(ServerState::Stopped);
    tracing::info!("server stopped");
    Ok(())
}

use std::time::Duration;

/// Watch the config file for changes and hot-reload on modification.
async fn spawn_config_watcher(
    state: Arc<AppState>,
    shutdown: Arc<tokio_util::sync::CancellationToken>,
) -> anyhow::Result<()> {
    use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc;

    let config_path = match Config::config_file_path() {
        Some(p) => p,
        None => {
            tracing::debug!("no config file found, skipping file watcher");
            return Ok(());
        }
    };

    tracing::info!(path=%config_path.display(), "watching config file for changes");

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )?;

    // Watch the parent directory (some editors delete+recreate files)
    let watch_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
    watcher.watch(watch_dir.as_ref(), RecursiveMode::NonRecursive)?;

    // Process events in a blocking thread since mpsc::Receiver is blocking
    tokio::task::spawn_blocking(move || {
        let debounce = Duration::from_secs(2);
        let mut last_reload = std::time::Instant::now() - debounce;

        loop {
            // Check shutdown (non-blocking)
            if shutdown.is_cancelled() {
                break;
            }

            // Wait for event with timeout so we can check shutdown periodically
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(event)) => {
                    let dominated_by_config = event.paths.iter().any(|p| {
                        p.file_name()
                            .map(|n| n == config_path.file_name().unwrap_or_default())
                            .unwrap_or(false)
                    });

                    let is_modify =
                        matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));

                    if dominated_by_config && is_modify {
                        let now = std::time::Instant::now();
                        if now.duration_since(last_reload) >= debounce {
                            last_reload = now;
                            match Config::load(None) {
                                Ok(new_config) => {
                                    state.config.store(Arc::new(new_config));
                                    tracing::info!("config reloaded via file watcher");
                                }
                                Err(e) => {
                                    tracing::error!(error=%e, "failed to reload config from file watcher");
                                }
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(error=%e, "file watcher error");
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Just loop around and check shutdown
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::debug!("file watcher channel disconnected");
                    break;
                }
            }
        }

        drop(watcher); // ensure watcher lives until we're done
    });

    Ok(())
}
