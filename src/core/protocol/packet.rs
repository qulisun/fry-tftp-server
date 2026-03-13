use bytes::{BufMut, BytesMut};
use std::fmt;

/// TFTP opcodes per RFC 1350 + RFC 2347
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Opcode {
    Rrq = 1,
    Wrq = 2,
    Data = 3,
    Ack = 4,
    Error = 5,
    Oack = 6,
}

impl Opcode {
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            1 => Some(Self::Rrq),
            2 => Some(Self::Wrq),
            3 => Some(Self::Data),
            4 => Some(Self::Ack),
            5 => Some(Self::Error),
            6 => Some(Self::Oack),
            _ => None,
        }
    }
}

/// TFTP error codes per RFC 1350
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    NotDefined = 0,
    FileNotFound = 1,
    AccessViolation = 2,
    DiskFull = 3,
    IllegalOperation = 4,
    UnknownTransferId = 5,
    FileAlreadyExists = 6,
    NoSuchUser = 7,
    OptionRejected = 8,
}

impl ErrorCode {
    pub fn from_u16(val: u16) -> Self {
        match val {
            0 => Self::NotDefined,
            1 => Self::FileNotFound,
            2 => Self::AccessViolation,
            3 => Self::DiskFull,
            4 => Self::IllegalOperation,
            5 => Self::UnknownTransferId,
            6 => Self::FileAlreadyExists,
            7 => Self::NoSuchUser,
            8 => Self::OptionRejected,
            _ => Self::NotDefined,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDefined => write!(f, "Not defined"),
            Self::FileNotFound => write!(f, "File not found"),
            Self::AccessViolation => write!(f, "Access violation"),
            Self::DiskFull => write!(f, "Disk full"),
            Self::IllegalOperation => write!(f, "Illegal TFTP operation"),
            Self::UnknownTransferId => write!(f, "Unknown transfer ID"),
            Self::FileAlreadyExists => write!(f, "File already exists"),
            Self::NoSuchUser => write!(f, "No such user"),
            Self::OptionRejected => write!(f, "Option rejected"),
        }
    }
}

/// Transfer mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    Octet,
    Netascii,
}

impl TransferMode {
    pub fn from_str_ignore_case(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "octet" => Some(Self::Octet),
            "netascii" => Some(Self::Netascii),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Octet => "octet",
            Self::Netascii => "netascii",
        }
    }
}

/// A negotiation option from RRQ/WRQ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TftpOption {
    pub name: String,
    pub value: String,
}

/// Parsed TFTP packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Rrq {
        filename: String,
        mode: TransferMode,
        options: Vec<TftpOption>,
    },
    Wrq {
        filename: String,
        mode: TransferMode,
        options: Vec<TftpOption>,
    },
    Data {
        block: u16,
        data: Vec<u8>,
    },
    Ack {
        block: u16,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    Oack {
        options: Vec<TftpOption>,
    },
}

/// Parse error
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("packet too short: {0} bytes")]
    TooShort(usize),
    #[error("unknown opcode: {0}")]
    UnknownOpcode(u16),
    #[error("malformed packet: {0}")]
    Malformed(String),
    #[error("invalid filename encoding")]
    InvalidFilename,
    #[error("unknown transfer mode: {0}")]
    UnknownMode(String),
}

/// Parse a TFTP packet from raw bytes (zero-copy where practical)
pub fn parse_packet(data: &[u8]) -> Result<Packet, ParseError> {
    if data.len() < 2 {
        return Err(ParseError::TooShort(data.len()));
    }

    let opcode = u16::from_be_bytes([data[0], data[1]]);
    let opcode = Opcode::from_u16(opcode).ok_or(ParseError::UnknownOpcode(opcode))?;

    match opcode {
        Opcode::Rrq => parse_request(data, true),
        Opcode::Wrq => parse_request(data, false),
        Opcode::Data => parse_data(data),
        Opcode::Ack => parse_ack(data),
        Opcode::Error => parse_error(data),
        Opcode::Oack => parse_oack(data),
    }
}

fn parse_null_terminated_strings(data: &[u8]) -> Vec<&[u8]> {
    let mut strings = Vec::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == 0 {
            strings.push(&data[start..i]);
            start = i + 1;
        }
    }
    strings
}

fn parse_request(data: &[u8], is_rrq: bool) -> Result<Packet, ParseError> {
    let payload = &data[2..];
    let strings = parse_null_terminated_strings(payload);

    if strings.len() < 2 {
        return Err(ParseError::Malformed(
            "missing filename or mode".to_string(),
        ));
    }

    let filename = std::str::from_utf8(strings[0]).map_err(|_| ParseError::InvalidFilename)?;

    // Validate filename: no null bytes, no control chars, no ..
    validate_filename(filename)?;

    let mode_str = std::str::from_utf8(strings[1])
        .map_err(|_| ParseError::Malformed("invalid mode encoding".to_string()))?;
    let mode = TransferMode::from_str_ignore_case(mode_str)
        .ok_or_else(|| ParseError::UnknownMode(mode_str.to_string()))?;

    // Parse options (pairs after mode)
    let mut options = Vec::new();
    let mut i = 2;
    while i + 1 < strings.len() {
        let name = std::str::from_utf8(strings[i])
            .map_err(|_| ParseError::Malformed("invalid option name encoding".to_string()))?;
        let value = std::str::from_utf8(strings[i + 1])
            .map_err(|_| ParseError::Malformed("invalid option value encoding".to_string()))?;
        options.push(TftpOption {
            name: name.to_ascii_lowercase(),
            value: value.to_string(),
        });
        i += 2;
    }

    let filename = filename.to_string();
    if is_rrq {
        Ok(Packet::Rrq {
            filename,
            mode,
            options,
        })
    } else {
        Ok(Packet::Wrq {
            filename,
            mode,
            options,
        })
    }
}

fn validate_filename(filename: &str) -> Result<(), ParseError> {
    if filename.is_empty() {
        return Err(ParseError::Malformed("empty filename".to_string()));
    }
    if filename.contains("..") {
        return Err(ParseError::Malformed(
            "path traversal attempt (..)".to_string(),
        ));
    }
    if filename.contains('~') {
        return Err(ParseError::Malformed("tilde in filename".to_string()));
    }
    for b in filename.bytes() {
        if b < 0x20 {
            return Err(ParseError::Malformed(format!(
                "control character 0x{:02x} in filename",
                b
            )));
        }
    }
    Ok(())
}

fn parse_data(data: &[u8]) -> Result<Packet, ParseError> {
    if data.len() < 4 {
        return Err(ParseError::TooShort(data.len()));
    }
    let block = u16::from_be_bytes([data[2], data[3]]);
    let payload = data[4..].to_vec();
    Ok(Packet::Data {
        block,
        data: payload,
    })
}

fn parse_ack(data: &[u8]) -> Result<Packet, ParseError> {
    if data.len() < 4 {
        return Err(ParseError::TooShort(data.len()));
    }
    let block = u16::from_be_bytes([data[2], data[3]]);
    Ok(Packet::Ack { block })
}

fn parse_error(data: &[u8]) -> Result<Packet, ParseError> {
    if data.len() < 4 {
        return Err(ParseError::TooShort(data.len()));
    }
    let code = u16::from_be_bytes([data[2], data[3]]);
    let message = if data.len() > 4 {
        let msg_bytes = &data[4..];
        // Find null terminator
        let end = msg_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(msg_bytes.len());
        std::str::from_utf8(&msg_bytes[..end])
            .unwrap_or("invalid error message")
            .to_string()
    } else {
        String::new()
    };
    Ok(Packet::Error {
        code: ErrorCode::from_u16(code),
        message,
    })
}

fn parse_oack(data: &[u8]) -> Result<Packet, ParseError> {
    let payload = &data[2..];
    let strings = parse_null_terminated_strings(payload);

    let mut options = Vec::new();
    let mut i = 0;
    while i + 1 < strings.len() {
        let name = std::str::from_utf8(strings[i])
            .map_err(|_| ParseError::Malformed("invalid option name in OACK".to_string()))?;
        let value = std::str::from_utf8(strings[i + 1])
            .map_err(|_| ParseError::Malformed("invalid option value in OACK".to_string()))?;
        options.push(TftpOption {
            name: name.to_ascii_lowercase(),
            value: value.to_string(),
        });
        i += 2;
    }

    Ok(Packet::Oack { options })
}

/// Serialize a packet into bytes
pub fn serialize_packet(packet: &Packet) -> BytesMut {
    match packet {
        Packet::Rrq {
            filename,
            mode,
            options,
        } => serialize_request(Opcode::Rrq as u16, filename, mode, options),
        Packet::Wrq {
            filename,
            mode,
            options,
        } => serialize_request(Opcode::Wrq as u16, filename, mode, options),
        Packet::Data { block, data } => {
            let mut buf = BytesMut::with_capacity(4 + data.len());
            buf.put_u16(Opcode::Data as u16);
            buf.put_u16(*block);
            buf.put_slice(data);
            buf
        }
        Packet::Ack { block } => {
            let mut buf = BytesMut::with_capacity(4);
            buf.put_u16(Opcode::Ack as u16);
            buf.put_u16(*block);
            buf
        }
        Packet::Error { code, message } => {
            let mut buf = BytesMut::with_capacity(5 + message.len());
            buf.put_u16(Opcode::Error as u16);
            buf.put_u16(*code as u16);
            buf.put_slice(message.as_bytes());
            buf.put_u8(0);
            buf
        }
        Packet::Oack { options } => {
            let mut buf = BytesMut::with_capacity(2 + options.len() * 20);
            buf.put_u16(Opcode::Oack as u16);
            for opt in options {
                buf.put_slice(opt.name.as_bytes());
                buf.put_u8(0);
                buf.put_slice(opt.value.as_bytes());
                buf.put_u8(0);
            }
            buf
        }
    }
}

fn serialize_request(
    opcode: u16,
    filename: &str,
    mode: &TransferMode,
    options: &[TftpOption],
) -> BytesMut {
    let mut buf =
        BytesMut::with_capacity(4 + filename.len() + mode.as_str().len() + options.len() * 20);
    buf.put_u16(opcode);
    buf.put_slice(filename.as_bytes());
    buf.put_u8(0);
    buf.put_slice(mode.as_str().as_bytes());
    buf.put_u8(0);
    for opt in options {
        buf.put_slice(opt.name.as_bytes());
        buf.put_u8(0);
        buf.put_slice(opt.value.as_bytes());
        buf.put_u8(0);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rrq_basic() {
        // opcode=1 | "test.bin" | 0 | "octet" | 0
        let mut data = vec![0x00, 0x01];
        data.extend_from_slice(b"test.bin");
        data.push(0);
        data.extend_from_slice(b"octet");
        data.push(0);

        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Rrq {
                filename,
                mode,
                options,
            } => {
                assert_eq!(filename, "test.bin");
                assert_eq!(mode, TransferMode::Octet);
                assert!(options.is_empty());
            }
            _ => panic!("expected RRQ"),
        }
    }

    #[test]
    fn test_parse_rrq_with_options() {
        let mut data = vec![0x00, 0x01];
        data.extend_from_slice(b"firmware.img");
        data.push(0);
        data.extend_from_slice(b"octet");
        data.push(0);
        data.extend_from_slice(b"blksize");
        data.push(0);
        data.extend_from_slice(b"1468");
        data.push(0);
        data.extend_from_slice(b"tsize");
        data.push(0);
        data.extend_from_slice(b"0");
        data.push(0);

        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Rrq { options, .. } => {
                assert_eq!(options.len(), 2);
                assert_eq!(options[0].name, "blksize");
                assert_eq!(options[0].value, "1468");
                assert_eq!(options[1].name, "tsize");
                assert_eq!(options[1].value, "0");
            }
            _ => panic!("expected RRQ"),
        }
    }

    #[test]
    fn test_parse_wrq() {
        let mut data = vec![0x00, 0x02];
        data.extend_from_slice(b"upload.bin");
        data.push(0);
        data.extend_from_slice(b"OCTET");
        data.push(0);

        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Wrq { filename, mode, .. } => {
                assert_eq!(filename, "upload.bin");
                assert_eq!(mode, TransferMode::Octet);
            }
            _ => panic!("expected WRQ"),
        }
    }

    #[test]
    fn test_parse_data() {
        let data = vec![0x00, 0x03, 0x00, 0x01, 0xDE, 0xAD, 0xBE, 0xEF];
        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Data { block, data } => {
                assert_eq!(block, 1);
                assert_eq!(data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
            }
            _ => panic!("expected DATA"),
        }
    }

    #[test]
    fn test_parse_ack() {
        let data = vec![0x00, 0x04, 0x00, 0x2A];
        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Ack { block } => assert_eq!(block, 42),
            _ => panic!("expected ACK"),
        }
    }

    #[test]
    fn test_parse_error() {
        let mut data = vec![0x00, 0x05, 0x00, 0x01];
        data.extend_from_slice(b"File not found");
        data.push(0);

        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Error { code, message } => {
                assert_eq!(code, ErrorCode::FileNotFound);
                assert_eq!(message, "File not found");
            }
            _ => panic!("expected ERROR"),
        }
    }

    #[test]
    fn test_parse_oack() {
        let mut data = vec![0x00, 0x06];
        data.extend_from_slice(b"blksize");
        data.push(0);
        data.extend_from_slice(b"1468");
        data.push(0);
        data.extend_from_slice(b"windowsize");
        data.push(0);
        data.extend_from_slice(b"8");
        data.push(0);

        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Oack { options } => {
                assert_eq!(options.len(), 2);
                assert_eq!(options[0].name, "blksize");
                assert_eq!(options[0].value, "1468");
                assert_eq!(options[1].name, "windowsize");
                assert_eq!(options[1].value, "8");
            }
            _ => panic!("expected OACK"),
        }
    }

    #[test]
    fn test_roundtrip_data() {
        let original = Packet::Data {
            block: 42,
            data: vec![1, 2, 3, 4, 5],
        };
        let bytes = serialize_packet(&original);
        let parsed = parse_packet(&bytes).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_roundtrip_ack() {
        let original = Packet::Ack { block: 100 };
        let bytes = serialize_packet(&original);
        let parsed = parse_packet(&bytes).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_roundtrip_error() {
        let original = Packet::Error {
            code: ErrorCode::AccessViolation,
            message: "Access denied".to_string(),
        };
        let bytes = serialize_packet(&original);
        let parsed = parse_packet(&bytes).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_path_traversal_rejected() {
        let mut data = vec![0x00, 0x01];
        data.extend_from_slice(b"../../etc/passwd");
        data.push(0);
        data.extend_from_slice(b"octet");
        data.push(0);

        assert!(parse_packet(&data).is_err());
    }

    #[test]
    fn test_tilde_rejected() {
        let mut data = vec![0x00, 0x01];
        data.extend_from_slice(b"~root/.ssh/keys");
        data.push(0);
        data.extend_from_slice(b"octet");
        data.push(0);

        assert!(parse_packet(&data).is_err());
    }

    #[test]
    fn test_control_chars_rejected() {
        let mut data = vec![0x00, 0x01];
        data.extend_from_slice(b"file\x01name");
        data.push(0);
        data.extend_from_slice(b"octet");
        data.push(0);

        assert!(parse_packet(&data).is_err());
    }

    #[test]
    fn test_too_short() {
        assert!(parse_packet(&[0x00]).is_err());
        assert!(parse_packet(&[]).is_err());
    }

    #[test]
    fn test_unknown_opcode() {
        assert!(parse_packet(&[0x00, 0xFF]).is_err());
    }

    #[test]
    fn test_zero_byte_data() {
        let data = vec![0x00, 0x03, 0x00, 0x01]; // DATA block 1, empty payload
        let pkt = parse_packet(&data).unwrap();
        match pkt {
            Packet::Data { block, data } => {
                assert_eq!(block, 1);
                assert!(data.is_empty());
            }
            _ => panic!("expected DATA"),
        }
    }

    // ─── Property-based tests (proptest) ─────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn roundtrip_data(block in 1u16..=65535, payload in proptest::collection::vec(any::<u8>(), 0..512)) {
                let pkt = Packet::Data { block, data: payload.clone() };
                let bytes = serialize_packet(&pkt);
                let parsed = parse_packet(&bytes).unwrap();
                assert_eq!(pkt, parsed);
            }

            #[test]
            fn roundtrip_ack(block in 0u16..=65535) {
                let pkt = Packet::Ack { block };
                let bytes = serialize_packet(&pkt);
                let parsed = parse_packet(&bytes).unwrap();
                assert_eq!(pkt, parsed);
            }

            #[test]
            fn roundtrip_error(code_val in 0u16..8, msg in "[a-zA-Z0-9 ]{0,50}") {
                let code = match code_val {
                    0 => ErrorCode::NotDefined,
                    1 => ErrorCode::FileNotFound,
                    2 => ErrorCode::AccessViolation,
                    3 => ErrorCode::DiskFull,
                    4 => ErrorCode::IllegalOperation,
                    5 => ErrorCode::UnknownTransferId,
                    6 => ErrorCode::FileAlreadyExists,
                    _ => ErrorCode::NoSuchUser,
                };
                let pkt = Packet::Error { code, message: msg };
                let bytes = serialize_packet(&pkt);
                let parsed = parse_packet(&bytes).unwrap();
                assert_eq!(pkt, parsed);
            }

            #[test]
            fn parse_never_panics(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
                // Should either succeed or return an error, never panic
                let _ = parse_packet(&data);
            }
        }
    }
}
