//! The pairing link a QR code carries. Port of Pairing.swift:223-307.

use crate::address::{parse_address, ParsedAddress};
use crate::identity::{b64url_decode, b64url_encode, is_well_formed_mac_id, mac_id};
use crate::safetext;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

pub const MAX_ADDRESSES: usize = 5;
pub const NAME_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct PairLink {
    pub mac_id: String,
    pub fingerprint: [u8; 32],
    pub secret: [u8; 16],
    pub name: String,
    pub port: u16,
    pub addresses: Vec<ParsedAddress>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("not a sill://pair link")]
    NotALink,
    #[error("unsupported link version")]
    Version,
    #[error("missing, repeated or malformed field {0}")]
    Field(String),
    #[error("the Mac ID does not match the key")]
    MacIdMismatch,
    #[error("bad address {0}")]
    Address(String),
}

/// Unreserved characters and ":" stay as they are (Pairing.swift:263-266).
const ESCAPE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b':');

fn escape(s: &str) -> String {
    utf8_percent_encode(s, ESCAPE).to_string()
}

/// The query items of a `sill://pair` link, or None when it is not one.
fn query_items(text: &str) -> Option<Vec<(String, Option<String>)>> {
    let t = text.trim();
    let (scheme, rest) = t.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("sill") {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (before, query) = rest.split_once('?')?;
    let (authority, path) = match before.find('/') {
        Some(i) => (&before[..i], &before[i..]),
        None => (before, ""),
    };
    if !authority.eq_ignore_ascii_case("pair") || !(path.is_empty() || path == "/") {
        return None;
    }
    let mut items = Vec::new();
    for item in query.split('&') {
        let (n, v) = match item.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (item, None),
        };
        let name = percent_decode_str(n).decode_utf8().ok()?.into_owned();
        let value = match v {
            Some(v) => Some(percent_decode_str(v).decode_utf8().ok()?.into_owned()),
            None => None,
        };
        items.push((name, value));
    }
    Some(items)
}

impl PairLink {
    /// Port of Pairing.swift:282-306.
    pub fn parse(text: &str) -> Result<PairLink, LinkError> {
        let items = query_items(text).ok_or(LinkError::NotALink)?;
        let single = |key: &str| -> Option<&str> {
            let mut it = items.iter().filter(|(n, _)| n == key);
            let first = it.next()?;
            if it.next().is_some() {
                return None;
            }
            first.1.as_deref().filter(|v| !v.is_empty())
        };
        let field = |key: &str| LinkError::Field(key.to_string());
        if single("v") != Some("1") {
            return Err(LinkError::Version);
        }
        let m = single("m")
            .filter(|m| is_well_formed_mac_id(m))
            .ok_or_else(|| field("m"))?;
        let fingerprint: [u8; 32] = single("k")
            .and_then(b64url_decode)
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| field("k"))?;
        let secret: [u8; 16] = single("s")
            .and_then(b64url_decode)
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| field("s"))?;
        let port = single("p")
            .filter(|p| p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse::<u32>().ok())
            .filter(|p| (1..=65535).contains(p))
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| field("p"))?;
        if m != mac_id(&fingerprint) {
            return Err(LinkError::MacIdMismatch);
        }
        let mut addresses = Vec::new();
        for (n, v) in &items {
            if n != "a" {
                continue;
            }
            let raw = v.as_deref().unwrap_or("");
            match parse_address(raw) {
                Ok(a) => addresses.push(a),
                Err(_) => return Err(LinkError::Address(raw.to_string())),
            }
        }
        addresses.truncate(MAX_ADDRESSES);
        let first_name = items
            .iter()
            .find(|(n, _)| n == "n")
            .and_then(|(_, v)| v.as_deref())
            .unwrap_or("");
        let clean = safetext::label(first_name, NAME_LIMIT);
        Ok(PairLink {
            mac_id: m.to_string(),
            fingerprint,
            secret,
            name: if clean.is_empty() {
                "Mac".to_string()
            } else {
                clean
            },
            port,
            addresses,
        })
    }

    /// The link as the QR code carries it (Pairing.swift:256-262).
    pub fn url(&self) -> String {
        let mut items: Vec<(&str, String)> = vec![
            ("v", "1".to_string()),
            ("m", self.mac_id.clone()),
            ("k", b64url_encode(&self.fingerprint)),
            ("s", b64url_encode(&self.secret)),
            ("n", self.name.clone()),
            ("p", self.port.to_string()),
        ];
        items.extend(
            self.addresses
                .iter()
                .take(MAX_ADDRESSES)
                .map(|a| ("a", a.text())),
        );
        let query: Vec<String> = items
            .iter()
            .map(|(k, v)| format!("{k}={}", escape(v)))
            .collect();
        format!("sill://pair?{}", query.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{b64url_encode, mac_id};

    fn link() -> PairLink {
        let fp: [u8; 32] = [
            0x34, 0x8a, 0x62, 0x9f, 0x5c, 0xee, 0xd0, 0x32, 0xc3, 0xe8, 0x70, 0x6e, 0xc4, 0x7d,
            0x9b, 0xfa, 0xfb, 0x00, 0xfb, 0x42, 0x50, 0xb0, 0x18, 0xdd, 0x96, 0x54, 0x35, 0xca,
            0x50, 0xcb, 0x83, 0x6e,
        ];
        PairLink {
            mac_id: mac_id(&fp),
            fingerprint: fp,
            secret: [0x22; 16],
            name: "Tom & Jerry".into(),
            port: 7455,
            addresses: vec![parse_address("100.1.2.3").unwrap()],
        }
    }

    #[test]
    fn roundtrip() {
        let l = link();
        let url = l.url();
        assert!(url.contains("n=Tom%20%26%20Jerry"), "{url}");
        assert_eq!(PairLink::parse(&url), Ok(l));
    }

    #[test]
    fn mac_id_mismatch() {
        let l = link();
        let other = if l.mac_id.starts_with('0') { "1" } else { "0" };
        let url = l.url().replace(
            &format!("m={}", l.mac_id),
            &format!("m={other}{}", &l.mac_id[1..]),
        );
        assert_eq!(PairLink::parse(&url), Err(LinkError::MacIdMismatch));
    }

    #[test]
    fn version() {
        assert_eq!(
            PairLink::parse(&link().url().replace("v=1", "v=2")),
            Err(LinkError::Version)
        );
    }

    #[test]
    fn missing_secret() {
        let l = link();
        let url = l
            .url()
            .replace(&format!("&s={}", b64url_encode(&l.secret)), "");
        assert_eq!(PairLink::parse(&url), Err(LinkError::Field("s".into())));
    }

    #[test]
    fn not_a_link() {
        assert_eq!(
            PairLink::parse(&link().url().replace("sill://", "https://")),
            Err(LinkError::NotALink)
        );
        assert_eq!(PairLink::parse("hello"), Err(LinkError::NotALink));
    }
}
