use tokio_util::sync::CancellationToken;

pub async fn register_signals(shutdown_token: CancellationToken) {
    // Ctrl+C — graceful shutdown
    let token = shutdown_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        tracing::info!("received Ctrl+C, initiating graceful shutdown");
        token.cancel();
    });

    // Ctrl+Break — immediate shutdown via native console handler
    let token = shutdown_token.clone();
    tokio::spawn(async move {
        let mut break_signal =
            tokio::signal::windows::ctrl_break().expect("failed to install Ctrl+Break handler");
        break_signal.recv().await;
        tracing::warn!("received Ctrl+Break, immediate shutdown");
        token.cancel();
        // Force exit after a short delay — Ctrl+Break means "now"
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        std::process::exit(1);
    });
}
