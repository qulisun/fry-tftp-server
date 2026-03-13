use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::core::config::Config;
use crate::core::fs;
use crate::core::net::{self, IpVersion};
use crate::core::protocol::packet::*;
use crate::core::state::*;

/// Negotiated session parameters
struct NegotiatedParams {
    blksize: u16,
    windowsize: u16,
    timeout: Duration,
    tsize: Option<u64>,
    has_options: bool,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn ip_version_for_client(client_addr: &SocketAddr, config: &Config) -> IpVersion {
    if client_addr.is_ipv4() {
        IpVersion::V4
    } else {
        IpVersion::from_str(&config.network.ip_version)
    }
}

fn compute_total_blocks(file_size: u64, blksize: usize) -> u64 {
    if file_size == 0 {
        1
    } else if (file_size as usize) % blksize == 0 {
        (file_size as usize / blksize + 1) as u64 // extra empty trailing block
    } else {
        (file_size as usize).div_ceil(blksize) as u64
    }
}

fn compute_backoff(base: Duration, attempt: u32, exponential: bool, max: Duration) -> Duration {
    if !exponential || attempt == 0 {
        return base;
    }
    let factor = 1u64 << attempt.min(8); // 2^attempt, cap at 256x
    let backoff = base.saturating_mul(factor as u32);
    backoff.min(max)
}

/// Serialize a DATA packet directly into `buf`, returning the slice of valid bytes.
/// Avoids allocating a Packet::Data and BytesMut for every block.
fn serialize_data_packet<'a>(buf: &'a mut Vec<u8>, block: u16, payload: &[u8]) -> &'a [u8] {
    buf.clear();
    buf.reserve(4 + payload.len());
    buf.extend_from_slice(&3u16.to_be_bytes()); // opcode DATA = 3
    buf.extend_from_slice(&block.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn get_block_payload(file_data: &[u8], block: u64, blksize: usize) -> &[u8] {
    let offset = ((block - 1) as usize) * blksize;
    let end = std::cmp::min(offset + blksize, file_data.len());
    if offset < file_data.len() {
        &file_data[offset..end]
    } else {
        &[]
    }
}

/// Netascii encoding: LF → CR+LF, bare CR → CR+NUL
pub fn encode_netascii(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 10);
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'\n' => {
                out.push(b'\r');
                out.push(b'\n');
            }
            b'\r' => {
                out.push(b'\r');
                if i + 1 < data.len() && data[i + 1] == b'\n' {
                    out.push(b'\n');
                    i += 1; // skip the LF — already emitted CR+LF
                } else {
                    out.push(0); // bare CR → CR+NUL
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    out
}

/// Netascii decoding: CR+LF → LF, CR+NUL → CR
pub fn decode_netascii(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\r' && i + 1 < data.len() {
            match data[i + 1] {
                b'\n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                0 => {
                    out.push(b'\r');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

// ─── OACK Negotiation ───────────────────────────────────────────────────────

fn negotiate_options(
    config: &Config,
    client_options: &[TftpOption],
    file_size: Option<u64>,
) -> NegotiatedParams {
    let mut blksize = config.protocol.default_blksize;
    let mut windowsize = config.protocol.default_windowsize;
    let mut timeout = Duration::from_secs(config.protocol.default_timeout as u64);
    let mut tsize: Option<u64> = None;
    let mut has_options = false;

    for opt in client_options {
        match opt.name.as_str() {
            "blksize" => {
                if let Ok(requested) = opt.value.parse::<u16>() {
                    blksize = requested.max(8).min(config.protocol.max_blksize);
                    has_options = true;
                }
            }
            "windowsize" => {
                if let Ok(requested) = opt.value.parse::<u16>() {
                    windowsize = requested.max(1).min(config.protocol.max_windowsize);
                    has_options = true;
                }
            }
            "timeout" => {
                if let Ok(requested) = opt.value.parse::<u8>() {
                    let negotiated = requested
                        .max(config.protocol.min_timeout)
                        .min(config.protocol.max_timeout);
                    timeout = Duration::from_secs(negotiated as u64);
                    has_options = true;
                }
            }
            "tsize" => {
                if let Some(size) = file_size {
                    tsize = Some(size);
                    has_options = true;
                }
            }
            _ => {} // unknown options silently ignored
        }
    }

    NegotiatedParams {
        blksize,
        windowsize,
        timeout,
        tsize,
        has_options,
    }
}

fn build_oack(params: &NegotiatedParams) -> Packet {
    let mut options = Vec::new();
    if params.blksize != 512 {
        options.push(TftpOption {
            name: "blksize".to_string(),
            value: params.blksize.to_string(),
        });
    }
    if params.windowsize != 1 {
        options.push(TftpOption {
            name: "windowsize".to_string(),
            value: params.windowsize.to_string(),
        });
    }
    if let Some(tsize) = params.tsize {
        options.push(TftpOption {
            name: "tsize".to_string(),
            value: tsize.to_string(),
        });
    }
    Packet::Oack { options }
}

// ─── OACK handshake (shared by RRQ and WRQ) ────────────────────────────────

/// Send OACK and wait for ACK(0). Returns Ok(()) on success.
async fn oack_handshake(
    socket: &UdpSocket,
    client_addr: SocketAddr,
    params: &NegotiatedParams,
    max_retries: u32,
    cancel: &CancellationToken,
    shutdown: &CancellationToken,
) -> anyhow::Result<()> {
    let oack = build_oack(params);
    let oack_bytes = serialize_packet(&oack);
    socket.send_to(&oack_bytes, client_addr).await?;

    let mut buf = vec![0u8; 512];
    let mut retries = 0u32;
    let mut invalid_packets = 0u32;
    let max_invalid = max_retries * 10; // hard cap on garbage packets
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Err(anyhow::anyhow!("cancelled")),
            _ = shutdown.cancelled() => return Err(anyhow::anyhow!("shutdown")),
            result = tokio::time::timeout(params.timeout, socket.recv_from(&mut buf)) => {
                match result {
                    Ok(Ok((len, from))) => {
                        if from != client_addr {
                            let err = serialize_packet(&Packet::Error { code: ErrorCode::UnknownTransferId, message: "Unknown TID".to_string() });
                            let _ = socket.send_to(&err, from).await;
                            continue;
                        }
                        match parse_packet(&buf[..len]) {
                            Ok(Packet::Ack { block: 0 }) => return Ok(()),
                            Ok(Packet::Error { message, .. }) => {
                                return Err(anyhow::anyhow!("client error: {}", message));
                            }
                            _ => {
                                invalid_packets += 1;
                                if invalid_packets > max_invalid {
                                    return Err(anyhow::anyhow!("OACK aborted: too many invalid packets"));
                                }
                                continue;
                            }
                        }
                    }
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => {
                        retries += 1;
                        if retries > max_retries {
                            return Err(anyhow::anyhow!("OACK timeout after {} retries", max_retries));
                        }
                        socket.send_to(&oack_bytes, client_addr).await?;
                    }
                }
            }
        }
    }
}

// ─── Spawn session helpers ──────────────────────────────────────────────────

fn spawn_session_task(
    state: Arc<AppState>,
    session_id: Uuid,
    filename: String,
    client_addr: SocketAddr,
    future: impl std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
) {
    let state_clone = state.clone();
    tokio::spawn(async move {
        match future.await {
            Ok(()) => {
                tracing::info!(session_id=%session_id, client=%client_addr, file=%filename, event="transfer_complete");
                state_clone
                    .complete_session(session_id, SessionStatus::Completed)
                    .await;
            }
            Err(e) => {
                let msg = e.to_string();
                // Don't log shutdown/cancel as errors
                if msg.contains("cancelled") || msg.contains("shutdown") {
                    state_clone
                        .complete_session(session_id, SessionStatus::Cancelled)
                        .await;
                } else {
                    tracing::warn!(session_id=%session_id, client=%client_addr, file=%filename, error=%e, event="transfer_failed");
                    state_clone
                        .complete_session(session_id, SessionStatus::Failed)
                        .await;
                    state_clone
                        .total_errors
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    });
}

// ─── RRQ (Read) ─────────────────────────────────────────────────────────────

pub async fn spawn_read_session(
    state: Arc<AppState>,
    client_addr: SocketAddr,
    filename: String,
    mode: TransferMode,
    options: Vec<TftpOption>,
    main_socket: &UdpSocket,
) {
    let config = state.config();

    // Resolve file (with virtual roots and symlink policy)
    let vroots = fs::VirtualRoots::new(&config.filesystem.virtual_roots);
    let file_path = match fs::resolve_path_with_virtual(
        &config.server.root,
        &vroots,
        &filename,
        true,
        config.filesystem.follow_symlinks,
    ) {
        Ok(p) => p,
        Err(e) => {
            let code = match &e {
                fs::FsError::FileNotFound(_) => ErrorCode::FileNotFound,
                fs::FsError::AccessViolation(_) => ErrorCode::AccessViolation,
                _ => ErrorCode::NotDefined,
            };
            let err_pkt = serialize_packet(&Packet::Error {
                code,
                message: e.to_string(),
            });
            let _ = main_socket.send_to(&err_pkt, client_addr).await;
            tracing::warn!(client=%client_addr, file=%filename, error=%e, "file resolve failed");
            return;
        }
    };

    let file_size = match std::fs::metadata(&file_path) {
        Ok(m) => m.len(),
        Err(e) => {
            let err_pkt = serialize_packet(&Packet::Error {
                code: ErrorCode::FileNotFound,
                message: e.to_string(),
            });
            let _ = main_socket.send_to(&err_pkt, client_addr).await;
            return;
        }
    };

    let params = negotiate_options(&config, &options, Some(file_size));
    let session_socket =
        match net::create_session_socket(&config, ip_version_for_client(&client_addr, &config)) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error=%e, "failed to create session socket");
                let err_pkt = serialize_packet(&Packet::Error {
                    code: ErrorCode::NotDefined,
                    message: "Server error".to_string(),
                });
                let _ = main_socket.send_to(&err_pkt, client_addr).await;
                return;
            }
        };

    let session_id = Uuid::new_v4();
    let info = SessionInfo {
        id: session_id,
        client_addr,
        filename: filename.clone(),
        direction: Direction::Read,
        status: SessionStatus::Negotiating,
        blksize: params.blksize,
        windowsize: params.windowsize,
        tsize: params.tsize,
        bytes_transferred: 0,
        started_at: Instant::now(),
        last_activity: Instant::now(),
        retransmits: 0,
    };
    state.register_session(info).await;

    let cancel = CancellationToken::new();
    let shutdown = state.get_shutdown_token();
    let state_clone = state.clone();
    let cancel_clone = cancel.clone();

    tracing::info!(
        client=%client_addr, file=%filename, mode=?mode,
        blksize=%params.blksize, windowsize=%params.windowsize,
        event="transfer_start"
    );

    spawn_session_task(state, session_id, filename, client_addr, async move {
        run_read_session(
            &session_socket,
            client_addr,
            &file_path,
            file_size,
            mode,
            &params,
            &state_clone,
            session_id,
            &cancel_clone,
            &shutdown,
        )
        .await
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_read_session(
    socket: &UdpSocket,
    client_addr: SocketAddr,
    file_path: &Path,
    _file_size: u64,
    mode: TransferMode,
    params: &NegotiatedParams,
    state: &Arc<AppState>,
    session_id: Uuid,
    cancel: &CancellationToken,
    shutdown: &CancellationToken,
) -> anyhow::Result<()> {
    let config = state.config();
    let max_retries = config.session.max_retries;
    let exp_backoff = config.session.exponential_backoff;
    let max_timeout = Duration::from_secs(config.protocol.max_timeout as u64);

    // Read file — use mmap for large files, buffered for small files
    let file_handle = fs::FileHandle::open(file_path)?;
    let raw_data = file_handle.as_bytes();
    let file_data: std::borrow::Cow<'_, [u8]> = if mode == TransferMode::Netascii {
        std::borrow::Cow::Owned(encode_netascii(raw_data))
    } else {
        std::borrow::Cow::Borrowed(raw_data)
    };

    // OACK handshake (RRQ: OACK → client ACK(0) → then DATA(1))
    if params.has_options {
        oack_handshake(socket, client_addr, params, max_retries, cancel, shutdown).await?;
    }

    state
        .update_session(session_id, 0, SessionStatus::Transferring)
        .await;

    let blksize = params.blksize as usize;
    let windowsize = params.windowsize as u64;
    let total_blocks = compute_total_blocks(file_data.len() as u64, blksize);
    let mut recv_buf = state.buffer_pool.acquire();
    let mut bytes_sent: u64 = 0;
    let mut base_block: u64 = 1; // first unacknowledged block
    let mut send_buf = Vec::with_capacity(blksize + 4);

    while base_block <= total_blocks {
        // ── Send window ──
        let window_end = std::cmp::min(base_block + windowsize - 1, total_blocks);
        for blk in base_block..=window_end {
            let block_num = (blk & 0xFFFF) as u16;
            let payload = get_block_payload(&file_data, blk, blksize);
            let pkt = serialize_data_packet(&mut send_buf, block_num, payload);
            socket.send_to(pkt, client_addr).await?;
        }

        // ── Wait for ACK ──
        let mut retries = 0u32;
        loop {
            let timeout = compute_backoff(params.timeout, retries, exp_backoff, max_timeout);
            tokio::select! {
                _ = cancel.cancelled() => return Err(anyhow::anyhow!("cancelled")),
                _ = shutdown.cancelled() => return Err(anyhow::anyhow!("shutdown")),
                result = tokio::time::timeout(timeout, socket.recv_from(&mut recv_buf)) => {
                    match result {
                        Ok(Ok((len, from))) => {
                            if from != client_addr {
                                let err = serialize_packet(&Packet::Error { code: ErrorCode::UnknownTransferId, message: "Unknown TID".to_string() });
                                let _ = socket.send_to(&err, from).await;
                                continue;
                            }
                            match parse_packet(&recv_buf[..len]) {
                                Ok(Packet::Ack { block }) => {
                                    let ack_abs = block_to_absolute(block, base_block);

                                    if ack_abs >= base_block && ack_abs <= window_end {
                                        // Update bytes_sent with actual payload sizes
                                        let mut acked_bytes: u64 = 0;
                                        for blk in base_block..=ack_abs {
                                            let payload_len = get_block_payload(&file_data, blk, blksize).len() as u64;
                                            bytes_sent += payload_len;
                                            acked_bytes += payload_len;
                                        }
                                        state.total_bytes_tx.fetch_add(
                                            acked_bytes,
                                            std::sync::atomic::Ordering::Relaxed,
                                        );
                                        state.update_session(session_id, bytes_sent, SessionStatus::Transferring).await;

                                        base_block = ack_abs + 1;
                                        break; // send next window
                                    } else if ack_abs < base_block {
                                        // Sorcerer's Apprentice: duplicate ACK → IGNORE
                                        continue;
                                    } else {
                                        // ACK beyond window — anomalous, ignore
                                        continue;
                                    }
                                }
                                Ok(Packet::Error { code, message }) => {
                                    return Err(anyhow::anyhow!("client error {:?}: {}", code, message));
                                }
                                _ => continue,
                            }
                        }
                        Ok(Err(e)) => return Err(e.into()),
                        Err(_) => {
                            // Timeout — retransmit entire window
                            retries += 1;
                            if retries > max_retries {
                                return Err(anyhow::anyhow!(
                                    "timeout after {} retries at block {}",
                                    max_retries, base_block
                                ));
                            }
                            for blk in base_block..=window_end {
                                let block_num = (blk & 0xFFFF) as u16;
                                let payload = get_block_payload(&file_data, blk, blksize);
                                let pkt = serialize_data_packet(&mut send_buf, block_num, payload);
                                socket.send_to(pkt, client_addr).await?;
                            }
                        }
                    }
                }
            }
        }
    }

    state.buffer_pool.release(recv_buf);
    Ok(())
}

/// Convert a u16 block number to absolute block number, accounting for rollover.
/// Uses `base_block` as reference to determine the correct epoch.
fn block_to_absolute(block_u16: u16, base_block: u64) -> u64 {
    let epoch = base_block / 65536;
    let candidate = epoch * 65536 + block_u16 as u64;
    if candidate < base_block {
        candidate + 65536
    } else {
        candidate
    }
}

// ─── WRQ (Write) ────────────────────────────────────────────────────────────

pub async fn spawn_write_session(
    state: Arc<AppState>,
    client_addr: SocketAddr,
    filename: String,
    mode: TransferMode,
    options: Vec<TftpOption>,
    main_socket: &UdpSocket,
) {
    let config = state.config();

    // Check if file already exists (with virtual roots and symlink policy)
    let vroots = fs::VirtualRoots::new(&config.filesystem.virtual_roots);
    let file_path = match fs::resolve_path_with_virtual(
        &config.server.root,
        &vroots,
        &filename,
        false,
        config.filesystem.follow_symlinks,
    ) {
        Ok(p) => p,
        Err(e) => {
            let err_pkt = serialize_packet(&Packet::Error {
                code: ErrorCode::AccessViolation,
                message: e.to_string(),
            });
            let _ = main_socket.send_to(&err_pkt, client_addr).await;
            return;
        }
    };

    // Existence check moved to atomic write (OpenOptions::create_new) in run_write_session
    // to avoid TOCTOU race. We still do a quick check here for early rejection.
    if !config.filesystem.allow_overwrite && file_path.exists() {
        let err_pkt = serialize_packet(&Packet::Error {
            code: ErrorCode::FileAlreadyExists,
            message: "File already exists".to_string(),
        });
        let _ = main_socket.send_to(&err_pkt, client_addr).await;
        return;
    }

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        if !parent.exists() {
            if config.filesystem.create_dirs {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    let err_pkt = serialize_packet(&Packet::Error {
                        code: ErrorCode::NotDefined,
                        message: e.to_string(),
                    });
                    let _ = main_socket.send_to(&err_pkt, client_addr).await;
                    return;
                }
            } else {
                let err_pkt = serialize_packet(&Packet::Error {
                    code: ErrorCode::FileNotFound,
                    message: "Parent directory does not exist".to_string(),
                });
                let _ = main_socket.send_to(&err_pkt, client_addr).await;
                return;
            }
        }
    }

    let params = negotiate_options(&config, &options, None);

    // Check tsize against max_file_size if client declared it
    let max_bytes = config.filesystem.max_file_size_bytes();
    if let Some(tsize) = params.tsize {
        if tsize > max_bytes {
            let err_pkt = serialize_packet(&Packet::Error {
                code: ErrorCode::DiskFull,
                message: format!(
                    "File too large: {} > {}",
                    tsize, config.filesystem.max_file_size
                ),
            });
            let _ = main_socket.send_to(&err_pkt, client_addr).await;
            return;
        }
    }

    let session_socket =
        match net::create_session_socket(&config, ip_version_for_client(&client_addr, &config)) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error=%e, "failed to create session socket");
                let err_pkt = serialize_packet(&Packet::Error {
                    code: ErrorCode::NotDefined,
                    message: "Server error".to_string(),
                });
                let _ = main_socket.send_to(&err_pkt, client_addr).await;
                return;
            }
        };

    let session_id = Uuid::new_v4();
    let info = SessionInfo {
        id: session_id,
        client_addr,
        filename: filename.clone(),
        direction: Direction::Write,
        status: SessionStatus::Negotiating,
        blksize: params.blksize,
        windowsize: params.windowsize,
        tsize: params.tsize,
        bytes_transferred: 0,
        started_at: Instant::now(),
        last_activity: Instant::now(),
        retransmits: 0,
    };
    state.register_session(info).await;

    let cancel = CancellationToken::new();
    let shutdown = state.get_shutdown_token();
    let state_clone = state.clone();
    let cancel_clone = cancel.clone();

    tracing::info!(client=%client_addr, file=%filename, mode=?mode, event="write_start");

    spawn_session_task(state, session_id, filename, client_addr, async move {
        run_write_session(
            &session_socket,
            client_addr,
            &file_path,
            mode,
            &params,
            &state_clone,
            session_id,
            &cancel_clone,
            &shutdown,
        )
        .await
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_write_session(
    socket: &UdpSocket,
    client_addr: SocketAddr,
    file_path: &Path,
    mode: TransferMode,
    params: &NegotiatedParams,
    state: &Arc<AppState>,
    session_id: Uuid,
    cancel: &CancellationToken,
    shutdown: &CancellationToken,
) -> anyhow::Result<()> {
    let config = state.config();
    let max_retries = config.session.max_retries;
    let exp_backoff = config.session.exponential_backoff;
    let max_timeout = Duration::from_secs(config.protocol.max_timeout as u64);
    let blksize = params.blksize as usize;
    let windowsize = params.windowsize as u64;
    let max_file_bytes = config.filesystem.max_file_size_bytes();

    // WRQ with options: send OACK, client responds with DATA(1) (NOT ACK(0)!)
    // WRQ without options: send ACK(0), client responds with DATA(1)
    if params.has_options {
        let oack = build_oack(params);
        let oack_bytes = serialize_packet(&oack);
        socket.send_to(&oack_bytes, client_addr).await?;
        // Do NOT wait for ACK(0) — client sends DATA(1) directly after OACK
    } else {
        let ack0 = serialize_packet(&Packet::Ack { block: 0 });
        socket.send_to(&ack0, client_addr).await?;
    }

    state
        .update_session(session_id, 0, SessionStatus::Transferring)
        .await;

    let mut received_data: Vec<u8> = Vec::new();
    let mut expected_block: u64 = 1;
    let mut recv_buf = state.buffer_pool.acquire();
    let mut bytes_received: u64 = 0;
    let mut window_buf: BTreeMap<u64, Vec<u8>> = BTreeMap::new();

    loop {
        let mut retries = 0u32;

        // Receive blocks for the current window
        let window_end = expected_block + windowsize - 1;

        loop {
            let timeout = compute_backoff(params.timeout, retries, exp_backoff, max_timeout);
            tokio::select! {
                _ = cancel.cancelled() => return Err(anyhow::anyhow!("cancelled")),
                _ = shutdown.cancelled() => return Err(anyhow::anyhow!("shutdown")),
                result = tokio::time::timeout(timeout, socket.recv_from(&mut recv_buf)) => {
                    match result {
                        Ok(Ok((len, from))) => {
                            if from != client_addr {
                                let err = serialize_packet(&Packet::Error { code: ErrorCode::UnknownTransferId, message: "Unknown TID".to_string() });
                                let _ = socket.send_to(&err, from).await;
                                continue;
                            }
                            match parse_packet(&recv_buf[..len]) {
                                Ok(Packet::Data { block, data }) => {
                                    let abs_block = block_to_absolute(block, expected_block);

                                    if abs_block >= expected_block && abs_block <= window_end {
                                        let is_last = data.len() < blksize;
                                        window_buf.insert(abs_block, data);

                                        // Flush contiguous blocks
                                        while window_buf.contains_key(&expected_block) {
                                            let blk_data = window_buf.remove(&expected_block).unwrap();
                                            bytes_received += blk_data.len() as u64;
                                            received_data.extend_from_slice(&blk_data);
                                            expected_block += 1;
                                        }

                                        // Enforce max_file_size
                                        if bytes_received > max_file_bytes {
                                            let err = serialize_packet(&Packet::Error {
                                                code: ErrorCode::DiskFull,
                                                message: "File size exceeds limit".to_string(),
                                            });
                                            let _ = socket.send_to(&err, client_addr).await;
                                            return Err(anyhow::anyhow!("file size {} exceeds max {}", bytes_received, max_file_bytes));
                                        }

                                        state.total_bytes_rx.fetch_add(
                                            (len - 4) as u64,
                                            std::sync::atomic::Ordering::Relaxed,
                                        );
                                        state.update_session(session_id, bytes_received, SessionStatus::Transferring).await;

                                        if is_last && window_buf.is_empty() {
                                            // Send final ACK
                                            let ack = serialize_packet(&Packet::Ack { block });
                                            socket.send_to(&ack, client_addr).await?;

                                            // Apply netascii decoding
                                            let final_data = if mode == TransferMode::Netascii {
                                                decode_netascii(&received_data)
                                            } else {
                                                received_data
                                            };

                                            // Write file — use create_new to prevent TOCTOU race
                                            // when allow_overwrite=false
                                            let write_path = file_path.to_path_buf();
                                            let allow_overwrite = config.filesystem.allow_overwrite;
                                            tokio::task::spawn_blocking(move || {
                                                use std::io::Write;
                                                let mut opts = std::fs::OpenOptions::new();
                                                opts.write(true);
                                                if allow_overwrite {
                                                    opts.create(true).truncate(true);
                                                } else {
                                                    opts.create_new(true);
                                                }
                                                let mut file = opts.open(&write_path)?;
                                                file.write_all(&final_data)?;
                                                file.sync_all()?;
                                                std::io::Result::Ok(())
                                            }).await??;
                                            state.buffer_pool.release(recv_buf);
                                            return Ok(());
                                        }

                                        // If we've received all blocks in window, ACK and move on
                                        if expected_block > window_end {
                                            let ack_block = ((expected_block - 1) & 0xFFFF) as u16;
                                            let ack = serialize_packet(&Packet::Ack { block: ack_block });
                                            socket.send_to(&ack, client_addr).await?;
                                            break; // window complete, continue outer loop
                                        }
                                    }
                                    // Else: duplicate or out-of-range block, ignore
                                }
                                Ok(Packet::Error { code, message }) => {
                                    return Err(anyhow::anyhow!("client error {:?}: {}", code, message));
                                }
                                _ => continue,
                            }
                        }
                        Ok(Err(e)) => return Err(e.into()),
                        Err(_) => {
                            // Timeout — ACK last contiguous block to trigger retransmit
                            retries += 1;
                            if retries > max_retries {
                                return Err(anyhow::anyhow!("timeout after {} retries at block {}", max_retries, expected_block));
                            }
                            if expected_block > 1 {
                                let ack_block = ((expected_block - 1) & 0xFFFF) as u16;
                                let ack = serialize_packet(&Packet::Ack { block: ack_block });
                                socket.send_to(&ack, client_addr).await?;
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_netascii() {
        // LF → CR+LF
        assert_eq!(encode_netascii(b"a\nb"), b"a\r\nb");
        // CR → CR+NUL
        assert_eq!(encode_netascii(b"a\rb"), b"a\r\0b");
        // CR+LF stays CR+LF
        assert_eq!(encode_netascii(b"a\r\nb"), b"a\r\nb");
        // No change
        assert_eq!(encode_netascii(b"hello"), b"hello");
    }

    #[test]
    fn test_decode_netascii() {
        assert_eq!(decode_netascii(b"a\r\nb"), b"a\nb");
        assert_eq!(decode_netascii(b"a\r\0b"), b"a\rb");
        assert_eq!(decode_netascii(b"hello"), b"hello");
    }

    #[test]
    fn test_netascii_roundtrip() {
        // Native text with LF and bare CR (no CR+LF since that's wire format)
        let native = b"line1\nline2\rline3";
        let encoded = encode_netascii(native);
        assert_eq!(encoded, b"line1\r\nline2\r\0line3");
        let decoded = decode_netascii(&encoded);
        assert_eq!(decoded, native);
    }

    #[test]
    fn test_compute_total_blocks() {
        assert_eq!(compute_total_blocks(0, 512), 1);
        assert_eq!(compute_total_blocks(1, 512), 1);
        assert_eq!(compute_total_blocks(512, 512), 2); // extra empty block
        assert_eq!(compute_total_blocks(513, 512), 2);
        assert_eq!(compute_total_blocks(1024, 512), 3);
        assert_eq!(compute_total_blocks(1536, 512), 4);
    }

    #[test]
    fn test_compute_backoff() {
        let base = Duration::from_secs(3);
        let max = Duration::from_secs(30);
        assert_eq!(compute_backoff(base, 0, true, max), Duration::from_secs(3));
        assert_eq!(compute_backoff(base, 1, true, max), Duration::from_secs(6));
        assert_eq!(compute_backoff(base, 2, true, max), Duration::from_secs(12));
        assert_eq!(compute_backoff(base, 3, true, max), Duration::from_secs(24));
        assert_eq!(compute_backoff(base, 4, true, max), Duration::from_secs(30)); // capped
                                                                                  // No backoff
        assert_eq!(compute_backoff(base, 3, false, max), Duration::from_secs(3));
    }

    #[test]
    fn test_block_to_absolute() {
        assert_eq!(block_to_absolute(1, 1), 1);
        assert_eq!(block_to_absolute(100, 50), 100);
        assert_eq!(block_to_absolute(0, 65536), 65536); // rollover: block 0 in second epoch
        assert_eq!(block_to_absolute(1, 65536), 65537);
    }
}
