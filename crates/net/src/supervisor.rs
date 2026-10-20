//! Keeps a session alive: finds the Mac, dials it, runs the session, and reconnects with a
//! backoff when that is the right thing to do.

use crate::dial::{dial, resolve, HOME_DIAL_TIMEOUT, REMOTE_DIAL_TIMEOUT};
use crate::discovery::{browse_until, recognize};
use crate::identity::DeviceIdentity;
use crate::session::{run_session, EndKind, Outputs, SessionCommand, SessionEnd, SessionEvent};
use crate::session::{SessionOptions, SourceChoice};
use crate::store::{SavedAddress, SavedMac, Store};
use crate::tls::{self, PinPolicy, SESSION_ALPN};
use crate::{BoxIo, NetError};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use transom_proto::goodbye::goodbye_outcome;
use transom_proto::messages::{goodbye_reason, MacInfo, StreamSource};
use transom_proto::safetext;

#[derive(Clone, Debug)]
pub enum Endpoint {
    Host(String, u16),
    Addrs(Vec<SocketAddr>),
    /// Several hosts, tried in order: each is resolved, and the addresses of all of them are
    /// dialed in that order. A host that does not resolve is skipped.
    Hosts(Vec<(String, u16)>),
}

/// The addresses of every host that resolves, in order; the first error when none does.
async fn resolve_hosts(hosts: &[(String, u16)]) -> Result<Vec<SocketAddr>, NetError> {
    let mut all = Vec::new();
    let mut first_err = None;
    for (host, port) in hosts {
        match resolve(host, *port).await {
            Ok(addrs) => all.extend(addrs),
            Err(e) => {
                log::debug!("{host}: {e}");
                first_err.get_or_insert(e);
            }
        }
    }
    if all.is_empty() {
        return Err(first_err.unwrap_or_else(|| NetError::Connect("no address to dial".into())));
    }
    Ok(all)
}

#[derive(Clone, Copy, Debug)]
pub enum Trust {
    Pinned([u8; 32]),
    AnyKey,
    Plain,
}

#[derive(Clone, Debug)]
pub enum Route {
    /// A saved Mac at home: found again by Bonjour (recognition tag) at every attempt, dialed
    /// pinned.
    SavedHome { mac: SavedMac },
    /// An address: a saved Mac's (Pinned, `saved` Some) or an unsaved one (AnyKey or Plain).
    Direct {
        endpoint: Endpoint,
        trust: Trust,
        label: String,
        saved: Option<SavedMac>,
        remote: bool,
    },
}

impl Route {
    fn label(&self) -> String {
        match self {
            Route::SavedHome { mac } => mac.name.clone(),
            Route::Direct { label, .. } => label.clone(),
        }
    }

    fn is_remote(&self) -> bool {
        matches!(self, Route::Direct { remote: true, .. })
    }

    fn saved(&self) -> Option<&SavedMac> {
        match self {
            Route::SavedHome { mac } => Some(mac),
            Route::Direct { saved, .. } => saved.as_ref(),
        }
    }

    fn set_saved(&mut self, new: SavedMac) {
        match self {
            Route::SavedHome { mac } => *mac = new,
            Route::Direct { saved, .. } => {
                if saved.is_some() {
                    *saved = Some(new);
                }
            }
        }
    }
}

pub struct SupervisorConfig {
    pub route: Route,
    pub options: SessionOptions,
    pub identity: DeviceIdentity,
    pub store: Option<Store>,
}

#[derive(Clone)]
pub struct SessionHandle {
    tx: mpsc::UnboundedSender<SessionCommand>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl SessionHandle {
    pub fn send(&self, c: SessionCommand) {
        let _ = self.tx.send(c);
    }

    pub fn close(&self) {
        self.send(SessionCommand::Close);
    }

    /// The supervisor task, once: await it (briefly) after `close` so queued input is flushed.
    pub fn take_join(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.join.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// Resolve the route, dial, TLS or plain. Returns the stream, peer, fingerprint and label.
pub async fn open(
    route: &Route,
    id: &DeviceIdentity,
    alpn: &str,
) -> Result<(BoxIo, SocketAddr, Option<[u8; 32]>, String), NetError> {
    match route {
        Route::SavedHome { mac } => {
            let fp = mac
                .fingerprint_bytes()
                .ok_or_else(|| NetError::Store("the saved fingerprint is damaged".into()))?;
            let saved = mac.clone();
            let found = tokio::task::spawn_blocking(move || {
                browse_until(Duration::from_secs(3), |f| {
                    recognize(f, std::slice::from_ref(&saved)).is_some()
                })
            })
            .await
            .map_err(|e| NetError::Connect(format!("discovery: {e}")))??;
            let Some(found) = found else {
                return Err(NetError::Connect(format!(
                    "{} was not found on this network",
                    mac.name
                )));
            };
            let (tcp, peer) = dial(&found.addresses, HOME_DIAL_TIMEOUT).await?;
            let (tls, seen) = tls::connect(tcp, id, alpn, PinPolicy::Pinned(fp)).await?;
            Ok((Box::new(tls), peer, Some(seen), mac.name.clone()))
        }
        Route::Direct {
            endpoint,
            trust,
            label,
            remote,
            ..
        } => {
            let per_attempt = if *remote {
                REMOTE_DIAL_TIMEOUT
            } else {
                HOME_DIAL_TIMEOUT
            };
            let addrs = match endpoint {
                Endpoint::Host(host, port) => resolve(host, *port).await?,
                Endpoint::Addrs(a) => a.clone(),
                Endpoint::Hosts(hosts) => resolve_hosts(hosts).await?,
            };
            let (tcp, peer) = dial(&addrs, per_attempt).await?;
            match trust {
                Trust::Plain => {
                    tcp.set_nodelay(true)?;
                    Ok((Box::new(tcp), peer, None, label.clone()))
                }
                Trust::Pinned(fp) => {
                    let (tls, seen) = tls::connect(tcp, id, alpn, PinPolicy::Pinned(*fp)).await?;
                    Ok((Box::new(tls), peer, Some(seen), label.clone()))
                }
                Trust::AnyKey => {
                    let (tls, seen) = tls::connect(tcp, id, alpn, PinPolicy::AnyKey).await?;
                    Ok((Box::new(tls), peer, Some(seen), label.clone()))
                }
            }
        }
    }
}

/// Applies a command that arrives while no session runs. Returns true for Close.
fn apply_idle(options: &mut SessionOptions, cmd: Option<SessionCommand>) -> bool {
    match cmd {
        None | Some(SessionCommand::Close) => true,
        Some(SessionCommand::Select(s)) => {
            match s {
                StreamSource::Desktop {} => options.source = SourceChoice::Desktop,
                StreamSource::Window { id } => options.source = SourceChoice::Window(id),
                StreamSource::None {} => {}
            }
            false
        }
        Some(SessionCommand::Viewport(v)) => {
            options.viewport = Some(v);
            false
        }
        Some(_) => false,
    }
}

/// Runs `fut` while draining commands. None means the supervisor must stop.
async fn while_idle<F: std::future::Future>(
    fut: F,
    cmds: &mut mpsc::UnboundedReceiver<SessionCommand>,
    options: &mut SessionOptions,
) -> Option<F::Output> {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            r = &mut fut => return Some(r),
            c = cmds.recv() => {
                if apply_idle(options, c) {
                    return None;
                }
            }
        }
    }
}

fn mac_info_update(
    saved: &mut Option<SavedMac>,
    store: &Option<Store>,
    info: &MacInfo,
    fp: [u8; 32],
) {
    let Some(mac) = saved.as_mut() else { return };
    if mac.fingerprint_bytes() != Some(fp) {
        return;
    }
    if mac.issued_at.is_some_and(|t| t >= info.issued_at) {
        return;
    }
    mac.remote_port = u16::try_from(info.remote_port).ok();
    mac.addresses = info
        .addresses
        .iter()
        .map(|a| SavedAddress {
            host: a.host.clone(),
            port: a.port.and_then(|p| u16::try_from(p).ok()),
            kind: a.kind.clone(),
            via: a.via.clone(),
        })
        .collect();
    mac.issued_at = Some(info.issued_at);
    let name = safetext::label(&info.name, 64);
    if !name.is_empty() {
        mac.name = name;
    }
    if let Some(store) = store {
        if let Err(e) = store.save_mac(mac.clone()) {
            log::warn!("could not save the Mac's new details: {e}");
        }
    }
}

const BACKOFF: [u64; 5] = [2, 2, 4, 8, 10];

pub fn spawn_supervisor(
    rt: &tokio::runtime::Handle,
    cfg: SupervisorConfig,
    out: Outputs,
) -> SessionHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let join = rt.spawn(supervise(cfg, out, rx));
    SessionHandle {
        tx,
        join: Arc::new(Mutex::new(Some(join))),
    }
}

async fn supervise(
    cfg: SupervisorConfig,
    out: Outputs,
    mut cmds: mpsc::UnboundedReceiver<SessionCommand>,
) {
    let SupervisorConfig {
        mut route,
        mut options,
        identity,
        store,
    } = cfg;
    let mut saved = route.saved().cloned();
    let mut attempt = 0usize;
    loop {
        let label = route.label();
        out.emit(SessionEvent::Status(format!(
            "Connecting to {label}\u{2026}"
        )));
        let Some(opened) = while_idle(
            open(&route, &identity, SESSION_ALPN),
            &mut cmds,
            &mut options,
        )
        .await
        else {
            return;
        };
        let (backoff_text, backoff_kind): (String, EndKind) = match opened {
            Err(e) => match terminal_text(&e, &label) {
                Some((text, kind)) => {
                    out.emit(SessionEvent::Finished { text, kind });
                    return;
                }
                None => (e.to_string(), EndKind::ConnectFailed),
            },
            Ok((io, peer, fp, label)) => {
                let end = {
                    let mut on_info = |info: &MacInfo, fp: [u8; 32]| {
                        mac_info_update(&mut saved, &store, info, fp);
                    };
                    run_session(io, peer, fp, &mut options, &mut cmds, &out, &mut on_info).await
                };
                if let Some(updated) = &saved {
                    route.set_saved(updated.clone());
                }
                match end {
                    SessionEnd::UserClosed => return,
                    SessionEnd::Goodbye { goodbye, admitted } => {
                        if admitted {
                            attempt = 0;
                        }
                        let o = goodbye_outcome(Some(&goodbye), &label);
                        let remote_off = matches!(
                            goodbye.reason.as_str(),
                            goodbye_reason::REMOTE_OFF | goodbye_reason::INTERNET_OFF
                        );
                        let kind = EndKind::from_goodbye(&goodbye);
                        if !o.reconnect || (route.is_remote() && remote_off) {
                            out.emit(SessionEvent::Finished { text: o.text, kind });
                            return;
                        }
                        (o.text, kind)
                    }
                    SessionEnd::Lost { error, admitted } => {
                        if admitted {
                            attempt = 0;
                        }
                        match terminal_text(&error, &label) {
                            Some((text, kind)) => {
                                out.emit(SessionEvent::Finished { text, kind });
                                return;
                            }
                            None if admitted => (goodbye_outcome(None, &label).text, EndKind::Lost),
                            None => (error.to_string(), EndKind::ConnectFailed),
                        }
                    }
                }
            }
        };
        let delay = Duration::from_secs(BACKOFF[attempt.min(BACKOFF.len() - 1)]);
        attempt += 1;
        out.emit(SessionEvent::Ended {
            text: backoff_text,
            retry_in: Some(delay),
            kind: backoff_kind,
        });
        if while_idle(tokio::time::sleep(delay), &mut cmds, &mut options)
            .await
            .is_none()
        {
            return;
        }
    }
}

/// The final message (the CLI's) and kind for errors that reconnecting cannot fix.
fn terminal_text(e: &NetError, label: &str) -> Option<(String, EndKind)> {
    match e {
        NetError::NotPaired => Some((
            format!("{label} does not know this computer. Pair it first: transom pair"),
            EndKind::NotPaired,
        )),
        NetError::PinMismatch => Some((
            format!(
                "{label} presented a different key from the one saved. If the Mac was reset, run transom forget and pair again."
            ),
            EndKind::PinMismatch,
        )),
        _ => None,
    }
}
