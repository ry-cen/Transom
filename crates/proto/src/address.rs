//! The strict parser for every address taken from a person or a link.
//! Port of AddressParser.swift:8-175.

use std::net::Ipv6Addr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressKind {
    Ipv4,
    Ipv6,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedAddress {
    /// No brackets; IPv6 compressed lowercase; names as typed.
    pub host: String,
    pub port: Option<u16>,
    pub kind: AddressKind,
}

impl ParsedAddress {
    /// "[v6]:port", "host:port" or "host" (AddressParser.swift:23-26).
    pub fn text(&self) -> String {
        let h = if self.kind == AddressKind::Ipv6 {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match self.port {
            Some(p) => format!("{h}:{p}"),
            None => h,
        }
    }

    fn with_port(mut self, p: u16) -> Self {
        self.port = Some(p);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("the address is empty")]
    Empty,
    #[error("the address has whitespace or a character no address has")]
    Malformed,
    #[error("an IPv6 zone only means something on the network it was copied from")]
    Zone,
    #[error("not a plain IPv4 address of four decimal octets")]
    Ipv4,
    #[error("not an IPv6 address")]
    Ipv6,
    #[error("not a valid host name")]
    Name,
    #[error("the port must be 1 to 65535")]
    Port,
    #[error("this is a pairing link, not an address")]
    PairingLink,
}

/// Port of AddressParser.swift:58-108. The ends are trimmed; other whitespace refuses it.
pub fn parse_address(raw: &str) -> Result<ParsedAddress, AddressError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(AddressError::Empty);
    }
    if s.to_lowercase().starts_with("sill:") {
        return Err(AddressError::PairingLink);
    }
    if s.chars()
        .any(|c| c.is_whitespace() || c < '\u{20}' || c == '\u{7f}')
    {
        return Err(AddressError::Malformed);
    }
    if s.contains('%') {
        return Err(AddressError::Zone);
    }

    if let Some(inner) = s.strip_prefix('[') {
        let close = inner.find(']').ok_or(AddressError::Ipv6)?;
        let host_part = &inner[..close];
        let rest = &inner[close + 1..];
        let mut port_part = None;
        if !rest.is_empty() {
            port_part = Some(rest.strip_prefix(':').ok_or(AddressError::Malformed)?);
        }
        let v6 = ipv6(host_part).ok_or(AddressError::Ipv6)?;
        return match port_part {
            Some(p) => Ok(v6.with_port(parse_port(p).ok_or(AddressError::Port)?)),
            None => Ok(v6),
        };
    }
    let colons = s.matches(':').count();
    if colons >= 2 {
        // A bare IPv6 address, never an address plus a port.
        return ipv6(s).ok_or(AddressError::Ipv6);
    }
    let (host_part, port_part) = match s.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (s, None),
    };
    let host = host_only(host_part)?;
    match port_part {
        Some(p) => Ok(host.with_port(parse_port(p).ok_or(AddressError::Port)?)),
        None => Ok(host),
    }
}

/// An IPv4 address or a name, no port (AddressParser.swift:111-128).
fn host_only(h: &str) -> Result<ParsedAddress, AddressError> {
    if h.is_empty() {
        return Err(AddressError::Malformed);
    }
    let trimmed = h.strip_suffix('.').unwrap_or(h);
    let labels: Vec<&str> = trimmed.split('.').collect();
    let all_numeric = labels
        .iter()
        .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()));
    if all_numeric {
        let ok = !h.ends_with('.')
            && labels.len() == 4
            && labels.iter().all(|l| {
                (*l == "0" || !l.starts_with('0'))
                    && l.len() <= 3
                    && l.parse::<u32>().is_ok_and(|v| v <= 255)
            });
        if !ok {
            return Err(AddressError::Ipv4);
        }
        return Ok(ParsedAddress {
            host: h.to_string(),
            port: None,
            kind: AddressKind::Ipv4,
        });
    }
    if inet_aton_accepts(h) {
        return Err(AddressError::Ipv4);
    }
    if !is_name(h) {
        return Err(AddressError::Name);
    }
    Ok(ParsedAddress {
        host: h.to_string(),
        port: None,
        kind: AddressKind::Name,
    })
}

/// RFC 1123 host name, ASCII only (AddressParser.swift:133-141).
fn is_name(h: &str) -> bool {
    let body = h.strip_suffix('.').unwrap_or(h);
    if body.is_empty() || body.chars().count() > 253 {
        return false;
    }
    body.split('.').all(|label| {
        (1..=63).contains(&label.chars().count())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// A bare IPv6 address (AddressParser.swift:145-156): compressed lowercase, or IPv4 for an
/// IPv4-mapped one.
fn ipv6(h: &str) -> Option<ParsedAddress> {
    if h.is_empty() || h.contains('%') {
        return None;
    }
    let addr: Ipv6Addr = h.parse().ok()?;
    Some(match addr.to_ipv4_mapped() {
        Some(v4) => ParsedAddress {
            host: v4.to_string(),
            port: None,
            kind: AddressKind::Ipv4,
        },
        None => ParsedAddress {
            host: addr.to_string(),
            port: None,
            kind: AddressKind::Ipv6,
        },
    })
}

/// Emulates `inet_aton` (AddressParser.swift:158-161): what the system would read as IPv4.
fn inet_aton_accepts(h: &str) -> bool {
    fn number(p: &str) -> Option<u64> {
        if p.is_empty() {
            return None;
        }
        let (digits, radix) =
            if let Some(hex) = p.strip_prefix("0x").or_else(|| p.strip_prefix("0X")) {
                (hex, 16)
            } else if p.len() > 1 && p.starts_with('0') {
                (&p[1..], 8)
            } else {
                (p, 10)
            };
        if digits.is_empty() {
            return Some(0);
        }
        // Digits only: from_str_radix would also accept a sign.
        if !digits.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        u64::from_str_radix(digits, radix).ok()
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return false;
    }
    let mut v = Vec::with_capacity(parts.len());
    for p in parts {
        match number(p) {
            Some(n) => v.push(n),
            None => return false,
        }
    }
    let last = v.len() - 1;
    let limit_last = match v.len() {
        1 => 0xFFFF_FFFF,
        2 => 0xFF_FFFF,
        3 => 0xFFFF,
        _ => 0xFF,
    };
    v[..last].iter().all(|&n| n <= 255) && v[last] <= limit_last
}

/// Port of AddressParser.swift:163-166.
fn parse_port(p: &str) -> Option<u16> {
    if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: u32 = p.parse().ok()?;
    if (1..=65535).contains(&v) {
        u16::try_from(v).ok()
    } else {
        None
    }
}
