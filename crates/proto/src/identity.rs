//! Device and Mac identity: SPKI, fingerprints, Mac IDs. Port of RemoteIdentity.swift:12-89.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// DER prefix of a P-256 SubjectPublicKeyInfo; the 65-byte point follows
/// (RemoteIdentity.swift:33-44).
pub const SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// base64url without padding.
pub fn b64url_encode(d: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(d)
}

/// Canonical unpadded base64url only. Port of RemoteIdentity.swift:22-31.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    if !s
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || s.len() % 4 == 1
    {
        return None;
    }
    let d = URL_SAFE_NO_PAD.decode(s).ok()?;
    if b64url_encode(&d) == s {
        Some(d)
    } else {
        None
    }
}

/// SPKI of a 65-byte uncompressed point; None otherwise.
pub fn spki_from_point(point: &[u8]) -> Option<Vec<u8>> {
    if point.len() != 65 || point[0] != 0x04 {
        return None;
    }
    let mut v = SPKI_PREFIX.to_vec();
    v.extend_from_slice(point);
    Some(v)
}

/// SHA-256(SPKI_PREFIX || point). The caller checked the length.
pub fn fingerprint_of_point(point: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(SPKI_PREFIX);
    h.update(point);
    h.finalize().into()
}

/// SHA-256 of `spki`, only when it is exactly SPKI_PREFIX || 0x04 || 64 bytes.
pub fn fingerprint_of_spki(spki: &[u8]) -> Option<[u8; 32]> {
    if spki.len() != SPKI_PREFIX.len() + 65
        || spki[..SPKI_PREFIX.len()] != SPKI_PREFIX
        || spki[SPKI_PREFIX.len()] != 0x04
    {
        return None;
    }
    Some(Sha256::digest(spki).into())
}

/// One DER TLV at `i`: (tag, content start, end). Long-form lengths of 1 to 4 bytes; every
/// index is bounds-checked and `end` never exceeds `b.len()`.
fn tlv(b: &[u8], i: usize) -> Option<(u8, usize, usize)> {
    let tag = *b.get(i)?;
    let first = *b.get(i.checked_add(1)?)?;
    let (len, start) = if first < 0x80 {
        (usize::from(first), i.checked_add(2)?)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for k in 0..n {
            len = (len << 8) | usize::from(*b.get(i.checked_add(2 + k)?)?);
        }
        (len, i.checked_add(2 + n)?)
    };
    let end = start.checked_add(len)?;
    if end > b.len() {
        return None;
    }
    Some((tag, start, end))
}

/// The whole SPKI TLV of an X.509 certificate. A minimal DER walk that never panics.
pub fn spki_from_certificate(der: &[u8]) -> Option<&[u8]> {
    let (tag, cs, end) = tlv(der, 0)?;
    if tag != 0x30 {
        return None;
    }
    let cert = &der[..end];
    let (tag, tcs, tend) = tlv(cert, cs)?;
    if tag != 0x30 {
        return None;
    }
    let tbs = &cert[..tend];
    let mut pos = tcs;
    let (first_tag, _, first_end) = tlv(tbs, pos)?;
    if first_tag == 0xA0 {
        pos = first_end;
    }
    // serial, signature algorithm, issuer, validity, subject, then the SPKI.
    for _ in 0..5 {
        let (_, _, e) = tlv(tbs, pos)?;
        pos = e;
    }
    let (tag, _, e) = tlv(tbs, pos)?;
    if tag != 0x30 {
        return None;
    }
    tbs.get(pos..e)
}

/// Fingerprint of a certificate's key; None for a key that is not P-256.
pub fn fingerprint_of_certificate(der: &[u8]) -> Option<[u8; 32]> {
    fingerprint_of_spki(spki_from_certificate(der)?)
}

/// Crockford base32 of the fingerprint's first 10 bytes, 16 characters; "" when shorter.
/// Port of RemoteIdentity.swift:68-83.
pub fn mac_id(fingerprint: &[u8]) -> String {
    if fingerprint.len() < 10 {
        return String::new();
    }
    let n = fingerprint[..10]
        .iter()
        .fold(0u128, |acc, &b| (acc << 8) | u128::from(b));
    (0..16)
        .map(|i| char::from(ALPHABET[((n >> (75 - 5 * i)) & 31) as usize]))
        .collect()
}

/// 16 characters of the Crockford alphabet (upper case).
pub fn is_well_formed_mac_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|c| ALPHABET.contains(&c))
}
