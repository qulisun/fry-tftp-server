use criterion::{black_box, criterion_group, criterion_main, Criterion};

use fry_tftp_server::core::protocol::packet::{parse_packet, serialize_packet, Packet, TftpOption};

fn bench_parse_rrq(c: &mut Criterion) {
    // RRQ: opcode(2) + "firmware.bin\0" + "octet\0" + "blksize\0" + "1468\0"
    let mut pkt = Vec::new();
    pkt.extend_from_slice(&[0, 1]); // RRQ
    pkt.extend_from_slice(b"firmware.bin\0octet\0blksize\01468\0windowsize\064\0");

    c.bench_function("parse_rrq_with_options", |b| {
        b.iter(|| {
            let _ = black_box(parse_packet(black_box(&pkt)));
        })
    });
}

fn bench_parse_data(c: &mut Criterion) {
    let mut pkt = vec![0, 3, 0, 42]; // DATA opcode + block 42
    pkt.extend_from_slice(&[0xAB; 512]); // 512 bytes payload

    c.bench_function("parse_data_512", |b| {
        b.iter(|| {
            let _ = black_box(parse_packet(black_box(&pkt)));
        })
    });
}

fn bench_parse_ack(c: &mut Criterion) {
    let pkt = [0, 4, 0, 100]; // ACK block 100

    c.bench_function("parse_ack", |b| {
        b.iter(|| {
            let _ = black_box(parse_packet(black_box(&pkt)));
        })
    });
}

fn bench_parse_error(c: &mut Criterion) {
    let mut pkt = vec![0, 5, 0, 1]; // ERROR code 1
    pkt.extend_from_slice(b"File not found\0");

    c.bench_function("parse_error", |b| {
        b.iter(|| {
            let _ = black_box(parse_packet(black_box(&pkt)));
        })
    });
}

fn bench_serialize_data(c: &mut Criterion) {
    let packet = Packet::Data {
        block: 42,
        data: vec![0xAB; 1468].into(),
    };

    c.bench_function("serialize_data_1468", |b| {
        b.iter(|| {
            let _ = black_box(serialize_packet(black_box(&packet)));
        })
    });
}

fn bench_serialize_ack(c: &mut Criterion) {
    let packet = Packet::Ack { block: 100 };

    c.bench_function("serialize_ack", |b| {
        b.iter(|| {
            let _ = black_box(serialize_packet(black_box(&packet)));
        })
    });
}

fn bench_serialize_oack(c: &mut Criterion) {
    let packet = Packet::Oack {
        options: vec![
            TftpOption {
                name: "blksize".to_string(),
                value: "1468".to_string(),
            },
            TftpOption {
                name: "windowsize".to_string(),
                value: "64".to_string(),
            },
            TftpOption {
                name: "tsize".to_string(),
                value: "52428800".to_string(),
            },
        ],
    };

    c.bench_function("serialize_oack", |b| {
        b.iter(|| {
            let _ = black_box(serialize_packet(black_box(&packet)));
        })
    });
}

fn bench_parse_malformed(c: &mut Criterion) {
    let pkt = [0xFF, 0xFF, 0, 0, 0]; // Unknown opcode

    c.bench_function("parse_malformed", |b| {
        b.iter(|| {
            let _ = black_box(parse_packet(black_box(&pkt)));
        })
    });
}

criterion_group!(
    benches,
    bench_parse_rrq,
    bench_parse_data,
    bench_parse_ack,
    bench_parse_error,
    bench_serialize_data,
    bench_serialize_ack,
    bench_serialize_oack,
    bench_parse_malformed,
);
criterion_main!(benches);
