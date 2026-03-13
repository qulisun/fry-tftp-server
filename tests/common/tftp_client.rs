//! Reusable TFTP test client for integration tests.
//!
//! Provides both high-level (get/put) and low-level (send_rrq/recv_packet) APIs
//! for testing the TFTP server with full protocol support including OACK flows.

use std::net::SocketAddr;
use std::time::Duration;

use fry_tftp_server::core::protocol::packet::*;
use tokio::net::UdpSocket;

/// Options to negotiate with the server.
#[derive(Debug, Clone, Default)]
pub struct TftpOptions {
    pub blksize: Option<u16>,
    pub windowsize: Option<u16>,
    pub timeout: Option<u8>,
    pub tsize: Option<u64>,
}

impl TftpOptions {
    pub fn to_tftp_options(&self) -> Vec<TftpOption> {
        let mut opts = Vec::new();
        if let Some(blksize) = self.blksize {
            opts.push(TftpOption {
                name: "blksize".to_string(),
                value: blksize.to_string(),
            });
        }
        if let Some(windowsize) = self.windowsize {
            opts.push(TftpOption {
                name: "windowsize".to_string(),
                value: windowsize.to_string(),
            });
        }
        if let Some(timeout) = self.timeout {
            opts.push(TftpOption {
                name: "timeout".to_string(),
                value: timeout.to_string(),
            });
        }
        if let Some(tsize) = self.tsize {
            opts.push(TftpOption {
                name: "tsize".to_string(),
                value: tsize.to_string(),
            });
        }
        opts
    }

    pub fn has_options(&self) -> bool {
        self.blksize.is_some()
            || self.windowsize.is_some()
            || self.timeout.is_some()
            || self.tsize.is_some()
    }
}

/// Result of an OACK negotiation — the negotiated values returned by the server.
#[derive(Debug, Clone)]
pub struct NegotiatedOptions {
    pub blksize: Option<u16>,
    pub windowsize: Option<u16>,
    pub timeout: Option<u8>,
    pub tsize: Option<u64>,
}

impl NegotiatedOptions {
    pub fn from_oack(options: &[TftpOption]) -> Self {
        let mut result = NegotiatedOptions {
            blksize: None,
            windowsize: None,
            timeout: None,
            tsize: None,
        };
        for opt in options {
            match opt.name.as_str() {
                "blksize" => result.blksize = opt.value.parse().ok(),
                "windowsize" => result.windowsize = opt.value.parse().ok(),
                "timeout" => result.timeout = opt.value.parse().ok(),
                "tsize" => result.tsize = opt.value.parse().ok(),
                _ => {}
            }
        }
        result
    }
}

/// A reusable TFTP test client for integration tests.
pub struct TftpTestClient {
    socket: UdpSocket,
    server_addr: SocketAddr,
    /// Default recv timeout
    pub recv_timeout: Duration,
}

#[allow(dead_code)]
impl TftpTestClient {
    /// Create a new test client targeting the given server address.
    pub async fn new(server_addr: SocketAddr) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        Self {
            socket,
            server_addr,
            recv_timeout: Duration::from_secs(5),
        }
    }

    // ─── High-level operations ──────────────────────────────────────────────

    /// GET a file from the server (no options).
    pub async fn get(&self, filename: &str) -> Result<Vec<u8>, String> {
        self.get_with_options(filename, &TftpOptions::default())
            .await
    }

    /// GET a file from the server with options. Returns (data, negotiated_options).
    pub async fn get_with_options(
        &self,
        filename: &str,
        opts: &TftpOptions,
    ) -> Result<Vec<u8>, String> {
        self.send_rrq(filename, &opts.to_tftp_options()).await?;

        let blksize = opts.blksize.unwrap_or(512) as usize;

        // If options were sent, expect OACK first
        if opts.has_options() {
            let pkt = self.recv_packet(self.recv_timeout).await?;
            match pkt {
                ReceivedPacket {
                    packet: Packet::Oack { .. },
                    from,
                } => {
                    // ACK(0) to confirm OACK
                    self.send_ack_to(0, from).await?;
                }
                ReceivedPacket {
                    packet: Packet::Data { block: 1, data },
                    from,
                } => {
                    // Server might skip OACK if no options accepted — handle inline
                    let is_last = data.len() < blksize;
                    self.send_ack_to(1, from).await?;
                    if is_last {
                        return Ok(data);
                    }
                    // Continue receiving from block 2
                    return self.recv_data_blocks(2, blksize, data, from).await;
                }
                ReceivedPacket {
                    packet: Packet::Error { code, message },
                    ..
                } => {
                    return Err(format!("server error {:?}: {}", code, message));
                }
                other => return Err(format!("expected OACK or DATA(1), got {:?}", other.packet)),
            }
        }

        // Receive DATA blocks
        let first = self.recv_packet(self.recv_timeout).await?;
        match first {
            ReceivedPacket {
                packet: Packet::Data { block: 1, data },
                from,
            } => {
                let is_last = data.len() < blksize;
                self.send_ack_to(1, from).await?;
                if is_last {
                    return Ok(data);
                }
                self.recv_data_blocks(2, blksize, data, from).await
            }
            ReceivedPacket {
                packet: Packet::Error { code, message },
                ..
            } => Err(format!("server error {:?}: {}", code, message)),
            other => Err(format!("expected DATA(1), got {:?}", other.packet)),
        }
    }

    /// PUT a file to the server (no options).
    pub async fn put(&self, filename: &str, data: &[u8]) -> Result<(), String> {
        self.put_with_options(filename, data, &TftpOptions::default())
            .await
    }

    /// PUT a file to the server with options.
    pub async fn put_with_options(
        &self,
        filename: &str,
        data: &[u8],
        opts: &TftpOptions,
    ) -> Result<(), String> {
        self.send_wrq(filename, &opts.to_tftp_options()).await?;

        let blksize = opts.blksize.unwrap_or(512) as usize;

        // If options were sent, expect OACK; otherwise expect ACK(0)
        let first = self.recv_packet(self.recv_timeout).await?;
        let from = first.from;
        match first.packet {
            Packet::Oack { .. } => {
                // WRQ with OACK: send DATA(1) directly (no ACK(0)!)
            }
            Packet::Ack { block: 0 } => {
                // WRQ without OACK: ACK(0) received, send DATA(1)
            }
            Packet::Error { code, message } => {
                return Err(format!("server error {:?}: {}", code, message));
            }
            other => return Err(format!("expected OACK or ACK(0), got {:?}", other)),
        }

        // Send DATA blocks
        let chunks: Vec<&[u8]> = data.chunks(blksize).collect();
        let total_chunks = if data.is_empty() { 1 } else { chunks.len() };
        let needs_empty_final = !data.is_empty() && data.len() % blksize == 0;

        for i in 0..total_chunks {
            let block = (i + 1) as u16;
            let chunk = if data.is_empty() {
                &[] as &[u8]
            } else {
                chunks[i]
            };
            self.send_data_to(block, chunk, from).await?;

            let ack = self.recv_packet(self.recv_timeout).await?;
            match ack.packet {
                Packet::Ack { block: b } if b == block => {}
                Packet::Error { code, message } => {
                    return Err(format!("server error {:?}: {}", code, message));
                }
                other => return Err(format!("expected ACK({}), got {:?}", block, other)),
            }
        }

        // Send trailing empty block if data is exact multiple of blksize
        if needs_empty_final {
            let block = (total_chunks + 1) as u16;
            self.send_data_to(block, &[], from).await?;
            let ack = self.recv_packet(self.recv_timeout).await?;
            match ack.packet {
                Packet::Ack { block: b } if b == block => {}
                other => return Err(format!("expected final ACK({}), got {:?}", block, other)),
            }
        }

        Ok(())
    }

    // ─── Low-level operations ───────────────────────────────────────────────

    /// Send a RRQ to the server.
    pub async fn send_rrq(&self, filename: &str, options: &[TftpOption]) -> Result<(), String> {
        let pkt = serialize_packet(&Packet::Rrq {
            filename: filename.to_string(),
            mode: TransferMode::Octet,
            options: options.to_vec(),
        });
        self.socket
            .send_to(&pkt, self.server_addr)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Send a WRQ to the server.
    pub async fn send_wrq(&self, filename: &str, options: &[TftpOption]) -> Result<(), String> {
        let pkt = serialize_packet(&Packet::Wrq {
            filename: filename.to_string(),
            mode: TransferMode::Octet,
            options: options.to_vec(),
        });
        self.socket
            .send_to(&pkt, self.server_addr)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Send ACK to the server address (main socket).
    pub async fn send_ack(&self, block: u16) -> Result<(), String> {
        self.send_ack_to(block, self.server_addr).await
    }

    /// Send ACK to a specific address (session socket).
    pub async fn send_ack_to(&self, block: u16, addr: SocketAddr) -> Result<(), String> {
        let pkt = serialize_packet(&Packet::Ack { block });
        self.socket
            .send_to(&pkt, addr)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Send DATA to a specific address.
    pub async fn send_data_to(
        &self,
        block: u16,
        data: &[u8],
        addr: SocketAddr,
    ) -> Result<(), String> {
        let pkt = serialize_packet(&Packet::Data {
            block,
            data: data.to_vec(),
        });
        self.socket
            .send_to(&pkt, addr)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Send a raw packet to the server.
    pub async fn send_raw(&self, data: &[u8]) -> Result<(), String> {
        self.socket
            .send_to(data, self.server_addr)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Receive a single packet with timeout.
    pub async fn recv_packet(&self, timeout: Duration) -> Result<ReceivedPacket, String> {
        let mut buf = vec![0u8; 65536];
        let (len, from) = tokio::time::timeout(timeout, self.socket.recv_from(&mut buf))
            .await
            .map_err(|_| "timeout waiting for packet".to_string())?
            .map_err(|e| e.to_string())?;
        let packet = parse_packet(&buf[..len]).map_err(|e| format!("parse error: {}", e))?;
        Ok(ReceivedPacket { packet, from })
    }

    /// Try to receive a packet, returning None on timeout.
    pub async fn try_recv_packet(
        &self,
        timeout: Duration,
    ) -> Result<Option<ReceivedPacket>, String> {
        let mut buf = vec![0u8; 65536];
        match tokio::time::timeout(timeout, self.socket.recv_from(&mut buf)).await {
            Ok(Ok((len, from))) => {
                let packet =
                    parse_packet(&buf[..len]).map_err(|e| format!("parse error: {}", e))?;
                Ok(Some(ReceivedPacket { packet, from }))
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Ok(None), // timeout — no packet
        }
    }

    // ─── Simulation helpers ─────────────────────────────────────────────────

    /// Send duplicate ACKs (for Sorcerer's Apprentice testing).
    pub async fn send_duplicate_acks(
        &self,
        block: u16,
        count: usize,
        addr: SocketAddr,
    ) -> Result<(), String> {
        for _ in 0..count {
            self.send_ack_to(block, addr).await?;
        }
        Ok(())
    }

    // ─── Private helpers ────────────────────────────────────────────────────

    async fn recv_data_blocks(
        &self,
        start_block: u16,
        blksize: usize,
        mut collected: Vec<u8>,
        session_addr: SocketAddr,
    ) -> Result<Vec<u8>, String> {
        let mut expected = start_block;
        loop {
            let pkt = self.recv_packet(self.recv_timeout).await?;
            match pkt.packet {
                Packet::Data { block, data } => {
                    // With sliding window, we may receive blocks in order but
                    // must ACK each one to advance. Accept current expected block.
                    if block != expected {
                        // Could be windowed: just accept any block >= expected
                        // and ACK it to keep the transfer going
                        if block > expected || (block < expected && expected > 100) {
                            // Out-of-order or rollover — skip to this block
                        }
                    }
                    let is_last = data.len() < blksize;
                    collected.extend_from_slice(&data);
                    self.send_ack_to(block, session_addr).await?;
                    if is_last {
                        return Ok(collected);
                    }
                    expected = block.wrapping_add(1);
                }
                Packet::Error { code, message } => {
                    return Err(format!("server error {:?}: {}", code, message));
                }
                other => return Err(format!("expected DATA({}), got {:?}", expected, other)),
            }
        }
    }
}

/// A received packet along with the sender address.
#[derive(Debug)]
pub struct ReceivedPacket {
    pub packet: Packet,
    pub from: SocketAddr,
}

// ─── Mini server helper ─────────────────────────────────────────────────────

use std::path::PathBuf;
use std::sync::Arc;

use fry_tftp_server::core::config::Config;
use fry_tftp_server::core::state::AppState;

/// Spawn a mini server for testing that handles one request.
/// Returns (client, server_addr, state, server_handle).
pub async fn mini_server(
    root: PathBuf,
    config_override: impl FnOnce(&mut Config),
) -> (
    TftpTestClient,
    SocketAddr,
    Arc<AppState>,
    tokio::task::JoinHandle<()>,
) {
    let mut config = Config::default();
    config.server.port = 0;
    config.server.root = root;
    config.server.log_file = String::new();
    config.server.log_level = "warn".to_string();
    config.network.ip_version = "v4".to_string();
    config_override(&mut config);

    let server_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    let state = AppState::new(config);
    let state_clone = state.clone();

    let handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        let (len, client_addr) = server_socket.recv_from(&mut buf).await.unwrap();
        let packet = match parse_packet(&buf[..len]) {
            Ok(p) => p,
            Err(e) => {
                let err_pkt = serialize_packet(&Packet::Error {
                    code: ErrorCode::NotDefined,
                    message: format!("{}", e),
                });
                let _ = server_socket.send_to(&err_pkt, client_addr).await;
                tokio::time::sleep(Duration::from_secs(1)).await;
                return;
            }
        };

        let config = state_clone.config();
        match packet {
            Packet::Rrq {
                filename,
                mode,
                options,
            } => {
                fry_tftp_server::core::session::spawn_read_session(
                    state_clone.clone(),
                    client_addr,
                    filename,
                    mode,
                    options,
                    &server_socket,
                )
                .await;
            }
            Packet::Wrq {
                filename,
                mode,
                options,
            } => {
                if !config.protocol.allow_write {
                    let err_pkt = serialize_packet(&Packet::Error {
                        code: ErrorCode::AccessViolation,
                        message: "Write not allowed".to_string(),
                    });
                    let _ = server_socket.send_to(&err_pkt, client_addr).await;
                } else {
                    fry_tftp_server::core::session::spawn_write_session(
                        state_clone.clone(),
                        client_addr,
                        filename,
                        mode,
                        options,
                        &server_socket,
                    )
                    .await;
                }
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let client = TftpTestClient::new(server_addr).await;
    (client, server_addr, state, handle)
}

/// Spawn a multi-request mini server (handles N requests before stopping).
pub async fn mini_server_multi(
    root: PathBuf,
    max_requests: usize,
    config_override: impl FnOnce(&mut Config),
) -> (SocketAddr, Arc<AppState>, tokio::task::JoinHandle<()>) {
    let mut config = Config::default();
    config.server.port = 0;
    config.server.root = root;
    config.server.log_file = String::new();
    config.server.log_level = "warn".to_string();
    config.network.ip_version = "v4".to_string();
    config_override(&mut config);

    let server_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    let state = AppState::new(config);
    let state_clone = state.clone();

    let handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        for _ in 0..max_requests {
            let result =
                tokio::time::timeout(Duration::from_secs(30), server_socket.recv_from(&mut buf))
                    .await;
            let (len, client_addr) = match result {
                Ok(Ok(v)) => v,
                _ => break,
            };
            let packet = match parse_packet(&buf[..len]) {
                Ok(p) => p,
                Err(e) => {
                    let err_pkt = serialize_packet(&Packet::Error {
                        code: ErrorCode::NotDefined,
                        message: format!("{}", e),
                    });
                    let _ = server_socket.send_to(&err_pkt, client_addr).await;
                    continue;
                }
            };

            let config = state_clone.config();
            match packet {
                Packet::Rrq {
                    filename,
                    mode,
                    options,
                } => {
                    fry_tftp_server::core::session::spawn_read_session(
                        state_clone.clone(),
                        client_addr,
                        filename,
                        mode,
                        options,
                        &server_socket,
                    )
                    .await;
                    // Wait for session to complete before handling next request
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Packet::Wrq {
                    filename,
                    mode,
                    options,
                } => {
                    if !config.protocol.allow_write {
                        let err_pkt = serialize_packet(&Packet::Error {
                            code: ErrorCode::AccessViolation,
                            message: "Write not allowed".to_string(),
                        });
                        let _ = server_socket.send_to(&err_pkt, client_addr).await;
                    } else {
                        fry_tftp_server::core::session::spawn_write_session(
                            state_clone.clone(),
                            client_addr,
                            filename,
                            mode,
                            options,
                            &server_socket,
                        )
                        .await;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    (server_addr, state, handle)
}
