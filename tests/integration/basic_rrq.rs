#[path = "../common/tftp_client.rs"]
mod tftp_client;
use tftp_client::*;

use std::sync::Arc;
use std::time::Duration;

use fry_tftp_server::core::protocol::packet::*;

/// Return the canonical path of a temp directory.
/// On macOS `/var` is a symlink to `/private/var`, which trips the
/// server's symlink check.  Canonicalizing up-front makes the tests
/// work on every platform.
fn canonical_temp_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path()
        .canonicalize()
        .expect("failed to canonicalize temp dir")
}

// ═══════════════════════════════════════════════════════════════════════════════
// Packet roundtrip tests (unit-level)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_packet_roundtrip_rrq() {
    let original = Packet::Rrq {
        filename: "firmware.bin".to_string(),
        mode: TransferMode::Octet,
        options: vec![
            TftpOption {
                name: "blksize".to_string(),
                value: "1468".to_string(),
            },
            TftpOption {
                name: "tsize".to_string(),
                value: "0".to_string(),
            },
        ],
    };
    let bytes = serialize_packet(&original);
    let parsed = parse_packet(&bytes).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn test_netascii_encode_decode() {
    use fry_tftp_server::core::session::{decode_netascii, encode_netascii};
    let native = b"line1\nline2\rline3";
    let encoded = encode_netascii(native);
    let decoded = decode_netascii(&encoded);
    assert_eq!(decoded, native);
    assert!(encoded.len() >= native.len());
}

#[test]
fn test_path_traversal_patterns() {
    use fry_tftp_server::core::fs::resolve_path;
    let root = std::path::PathBuf::from(if cfg!(windows) {
        "C:\\TFTP"
    } else {
        "/tmp/test-root"
    });

    let bad_paths = &[
        "../etc/passwd",
        "..\\windows\\system32",
        "subdir/../../etc/shadow",
        "....//....//etc/passwd",
        "foo/../../../bar",
    ];

    for path in bad_paths {
        let result = resolve_path(&root, path, false, false);
        assert!(
            result.is_err(),
            "path '{}' should be rejected, got {:?}",
            path,
            result
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// RRQ tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rrq_localhost() {
    let dir = tempfile::tempdir().unwrap();
    let test_data: Vec<u8> = (0..1536).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.path().join("data.bin"), &test_data).unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |_| {}).await;

    let received = client.get("data.bin").await.unwrap();
    assert_eq!(received, test_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_rrq_zero_byte_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("empty.bin"), b"").unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |_| {}).await;

    let received = client.get("empty.bin").await.unwrap();
    assert!(received.is_empty());

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_rrq_file_not_found() {
    let dir = tempfile::tempdir().unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |_| {}).await;

    let result = client.get("nonexistent.bin").await;
    assert!(result.is_err());

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_rrq_with_blksize() {
    let dir = tempfile::tempdir().unwrap();
    let test_data: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.path().join("big.bin"), &test_data).unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |_| {}).await;

    let opts = TftpOptions {
        blksize: Some(1024),
        ..Default::default()
    };
    let received = client.get_with_options("big.bin", &opts).await.unwrap();
    assert_eq!(received, test_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_rrq_sliding_window() {
    let dir = tempfile::tempdir().unwrap();
    let test_data: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.path().join("win.bin"), &test_data).unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |c| {
        c.protocol.max_windowsize = 4;
    })
    .await;

    // Use low-level API for sliding window — ACK only every windowsize-th or last block
    let windowsize = 4u16;
    client
        .send_rrq(
            "win.bin",
            &[TftpOption {
                name: "windowsize".to_string(),
                value: windowsize.to_string(),
            }],
        )
        .await
        .unwrap();

    // OACK
    let oack = client.recv_packet(Duration::from_secs(5)).await.unwrap();
    assert!(matches!(&oack.packet, Packet::Oack { .. }));
    client.send_ack_to(0, oack.from).await.unwrap();

    // Receive all DATA blocks, ACK every 4th or last
    let mut received = Vec::new();
    let mut expected = 1u16;
    loop {
        let pkt = client.recv_packet(Duration::from_secs(5)).await.unwrap();
        if let Packet::Data { block, data } = pkt.packet {
            assert_eq!(block, expected);
            let is_last = data.len() < 512;
            received.extend_from_slice(&data);
            if block % windowsize == 0 || is_last {
                client.send_ack_to(block, pkt.from).await.unwrap();
            }
            if is_last {
                break;
            }
            expected = expected.wrapping_add(1);
        }
    }
    assert_eq!(received, test_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_rrq_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |_| {}).await;

    let result = client.get("../etc/passwd").await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("path traversal") || err_msg.contains("error"),
        "error should mention path traversal, got: {}",
        err_msg
    );

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// WRQ tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_wrq_basic() {
    let dir = tempfile::tempdir().unwrap();
    let upload_data: Vec<u8> = (0..1500).map(|i| (i % 256) as u8).collect();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |c| {
        c.protocol.allow_write = true;
    })
    .await;

    client.put("upload.bin", &upload_data).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let written = std::fs::read(dir.path().join("upload.bin")).unwrap();
    assert_eq!(written, upload_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_wrq_with_blksize_oack() {
    let dir = tempfile::tempdir().unwrap();
    let upload_data: Vec<u8> = (0..3072).map(|i| (i % 256) as u8).collect();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |c| {
        c.protocol.allow_write = true;
    })
    .await;

    let opts = TftpOptions {
        blksize: Some(1024),
        ..Default::default()
    };
    client
        .put_with_options("upload_blk.bin", &upload_data, &opts)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let written = std::fs::read(dir.path().join("upload_blk.bin")).unwrap();
    assert_eq!(written, upload_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn test_wrq_denied() {
    let dir = tempfile::tempdir().unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |c| {
        c.protocol.allow_write = false;
    })
    .await;

    let result = client.put("denied.bin", b"test data").await;
    assert!(result.is_err());

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Duplicate ACK / Sorcerer's Apprentice test
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rrq_duplicate_ack_no_retransmit() {
    let dir = tempfile::tempdir().unwrap();
    let test_data: Vec<u8> = (0..1536).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.path().join("dup.bin"), &test_data).unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |_| {}).await;

    // Low-level: send RRQ, then manually handle ACKs with duplicates
    client.send_rrq("dup.bin", &[]).await.unwrap();

    let mut received = Vec::new();
    let mut expected_block = 1u16;

    loop {
        let pkt = client.recv_packet(Duration::from_secs(5)).await.unwrap();
        match pkt.packet {
            Packet::Data { block, data } => {
                assert_eq!(block, expected_block);
                let is_last = data.len() < 512;
                received.extend_from_slice(&data);

                // Send ACK
                client.send_ack_to(block, pkt.from).await.unwrap();
                // Send duplicate ACK (Sorcerer's Apprentice scenario)
                client.send_ack_to(block, pkt.from).await.unwrap();

                if !is_last {
                    // Check no extra duplicate DATA arrives
                    let dup_check = client
                        .try_recv_packet(Duration::from_millis(200))
                        .await
                        .unwrap();
                    if let Some(extra) = dup_check {
                        if let Packet::Data {
                            block: b2,
                            data: d2,
                        } = extra.packet
                        {
                            // Must be next block, not a duplicate
                            assert_eq!(
                                b2,
                                expected_block + 1,
                                "got duplicate DATA({}), expected next block {}",
                                b2,
                                expected_block + 1
                            );
                            expected_block += 1;
                            let is_last2 = d2.len() < 512;
                            received.extend_from_slice(&d2);
                            client.send_ack_to(b2, extra.from).await.unwrap();
                            if is_last2 {
                                break;
                            }
                        }
                    }
                }

                if is_last {
                    break;
                }
                expected_block += 1;
            }
            Packet::Error { code, message } => panic!("error {:?}: {}", code, message),
            _ => {}
        }
    }

    assert_eq!(received, test_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 4 new tests: retransmit, config reload, block rollover
// ═══════════════════════════════════════════════════════════════════════════════

/// Test that the server retransmits DATA after timeout when no ACK is received.
#[tokio::test]
async fn test_retransmit_on_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let test_data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.path().join("retry.bin"), &test_data).unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |c| {
        c.protocol.default_timeout = 1;
        c.protocol.min_timeout = 1;
    })
    .await;

    // Send RRQ with timeout=1
    let opts = TftpOptions {
        timeout: Some(1),
        ..Default::default()
    };
    client
        .send_rrq("retry.bin", &opts.to_tftp_options())
        .await
        .unwrap();

    // Expect OACK (because we sent options)
    let oack = client.recv_packet(Duration::from_secs(3)).await.unwrap();
    assert!(
        matches!(&oack.packet, Packet::Oack { .. }),
        "expected OACK, got {:?}",
        oack.packet
    );
    client.send_ack_to(0, oack.from).await.unwrap();

    // Receive DATA(1) — do NOT send ACK
    let data1_first = client.recv_packet(Duration::from_secs(3)).await.unwrap();
    assert!(
        matches!(&data1_first.packet, Packet::Data { block: 1, .. }),
        "expected DATA(1), got {:?}",
        data1_first.packet
    );

    // Wait for retransmission (timeout is 1 sec, wait up to 3 sec)
    let data1_retry = client.recv_packet(Duration::from_secs(3)).await.unwrap();
    assert!(
        matches!(&data1_retry.packet, Packet::Data { block: 1, .. }),
        "expected retransmitted DATA(1), got {:?}",
        data1_retry.packet
    );

    // Now ACK block 1 and receive the rest normally
    client.send_ack_to(1, data1_retry.from).await.unwrap();

    // Continue receiving remaining blocks
    let mut received = Vec::new();
    if let Packet::Data { data, .. } = data1_first.packet {
        received.extend_from_slice(&data);
    }

    let mut expected = 2u16;
    loop {
        let pkt = client.recv_packet(Duration::from_secs(5)).await.unwrap();
        match pkt.packet {
            Packet::Data { block, data } => {
                assert_eq!(block, expected);
                let is_last = data.len() < 512;
                received.extend_from_slice(&data);
                client.send_ack_to(block, pkt.from).await.unwrap();
                if is_last {
                    break;
                }
                expected = expected.wrapping_add(1);
            }
            Packet::Error { code, message } => panic!("error {:?}: {}", code, message),
            _ => {}
        }
    }

    assert_eq!(received, test_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

/// Test block number rollover for files > 65535 blocks.
/// This is a heavy test (~32 MB), marked #[ignore] for normal CI.
#[tokio::test]
#[ignore]
async fn test_block_number_rollover() {
    let dir = tempfile::tempdir().unwrap();
    // 65536 blocks * 512 + 512 = 33,554,944 bytes → 65537 blocks
    // block 65535 has block# 65535 (0xFFFF), block 65536 has block# 0 (rollover)
    let file_size = 65536 * 512 + 512;
    let test_data: Vec<u8> = (0..file_size).map(|i| ((i * 7 + 13) % 256) as u8).collect();
    std::fs::write(dir.path().join("rollover.bin"), &test_data).unwrap();

    let (client, _addr, state, handle) = mini_server(canonical_temp_path(&dir), |c| {
        c.session.max_retries = 3;
    })
    .await;

    let received = client.get("rollover.bin").await.unwrap();
    assert_eq!(received.len(), test_data.len());
    assert_eq!(received, test_data);

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

/// Test config hot-reload: changing max_blksize in config file affects new sessions.
#[tokio::test]
async fn test_config_hot_reload() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical_temp_path(&dir);

    // Create a test file
    let test_data: Vec<u8> = (0..2048).map(|i| (i % 256) as u8).collect();
    std::fs::write(root.join("reload.bin"), &test_data).unwrap();

    // Create initial config file with max_blksize = 1024
    let config_path = root.join("test_config.toml");
    std::fs::write(
        &config_path,
        "[server]\nport = 0\nlog_level = \"warn\"\nlog_file = \"\"\n\n[protocol]\nmax_blksize = 1024\n",
    )
    .unwrap();

    // Start a multi-request server
    let (server_addr, state, handle) = mini_server_multi(root.clone(), 3, |c| {
        c.protocol.max_blksize = 1024;
    })
    .await;

    // First request: blksize=2048 should be clamped to 1024
    let client1 = TftpTestClient::new(server_addr).await;
    client1
        .send_rrq(
            "reload.bin",
            &[TftpOption {
                name: "blksize".to_string(),
                value: "2048".to_string(),
            }],
        )
        .await
        .unwrap();

    let oack1 = client1.recv_packet(Duration::from_secs(5)).await.unwrap();
    let mut negotiated_blksize_1 = 512u16;
    if let Packet::Oack { options } = &oack1.packet {
        let neg = NegotiatedOptions::from_oack(options);
        if let Some(bs) = neg.blksize {
            negotiated_blksize_1 = bs;
        }
    }
    assert_eq!(
        negotiated_blksize_1, 1024,
        "first request should negotiate blksize=1024"
    );

    // Finish the transfer to free the server for next request
    client1.send_ack_to(0, oack1.from).await.unwrap();
    let _data = recv_data_until_done(&client1, 1024).await;

    // Now update config: bump max_blksize to 4096
    // Directly update the ArcSwap config (simulating hot-reload)
    {
        let mut new_config = (*state.config()).clone();
        new_config.protocol.max_blksize = 4096;
        state.config.store(Arc::new(new_config));
    }

    // Small delay for config propagation
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Second request: blksize=2048 should now be accepted (max is 4096)
    let client2 = TftpTestClient::new(server_addr).await;
    client2
        .send_rrq(
            "reload.bin",
            &[TftpOption {
                name: "blksize".to_string(),
                value: "2048".to_string(),
            }],
        )
        .await
        .unwrap();

    let oack2 = client2.recv_packet(Duration::from_secs(5)).await.unwrap();
    let mut negotiated_blksize_2 = 512u16;
    if let Packet::Oack { options } = &oack2.packet {
        let neg = NegotiatedOptions::from_oack(options);
        if let Some(bs) = neg.blksize {
            negotiated_blksize_2 = bs;
        }
    }
    assert_eq!(
        negotiated_blksize_2, 2048,
        "after config reload, blksize=2048 should be accepted"
    );

    // Finish transfer
    client2.send_ack_to(0, oack2.from).await.unwrap();
    let _data = recv_data_until_done(&client2, 2048).await;

    state.cancel_shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

// ─── Helper ────────────────────────────────────────────────────────────────

/// Receive DATA blocks until transfer is done, returning collected bytes.
async fn recv_data_until_done(client: &TftpTestClient, blksize: usize) -> Vec<u8> {
    let mut received = Vec::new();
    let mut expected = 1u16;
    loop {
        let pkt = client.recv_packet(Duration::from_secs(5)).await.unwrap();
        match pkt.packet {
            Packet::Data { block, data } => {
                assert_eq!(block, expected);
                let is_last = data.len() < blksize;
                received.extend_from_slice(&data);
                client.send_ack_to(block, pkt.from).await.unwrap();
                if is_last {
                    break;
                }
                expected = expected.wrapping_add(1);
            }
            Packet::Error { code, message } => panic!("error {:?}: {}", code, message),
            _ => {}
        }
    }
    received
}
