//! Pairing code, proofs, recognition tag and the Bonjour door. Ports of Pairing.swift:14-221.

use crate::identity::{b64url_decode, b64url_encode};
use hmac::Mac;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

pub const CODE_LEN: usize = 12;
pub const PBKDF2_ROUNDS: u32 = 600_000;
const LABEL: &str = "sill-pair-v1";

/// Damm quasigroup of order 10 (Pairing.swift:18-23).
const TABLE: [[u8; 10]; 10] = [
    [0, 3, 1, 7, 5, 9, 8, 6, 4, 2],
    [7, 0, 9, 2, 1, 5, 4, 8, 6, 3],
    [4, 2, 0, 6, 8, 7, 1, 3, 5, 9],
    [1, 7, 5, 0, 9, 8, 3, 4, 2, 6],
    [6, 1, 2, 3, 0, 4, 5, 9, 7, 8],
    [3, 6, 7, 4, 2, 0, 9, 5, 8, 1],
    [5, 8, 6, 9, 7, 2, 0, 1, 3, 4],
    [8, 9, 4, 5, 3, 6, 2, 0, 1, 7],
    [9, 4, 3, 8, 6, 1, 7, 2, 0, 5],
    [2, 5, 8, 1, 4, 3, 6, 7, 9, 0],
];

/// The Damm check digit for `digits` (each 0..=9). Port of Pairing.swift:26-28. A value above 9
/// is ignored (treated as no change), never a panic.
pub fn damm_check_digit(digits: &[u8]) -> u8 {
    digits.iter().fold(0u8, |interim, &d| {
        TABLE
            .get(usize::from(interim))
            .and_then(|row| row.get(usize::from(d)))
            .copied()
            .unwrap_or(interim)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodeProblem {
    #[error("A code has 12 digits.")]
    Length,
    #[error("That code has a typo. Check it against your Mac.")]
    Typo,
}

/// Port of Pairing.swift:50-55: spaces, hyphens and no-break spaces removed, exactly 12 ASCII
/// digits, a valid check digit.
pub fn check_code(input: &str) -> Result<String, CodeProblem> {
    let kept: String = input
        .chars()
        .filter(|&c| c != ' ' && c != '-' && c != '\u{A0}')
        .collect();
    if kept.len() != CODE_LEN || !kept.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CodeProblem::Length);
    }
    let digits: Vec<u8> = kept.bytes().map(|b| b - b'0').collect();
    if damm_check_digit(&digits) != 0 {
        return Err(CodeProblem::Typo);
    }
    Ok(kept)
}

/// "4829 1355 7208" (Pairing.swift:60-65).
pub fn group_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The typed path's key (Pairing.swift:79-95): PBKDF2-HMAC-SHA256(code, "sill-pair-v1" || fp_mac,
/// 600000 rounds). None unless `code` is 12 ASCII digits.
pub fn code_key(code: &str, mac_fp: &[u8; 32]) -> Option<[u8; 32]> {
    if code.len() != CODE_LEN || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut salt = LABEL.as_bytes().to_vec();
    salt.extend_from_slice(mac_fp);
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(code.as_bytes(), &salt, PBKDF2_ROUNDS, &mut out);
    Some(out)
}

fn device_message(device_fp: &[u8; 32], mac_fp: &[u8; 32]) -> Vec<u8> {
    let mut m = format!("{LABEL} device").into_bytes();
    m.push(0);
    m.extend_from_slice(device_fp);
    m.extend_from_slice(mac_fp);
    m
}

fn mac_message(mac_fp: &[u8; 32], device_fp: &[u8; 32]) -> Vec<u8> {
    let mut m = format!("{LABEL} mac").into_bytes();
    m.push(0);
    m.extend_from_slice(mac_fp);
    m.extend_from_slice(device_fp);
    m
}

/// HMAC-SHA256 keyed by `key` over `msg`. HMAC takes any key length, so the `None` arm is
/// unreachable; it is a value rather than a panic.
fn hmac(key: &[u8], msg: &[u8]) -> Option<HmacSha256> {
    let mut m = <HmacSha256 as hmac::KeyInit>::new_from_slice(key).ok()?;
    m.update(msg);
    Some(m)
}

fn tag_of(key: &[u8], msg: &[u8]) -> [u8; 32] {
    match hmac(key, msg) {
        Some(m) => m.finalize().into_bytes().into(),
        None => [0u8; 32],
    }
}

fn verify(proof: &[u8], key: &[u8], msg: &[u8]) -> bool {
    hmac(key, msg).is_some_and(|m| m.verify_slice(proof).is_ok())
}

/// proof_D, sent in kind 19 (Pairing.swift:112-114).
pub fn device_proof(key: &[u8], device_fp: &[u8; 32], mac_fp: &[u8; 32]) -> [u8; 32] {
    tag_of(key, &device_message(device_fp, mac_fp))
}

/// proof_M, sent by the Mac in kind 20 (Pairing.swift:117-119).
pub fn mac_proof(key: &[u8], mac_fp: &[u8; 32], device_fp: &[u8; 32]) -> [u8; 32] {
    tag_of(key, &mac_message(mac_fp, device_fp))
}

/// Constant-time check of proof_D (Pairing.swift:122-124).
pub fn verify_device_proof(
    proof: &[u8],
    key: &[u8],
    device_fp: &[u8; 32],
    mac_fp: &[u8; 32],
) -> bool {
    verify(proof, key, &device_message(device_fp, mac_fp))
}

/// Constant-time check of proof_M (Pairing.swift:127-129).
pub fn verify_mac_proof(proof: &[u8], key: &[u8], mac_fp: &[u8; 32], device_fp: &[u8; 32]) -> bool {
    verify(proof, key, &mac_message(mac_fp, device_fp))
}

const TAG_LABEL: &[u8] = b"sill-tag-v1";

fn tag_mac(recognition_key: &[u8], nonce: &[u8]) -> [u8; 6] {
    let mut msg = TAG_LABEL.to_vec();
    msg.extend_from_slice(nonce);
    let full = tag_of(recognition_key, &msg);
    [full[0], full[1], full[2], full[3], full[4], full[5]]
}

/// base64url(nonce || HMAC(rk, "sill-tag-v1" || nonce)[..6]) (Pairing.swift:140-157).
pub fn recognition_tag(recognition_key: &[u8], nonce: [u8; 6]) -> String {
    let mut tag = nonce.to_vec();
    tag.extend_from_slice(&tag_mac(recognition_key, &nonce));
    b64url_encode(&tag)
}

/// Whether `tag` was made with `recognition_key`; constant time in the 6 compared bytes.
pub fn tag_matches(tag: &str, recognition_key: &[u8]) -> bool {
    let Some(bytes) = b64url_decode(tag) else {
        return false;
    };
    if bytes.len() != 12 {
        return false;
    }
    let expected = tag_mac(recognition_key, &bytes[..6]);
    let diff = expected
        .iter()
        .zip(&bytes[6..])
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    diff == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeDoor {
    Plain,
    PairingRequired,
    Open,
}

/// The door a TXT `p` describes (Pairing.swift:180-217). `p` as mdns-sd returns it: None = no
/// key; Some(None) = key without value; Some(Some(bytes)).
pub fn home_door(p: Option<Option<&[u8]>>) -> HomeDoor {
    match p {
        None => HomeDoor::Plain,
        Some(Some(b"0")) => HomeDoor::Open,
        Some(_) => HomeDoor::PairingRequired,
    }
}
