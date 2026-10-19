//! Resolving and dialing a Mac's addresses.

use crate::NetError;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;

pub const HOME_DIAL_TIMEOUT: Duration = Duration::from_secs(3);
pub const REMOTE_DIAL_TIMEOUT: Duration = Duration::from_secs(10);
pub const REMOTE_DEFAULT_PORT: u16 = 7455;

fn is_link_local_v6(a: &Ipv6Addr) -> bool {
    (a.segments()[0] & 0xffc0) == 0xfe80
}

/// IPv4 non-loopback, IPv4 loopback, IPv6 global, IPv6 link-local (kept only with a scope id),
/// then ::1. Duplicates removed, order otherwise kept.
pub fn order_addresses(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut seen = HashSet::new();
    let mut buckets: [Vec<SocketAddr>; 5] = Default::default();
    for a in addrs {
        if !seen.insert(a) {
            continue;
        }
        let slot = match a {
            SocketAddr::V4(v4) if !v4.ip().is_loopback() => 0,
            SocketAddr::V4(_) => 1,
            SocketAddr::V6(v6) if v6.ip().is_loopback() => 4,
            SocketAddr::V6(v6) if is_link_local_v6(v6.ip()) => {
                if v6.scope_id() == 0 {
                    continue;
                }
                3
            }
            SocketAddr::V6(_) => 2,
        };
        buckets[slot].push(a);
    }
    buckets.into_iter().flatten().collect()
}

/// Each address in turn, `per_attempt` each; the first success wins.
pub async fn dial(
    addrs: &[SocketAddr],
    per_attempt: Duration,
) -> Result<(TcpStream, SocketAddr), NetError> {
    let mut last: Option<NetError> = None;
    for addr in addrs {
        match tokio::time::timeout(per_attempt, TcpStream::connect(addr)).await {
            Ok(Ok(s)) => return Ok((s, *addr)),
            Ok(Err(e)) => last = Some(NetError::Connect(format!("{addr}: {e}"))),
            Err(_) => last = Some(NetError::Timeout),
        }
    }
    Err(last.unwrap_or_else(|| NetError::Connect("no address to try".into())))
}

pub async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, NetError> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| NetError::Connect(format!("{host}: {e}")))?
        .collect();
    let ordered = order_addresses(addrs);
    if ordered.is_empty() {
        return Err(NetError::Connect(format!("{host}: no usable address")));
    }
    Ok(ordered)
}

/// Loopback, or an address of this machine (binding a UDP socket to it succeeds).
pub fn is_local_address(addr: &SocketAddr) -> bool {
    let mapped = match addr {
        SocketAddr::V6(v6) => v6
            .ip()
            .to_ipv4_mapped()
            .map(|ip| SocketAddr::new(IpAddr::V4(ip), v6.port())),
        SocketAddr::V4(_) => None,
    };
    let addr = &mapped.unwrap_or(*addr);
    if addr.ip().is_loopback() {
        return true;
    }
    let bind = match addr {
        SocketAddr::V4(v4) => SocketAddr::new(IpAddr::V4(*v4.ip()), 0),
        SocketAddr::V6(v6) => {
            SocketAddr::V6(std::net::SocketAddrV6::new(*v6.ip(), 0, 0, v6.scope_id()))
        }
    };
    std::net::UdpSocket::bind(bind).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV6};

    fn v4(a: u8, b: u8, c: u8, d: u8) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), 1)
    }

    fn v6(s: &str, scope: u32) -> SocketAddr {
        SocketAddr::V6(SocketAddrV6::new(s.parse().unwrap(), 1, 0, scope))
    }

    #[test]
    fn ordering() {
        let input = vec![
            v6("::1", 0),
            v6("fe80::1", 4),
            v6("fd7a::1", 0),
            v4(127, 0, 0, 1),
            v4(192, 168, 1, 5),
            v4(192, 168, 1, 5),
        ];
        let out = order_addresses(input);
        assert_eq!(
            out,
            vec![
                v4(192, 168, 1, 5),
                v4(127, 0, 0, 1),
                v6("fd7a::1", 0),
                v6("fe80::1", 4),
                v6("::1", 0),
            ]
        );
    }

    #[test]
    fn link_local_without_scope_dropped() {
        let out = order_addresses(vec![v6("fe80::1", 0), v4(10, 0, 0, 1)]);
        assert_eq!(out, vec![v4(10, 0, 0, 1)]);
    }

    #[test]
    fn loopback_is_local() {
        assert!(is_local_address(&v4(127, 0, 0, 1)));
    }

    #[test]
    fn mapped_loopback_is_local() {
        assert!(is_local_address(&v6("::ffff:127.0.0.1", 0)));
        assert!(!is_local_address(&v6("::ffff:192.0.2.1", 0)));
    }

    #[test]
    fn documentation_address_is_not_local() {
        assert!(!is_local_address(&v4(192, 0, 2, 1)));
    }

    #[tokio::test]
    async fn dial_refused_is_connect_error() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        let r = dial(&[addr], Duration::from_secs(5)).await;
        assert!(matches!(r, Err(NetError::Connect(_))));
    }
}
