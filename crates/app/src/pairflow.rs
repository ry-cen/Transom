//! The async pairing steps, shared by `transom pair` (pair_cmd.rs, through `block_on`) and the
//! graphical app (tokio tasks), so key derivation never runs on a UI thread (docs/ui-spec.md
//! section 8.4). The bodies moved here from pair_cmd.rs in WP-B2; the signatures are frozen. Never log a code or a link's secret.

use crate::target::BROWSE_TIME;
use std::net::SocketAddr;
use std::time::Duration;
use transom_net::dial::{resolve, HOME_DIAL_TIMEOUT, REMOTE_DIAL_TIMEOUT};
use transom_net::discovery::browse;
use transom_net::identity::DeviceIdentity;
use transom_net::pairing::{self, PairTarget, Paired};
use transom_net::tls::PinPolicy;
use transom_net::NetError;
use transom_proto::messages::PairResult;
use transom_proto::pairing::HomeDoor;
use transom_proto::pairlink::PairLink;

/// Why a link pairing failed.
#[derive(Debug)]
pub enum LinkPairError {
    /// A refusal or another error that trying the next candidate cannot fix (pair_cmd.rs's
    /// `bail!("{}", print_failure(&e))`).
    Refused(NetError),
    /// Every candidate failed to connect; the last error, or None when there was no candidate.
    Exhausted(Option<NetError>),
}

/// The per-address dial timeout: REMOTE_DIAL_TIMEOUT for a remote door, else HOME_DIAL_TIMEOUT.
pub fn per_attempt(remote: bool) -> Duration {
    if remote {
        REMOTE_DIAL_TIMEOUT
    } else {
        HOME_DIAL_TIMEOUT
    }
}

/// Method "ask" with any key: the result and the Mac's fingerprint (pin it for the code).
pub async fn ask(
    id: &DeviceIdentity,
    addrs: Vec<SocketAddr>,
    per_attempt: Duration,
    name: &str,
) -> Result<(PairResult, [u8; 32]), NetError> {
    let target = PairTarget {
        addrs,
        pin: PinPolicy::AnyKey,
        per_attempt,
    };
    pairing::ask(id, &target, name).await
}

/// Pairs with a checked 12-digit code.
pub async fn pair_code(
    id: &DeviceIdentity,
    addrs: Vec<SocketAddr>,
    pin: PinPolicy,
    per_attempt: Duration,
    code: &str,
    name: &str,
) -> Result<Paired, NetError> {
    let target = PairTarget {
        addrs,
        pin,
        per_attempt,
    };
    pairing::pair_with_code(id, &target, code, name).await
}

/// Method "cancel", best effort: errors are logged at debug and dropped.
pub async fn cancel(
    id: &DeviceIdentity,
    addrs: Vec<SocketAddr>,
    pin: PinPolicy,
    per_attempt: Duration,
    name: &str,
) {
    let target = PairTarget {
        addrs,
        pin,
        per_attempt,
    };
    if let Err(e) = pairing::cancel(id, &target, name).await {
        log::debug!("cancel: {e}");
    }
}

/// The address lists to try for a link, in order: `explicit` alone when given; else every TLS
/// door Bonjour finds (a blocking browse of `target::BROWSE_TIME`, in `spawn_blocking`), then
/// each of the link's addresses resolved (unresolvable ones skipped, logged at debug).
pub async fn link_candidates(
    link: &PairLink,
    explicit: Option<Vec<SocketAddr>>,
) -> Result<Vec<Vec<SocketAddr>>, NetError> {
    if let Some(addrs) = explicit {
        return Ok(vec![addrs]);
    }
    let found = tokio::task::spawn_blocking(move || browse(BROWSE_TIME))
        .await
        .map_err(|_| NetError::Connect("browse failed".into()))??;
    let mut candidates: Vec<Vec<SocketAddr>> = found
        .iter()
        .filter(|f| matches!(f.door, HomeDoor::PairingRequired | HomeDoor::Open))
        .map(|f| f.addresses.clone())
        .collect();
    for a in &link.addresses {
        let port = a.port.unwrap_or(link.port);
        match resolve(&a.host, port).await {
            Ok(addrs) => candidates.push(addrs),
            Err(e) => log::debug!("{}: {e}", a.host),
        }
    }
    Ok(candidates)
}

/// Tries each candidate in order with the link's pinned key; a connect-type failure
/// (PinMismatch, Connect, Timeout, Closed) moves to the next one.
pub async fn pair_link(
    id: &DeviceIdentity,
    link: &PairLink,
    candidates: Vec<Vec<SocketAddr>>,
    per_attempt: Duration,
    name: &str,
) -> Result<Paired, LinkPairError> {
    let mut last: Option<NetError> = None;
    for addrs in candidates {
        match pairing::pair_with_link(id, link, addrs, per_attempt, name).await {
            Ok(paired) => return Ok(paired),
            Err(
                e @ (NetError::PinMismatch
                | NetError::Connect(_)
                | NetError::Timeout
                | NetError::Closed),
            ) => last = Some(e),
            Err(e) => return Err(LinkPairError::Refused(e)),
        }
    }
    Err(LinkPairError::Exhausted(last))
}
