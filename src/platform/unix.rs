use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::core::state::AppState;

pub async fn register_signals(shutdown_token: CancellationToken, state: Option<Arc<AppState>>) {
    use tokio::signal::unix::{signal, SignalKind};

    // SIGTERM
    let token = shutdown_token.clone();
    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
        sigterm.recv().await;
        tracing::info!("received SIGTERM, initiating shutdown");
        token.cancel();
    });

    // SIGINT
    let token = shutdown_token.clone();
    tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT");
        sigint.recv().await;
        tracing::info!("received SIGINT, initiating shutdown");
        token.cancel();
    });

    // SIGHUP + SIGUSR1
    if let Some(state) = state {
        // SIGUSR1 - state dump
        let dump_state = state.clone();
        tokio::spawn(async move {
            let mut sigusr1 =
                signal(SignalKind::user_defined1()).expect("failed to register SIGUSR1");
            loop {
                sigusr1.recv().await;
                let config = dump_state.config();
                let server_state = dump_state.get_server_state();
                let active = dump_state.count_sessions().await;
                let total = dump_state
                    .total_sessions
                    .load(std::sync::atomic::Ordering::Relaxed);
                let errors = dump_state
                    .total_errors
                    .load(std::sync::atomic::Ordering::Relaxed);
                let tx = dump_state
                    .total_bytes_tx
                    .load(std::sync::atomic::Ordering::Relaxed);
                let rx = dump_state
                    .total_bytes_rx
                    .load(std::sync::atomic::Ordering::Relaxed);
                tracing::info!(
                    state=?server_state,
                    bind=%format!("{}:{}", config.server.bind_address, config.server.port),
                    root=%config.server.root.display(),
                    active_sessions=%active,
                    total_sessions=%total,
                    total_errors=%errors,
                    bytes_tx=%tx,
                    bytes_rx=%rx,
                    "SIGUSR1 state dump"
                );
                let sessions = dump_state.active_sessions.read().await;
                for (id, info) in sessions.iter() {
                    tracing::info!(
                        session_id=%id,
                        client=%info.client_addr,
                        file=%info.filename,
                        direction=?info.direction,
                        status=?info.status,
                        bytes=%info.bytes_transferred,
                        "active session"
                    );
                }
            }
        });

        // SIGHUP - config reload
        tokio::spawn(async move {
            let mut sighup = signal(SignalKind::hangup()).expect("failed to register SIGHUP");
            loop {
                sighup.recv().await;
                tracing::info!("received SIGHUP, reloading config");
                match state.reload_config() {
                    Ok(()) => {
                        tracing::info!("config reloaded via SIGHUP");
                    }
                    Err(e) => {
                        tracing::error!(error=%e, "failed to reload config on SIGHUP");
                    }
                }
            }
        });
    }
}
