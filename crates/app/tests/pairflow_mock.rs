//! `pairflow` against the in-process mock host (127.0.0.1 only; no Bonjour).
#![cfg(feature = "video")]

use std::time::Duration;
use transom_app::pairflow::{ask, link_candidates, pair_code, pair_link, LinkPairError};
use transom_net::identity::DeviceIdentity;
use transom_net::mock::{MockConfig, MockHost};
use transom_net::tls::PinPolicy;
use transom_proto::identity::mac_id;
use transom_proto::pairlink::PairLink;

const ATTEMPT: Duration = Duration::from_secs(3);
const CODE: &str = "4829 1355 7208";

fn id() -> DeviceIdentity {
    DeviceIdentity::generate().unwrap().0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_returns_shown_and_fingerprint() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let (result, fp) = ask(&id(), vec![host.addr], ATTEMPT, "Test").await.unwrap();
    assert!(!result.ok);
    assert_eq!(result.reason.as_deref(), Some("shown"));
    assert_eq!(fp, host.fingerprint);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_code_pinned_succeeds() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let paired = pair_code(
        &id(),
        vec![host.addr],
        PinPolicy::Pinned(host.fingerprint),
        ATTEMPT,
        CODE,
        "Test",
    )
    .await
    .unwrap();
    assert!(!paired.mac_id.is_empty());
}

fn link_for(host: &MockHost, secret: [u8; 16]) -> PairLink {
    PairLink {
        mac_id: mac_id(&host.fingerprint),
        fingerprint: host.fingerprint,
        secret,
        name: "Mock".to_string(),
        port: host.addr.port(),
        addresses: vec![],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_link_explicit_candidate_succeeds() {
    let cfg = MockConfig::default();
    let secret = cfg.qr_secret;
    let host = MockHost::start(cfg).await.unwrap();
    let link = link_for(&host, secret);
    let candidates = link_candidates(&link, Some(vec![host.addr])).await.unwrap();
    assert_eq!(candidates, vec![vec![host.addr]]);
    let paired = pair_link(&id(), &link, candidates, ATTEMPT, "Test")
        .await
        .unwrap();
    assert_eq!(paired.fingerprint, host.fingerprint);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_link_exhausted_without_candidates() {
    let cfg = MockConfig::default();
    let secret = cfg.qr_secret;
    let host = MockHost::start(cfg).await.unwrap();
    let link = link_for(&host, secret);
    let r = pair_link(&id(), &link, vec![], ATTEMPT, "Test").await;
    assert!(matches!(r, Err(LinkPairError::Exhausted(None))));
}
