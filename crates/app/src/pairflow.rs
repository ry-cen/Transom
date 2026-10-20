//! The async pairing steps, shared by `transom pair` (pair_cmd.rs, through `block_on`) and the
//! graphical app (tokio tasks), so key derivation never runs on a UI thread (docs/ui-spec.md
//! section 8.4). WP-B2 moves the bodies here from pair_cmd.rs; the signatures are frozen
//! (wave A). Never log a code or a link's secret.

use std::net::SocketAddr;
use std::time::Duration;
use transom_net::dial::{HOME_DIAL_TIMEOUT, REMOTE_DIAL_TIMEOUT};
use transom_net::identity::DeviceIdentity;
use transom_net::pairing::Paired;
use transom_net::tls::PinPolicy;
use transom_net::NetError;
use transom_proto::messages::PairResult;
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

fn not_built() -> NetError {
    NetError::Connect("pairflow is not built yet".into())
}

/// Method "ask" with any key: the result and the Mac's fingerprint (pin it for the code).
pub async fn ask(
    id: &DeviceIdentity,
    addrs: Vec<SocketAddr>,
    per_attempt: Duration,
    name: &str,
) -> Result<(PairResult, [u8; 32]), NetError> {
    let _ = (id, addrs, per_attempt, name);
    Err(not_built())
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
    let _ = (id, addrs, pin, per_attempt, code, name);
    Err(not_built())
}

/// Method "cancel", best effort: errors are logged at debug and dropped.
pub async fn cancel(
    id: &DeviceIdentity,
    addrs: Vec<SocketAddr>,
    pin: PinPolicy,
    per_attempt: Duration,
    name: &str,
) {
    let _ = (id, addrs, pin, per_attempt, name);
}

/// The address lists to try for a link, in order: `explicit` alone when given; else every TLS
/// door Bonjour finds (a blocking browse of `target::BROWSE_TIME`, in `spawn_blocking`), then
/// each of the link's addresses resolved (unresolvable ones skipped, logged at debug).
pub async fn link_candidates(
    link: &PairLink,
    explicit: Option<Vec<SocketAddr>>,
) -> Result<Vec<Vec<SocketAddr>>, NetError> {
    let _ = (link, explicit);
    Err(not_built())
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
    let _ = (id, link, candidates, per_attempt, name);
    Err(LinkPairError::Exhausted(None))
}
