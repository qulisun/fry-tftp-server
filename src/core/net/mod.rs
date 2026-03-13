use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::core::config::Config;

/// IP version mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpVersion {
    Dual,
    V4,
    V6,
}

impl IpVersion {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "v4" => Self::V4,
            "v6" => Self::V6,
            _ => Self::Dual,
        }
    }
}

/// Create the main listening UDP socket with proper dual-stack configuration
pub fn create_main_socket(config: &Config) -> Result<tokio::net::UdpSocket> {
    let ip_version = IpVersion::from_str(&config.network.ip_version);

    // Parse bind_address; fall back to UNSPECIFIED if invalid
    let parsed_ip: Option<IpAddr> = config.server.bind_address.parse().ok();

    let (socket, bind_addr) = match ip_version {
        IpVersion::Dual => {
            let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
            socket.set_only_v6(false)?; // dual-stack
            let ip = match parsed_ip {
                Some(IpAddr::V6(v6)) => v6,
                Some(IpAddr::V4(v4)) => v4.to_ipv6_mapped(),
                None => Ipv6Addr::UNSPECIFIED,
            };
            (socket, SocketAddr::from((ip, config.server.port)))
        }
        IpVersion::V4 => {
            let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            let ip = match parsed_ip {
                Some(IpAddr::V4(v4)) => v4,
                _ => Ipv4Addr::UNSPECIFIED,
            };
            (socket, SocketAddr::from((ip, config.server.port)))
        }
        IpVersion::V6 => {
            let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
            socket.set_only_v6(true)?; // IPv6 only
            let ip = match parsed_ip {
                Some(IpAddr::V6(v6)) => v6,
                _ => Ipv6Addr::UNSPECIFIED,
            };
            (socket, SocketAddr::from((ip, config.server.port)))
        }
    };

    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;

    // Set buffer sizes (ignore errors — OS may not support requested sizes)
    let _ = socket.set_recv_buffer_size(config.network.recv_buffer_size);
    let _ = socket.set_send_buffer_size(config.network.send_buffer_size);

    socket.bind(&socket2::SockAddr::from(bind_addr))?;

    let std_socket: std::net::UdpSocket = socket.into();
    let tokio_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    Ok(tokio_socket)
}

/// Create a per-session UDP socket on an ephemeral port
pub fn create_session_socket(
    config: &Config,
    ip_version: IpVersion,
) -> Result<tokio::net::UdpSocket> {
    let (socket, bind_addr) = match ip_version {
        IpVersion::V4 => {
            let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0u16));
            (socket, addr)
        }
        _ => {
            let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
            if ip_version == IpVersion::Dual {
                socket.set_only_v6(false)?;
            } else {
                socket.set_only_v6(true)?;
            }
            let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0u16));
            (socket, addr)
        }
    };

    socket.set_nonblocking(true)?;
    let _ = socket.set_recv_buffer_size(config.network.session_recv_buffer);
    let _ = socket.set_send_buffer_size(config.network.session_send_buffer);
    socket.bind(&socket2::SockAddr::from(bind_addr))?;

    let std_socket: std::net::UdpSocket = socket.into();
    let tokio_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    Ok(tokio_socket)
}
