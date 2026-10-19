//! Pairing with a Mac: by code, by link, and the "ask" and "cancel" requests.
//! Port of Remote.swift:112-209 and Scripts/sillclient.py:336-403.

use crate::dial::dial;
use crate::identity::DeviceIdentity;
use crate::session::read_message;
use crate::store::SavedMac;
use crate::tls::{self, PinPolicy, PAIRING_ALPN};
use crate::NetError;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use transom_proto::compat::pair_model;
use transom_proto::identity::{b64url_decode, b64url_encode, mac_id};
use transom_proto::messages::{
    from_json, pair_method, pair_reason, to_json, PairRequest, PairResult, PAIR_REQUEST_VERSION,
};
use transom_proto::pairing::{code_key, device_proof, verify_mac_proof};
use transom_proto::pairlink::PairLink;
use transom_proto::safetext::label as safetext_label;
use transom_proto::wire::{encode_message, kind, now_timestamp, MAX_OTHER_HOST_PAYLOAD};

const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

pub struct PairTarget {
    pub addrs: Vec<SocketAddr>,
    pub pin: PinPolicy,
    pub per_attempt: Duration,
}

#[derive(Clone, Debug)]
pub struct Paired {
    pub mac_id: String,
    pub name: String,
    pub fingerprint: [u8; 32],
    pub recognition_key: Vec<u8>,
    pub peer: SocketAddr,
}

impl Paired {
    pub fn to_saved(&self) -> SavedMac {
        SavedMac {
            mac_id: self.mac_id.clone(),
            name: self.name.clone(),
            fingerprint: b64url_encode(&self.fingerprint),
            recognition_key: b64url_encode(&self.recognition_key),
            remote_port: None,
            addresses: Vec::new(),
            issued_at: None,
            last_address: Some(self.peer.to_string()),
        }
    }
}

struct Open {
    stream: tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
    mac_fp: [u8; 32],
    peer: SocketAddr,
}

async fn open(
    id: &DeviceIdentity,
    addrs: &[SocketAddr],
    per_attempt: Duration,
    pin: PinPolicy,
) -> Result<Open, NetError> {
    let (tcp, peer) = dial(addrs, per_attempt).await?;
    let (stream, mac_fp) = tls::connect(tcp, id, PAIRING_ALPN, pin).await?;
    Ok(Open {
        stream,
        mac_fp,
        peer,
    })
}

fn request(method: &str, proof: String, name: &str) -> PairRequest {
    PairRequest {
        v: PAIR_REQUEST_VERSION,
        method: method.to_string(),
        proof,
        name: name.to_string(),
        model: Some(pair_model()),
        cable: None,
    }
}

/// Writes one kind 19 and reads one kind 20.
async fn exchange(
    stream: &mut tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
    req: &PairRequest,
) -> Result<PairResult, NetError> {
    let msg = encode_message(kind::PAIR_REQUEST, now_timestamp(), false, &to_json(req));
    stream.write_all(&msg).await?;
    stream.flush().await?;
    let read = tokio::time::timeout(
        REPLY_TIMEOUT,
        read_message(stream, |_| MAX_OTHER_HOST_PAYLOAD, || {}),
    )
    .await
    .map_err(|_| NetError::Timeout)??;
    let (header, payload) = read;
    if header.kind != kind::PAIR_RESULT {
        return Err(NetError::Protocol(format!(
            "expected a pairing result, got kind {}",
            header.kind
        )));
    }
    from_json::<PairResult>(&payload)
        .ok_or_else(|| NetError::Protocol("the pairing result did not decode".into()))
}

/// Method "ask": the Mac shows or opens its pairing window. Returns the result and the Mac's
/// fingerprint.
pub async fn ask(
    id: &DeviceIdentity,
    target: &PairTarget,
    name: &str,
) -> Result<(PairResult, [u8; 32]), NetError> {
    let mut o = open(id, &target.addrs, target.per_attempt, target.pin).await?;
    let result = exchange(
        &mut o.stream,
        &request(pair_method::ASK, String::new(), name),
    )
    .await?;
    Ok((result, o.mac_fp))
}

/// Method "cancel"; the answer is ignored.
pub async fn cancel(id: &DeviceIdentity, target: &PairTarget, name: &str) -> Result<(), NetError> {
    let mut o = open(id, &target.addrs, target.per_attempt, target.pin).await?;
    let _ = exchange(
        &mut o.stream,
        &request(pair_method::CANCEL, String::new(), name),
    )
    .await;
    Ok(())
}

enum Secret {
    Code(String),
    Link([u8; 16]),
}

fn normalized_code(code: &str) -> String {
    code.chars()
        .filter(|c| !matches!(c, ' ' | '-' | '\u{a0}'))
        .collect()
}

async fn derive_key(secret: &Secret, mac_fp: [u8; 32]) -> Result<Vec<u8>, NetError> {
    match secret {
        Secret::Link(s) => Ok(s.to_vec()),
        Secret::Code(code) => {
            let code = normalized_code(code);
            tokio::task::spawn_blocking(move || code_key(&code, &mac_fp).map(|k| k.to_vec()))
                .await
                .map_err(|e| NetError::Protocol(format!("key derivation: {e}")))?
                .ok_or_else(|| NetError::Protocol("the pairing code must be 12 digits".into()))
        }
    }
}

struct PairJob<'a> {
    addrs: &'a [SocketAddr],
    per_attempt: Duration,
    pin: PinPolicy,
    secret: Secret,
    method: &'a str,
    name: &'a str,
    fallback_mac_name: &'a str,
}

async fn pair_with_secret(id: &DeviceIdentity, job: PairJob<'_>) -> Result<Paired, NetError> {
    let PairJob {
        addrs,
        per_attempt,
        pin,
        secret,
        method,
        name,
        fallback_mac_name,
    } = job;
    // With a known Mac key the (slow) key derivation happens before the connection is made.
    let early = match pin {
        PinPolicy::Pinned(fp) => Some(derive_key(&secret, fp).await?),
        PinPolicy::AnyKey => None,
    };
    let mut retried = false;
    loop {
        let mut o = open(id, addrs, per_attempt, pin).await?;
        let key = match &early {
            Some(k) => k.clone(),
            None => derive_key(&secret, o.mac_fp).await?,
        };
        let proof = b64url_encode(&device_proof(&key, &id.fingerprint, &o.mac_fp));
        let result = exchange(&mut o.stream, &request(method, proof, name)).await?;
        if !result.ok {
            if !retried
                && result.reason.as_deref() == Some(pair_reason::BUSY)
                && result
                    .retry_after
                    .is_some_and(|s| (0.0..=10.0).contains(&s))
            {
                retried = true;
                let wait = result.retry_after.unwrap_or(0.0);
                tokio::time::sleep(Duration::from_secs_f64(wait)).await;
                continue;
            }
            return Err(NetError::PairRefused {
                reason: result.reason.clone().unwrap_or_default(),
                tries_left: result.tries_left,
                retry_after: result.retry_after,
                message: result.unknown_reason_message(),
            });
        }
        return accept(&result, &key, &o, id, fallback_mac_name);
    }
}

/// Accepts an ok only with a valid Mac proof, a Mac ID that matches the key we saw, a 32-byte
/// recognition key, and no `method` (a proof-less "cable" ok is refused).
fn accept(
    result: &PairResult,
    key: &[u8],
    o: &Open,
    id: &DeviceIdentity,
    fallback_name: &str,
) -> Result<Paired, NetError> {
    let proof = result
        .proof
        .as_deref()
        .and_then(b64url_decode)
        .ok_or(NetError::BadMacProof)?;
    if !verify_mac_proof(&proof, key, &o.mac_fp, &id.fingerprint) {
        return Err(NetError::BadMacProof);
    }
    let expected = mac_id(&o.mac_fp);
    if result.mac_id.as_deref() != Some(expected.as_str()) {
        return Err(NetError::BadMacProof);
    }
    let recognition_key = result
        .recognition_key
        .as_deref()
        .and_then(b64url_decode)
        .filter(|k| k.len() == 32)
        .ok_or(NetError::BadMacProof)?;
    if result.method.is_some() {
        return Err(NetError::BadMacProof);
    }
    let name = safetext_label(result.name.as_deref().unwrap_or(""), 64);
    let name = if name.is_empty() {
        fallback_name.to_string()
    } else {
        name
    };
    Ok(Paired {
        mac_id: expected,
        name,
        fingerprint: o.mac_fp,
        recognition_key,
        peer: o.peer,
    })
}

pub async fn pair_with_code(
    id: &DeviceIdentity,
    target: &PairTarget,
    code: &str,
    name: &str,
) -> Result<Paired, NetError> {
    pair_with_secret(
        id,
        PairJob {
            addrs: &target.addrs,
            per_attempt: target.per_attempt,
            pin: target.pin,
            secret: Secret::Code(code.to_string()),
            method: pair_method::CODE,
            name,
            fallback_mac_name: "Mac",
        },
    )
    .await
}

/// Pins `link.fingerprint`; the key is `link.secret`; method "qr".
pub async fn pair_with_link(
    id: &DeviceIdentity,
    link: &PairLink,
    addrs: Vec<SocketAddr>,
    per_attempt: Duration,
    name: &str,
) -> Result<Paired, NetError> {
    pair_with_secret(
        id,
        PairJob {
            addrs: &addrs,
            per_attempt,
            pin: PinPolicy::Pinned(link.fingerprint),
            secret: Secret::Link(link.secret),
            method: pair_method::QR,
            name,
            fallback_mac_name: &link.name,
        },
    )
    .await
}
