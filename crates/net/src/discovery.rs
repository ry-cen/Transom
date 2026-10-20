//! Finding Macs on the local network by Bonjour (`_sill._tcp`). Port of Pairing.swift:140-221.

use crate::dial::order_addresses;
use crate::store::SavedMac;
use crate::NetError;
use mdns_sd::{ScopedIp, ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV6};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use transom_proto::pairing::{home_door, tag_matches, HomeDoor};

pub const SERVICE: &str = "_sill._tcp.local.";

#[derive(Clone, Debug)]
pub struct FoundMac {
    pub name: String,
    pub fullname: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<SocketAddr>,
    pub door: HomeDoor,
    pub tag: Option<String>,
}

/// "Ryan’s MacBook Pro._sill._tcp.local." to "Ryan’s MacBook Pro".
pub fn instance_name(fullname: &str) -> String {
    let base = fullname
        .strip_suffix(SERVICE)
        .map(|s| s.strip_suffix('.').unwrap_or(s))
        .unwrap_or(fullname);
    base.replace("\\.", ".").replace("\\\\", "\\")
}

fn to_found(info: &mdns_sd::ResolvedService) -> FoundMac {
    let port = info.port;
    let addrs: Vec<SocketAddr> = info
        .addresses
        .iter()
        .filter_map(|a| match a {
            ScopedIp::V4(v4) => Some(SocketAddr::new((*v4.addr()).into(), port)),
            ScopedIp::V6(v6) => Some(SocketAddr::V6(SocketAddrV6::new(
                *v6.addr(),
                port,
                0,
                v6.scope_id().index,
            ))),
            _ => None,
        })
        .collect();
    FoundMac {
        name: transom_proto::safetext::label(
            &instance_name(&info.fullname),
            transom_proto::safetext::LABEL_LIMIT,
        ),
        fullname: info.fullname.clone(),
        host: info.host.clone(),
        port,
        addresses: order_addresses(addrs),
        door: home_door(info.txt_properties.get_property_val("p")),
        tag: info
            .txt_properties
            .get_property_val_str("r")
            .map(String::from),
    }
}

fn run_browse(
    timeout: Duration,
    mut on_found: impl FnMut(&FoundMac) -> bool,
) -> Result<(), NetError> {
    let daemon = ServiceDaemon::new().map_err(|e| NetError::Connect(format!("mDNS: {e}")))?;
    let result = (|| {
        let rx = daemon
            .browse(SERVICE)
            .map_err(|e| NetError::Connect(format!("mDNS: {e}")))?;
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            match rx.recv_timeout(left) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if on_found(&to_found(&info)) {
                        return Ok(());
                    }
                }
                Ok(_) => {}
                Err(_) => return Ok(()),
            }
        }
    })();
    let _ = daemon.shutdown();
    result
}

/// Blocking: browses for `timeout`; one entry per fullname (the latest resolution), by name.
pub fn browse(timeout: Duration) -> Result<Vec<FoundMac>, NetError> {
    let mut found: HashMap<String, FoundMac> = HashMap::new();
    run_browse(timeout, |f| {
        found.insert(f.fullname.clone(), f.clone());
        false
    })?;
    let mut all: Vec<FoundMac> = found.into_values().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name).then(a.fullname.cmp(&b.fullname)));
    Ok(all)
}

/// Blocking: the first Mac for which `want` is true, or None at `timeout`.
pub fn browse_until(
    timeout: Duration,
    want: impl Fn(&FoundMac) -> bool,
) -> Result<Option<FoundMac>, NetError> {
    let mut hit = None;
    run_browse(timeout, |f| {
        if want(f) {
            hit = Some(f.clone());
            true
        } else {
            false
        }
    })?;
    Ok(hit)
}

/// One change seen by a live browse (`watch`).
#[derive(Clone, Debug)]
pub enum BrowseEvent {
    /// A Mac resolved, or resolved again (same `fullname`: replace the earlier entry).
    Found(FoundMac),
    /// The Mac with this `fullname` left the network.
    Removed { fullname: String },
}

/// A live browse of `_sill._tcp`, for the app's lifetime. Stopping it (or dropping it) shuts its
/// mDNS daemon down and joins its reader thread.
pub struct Watch {
    daemon: Option<ServiceDaemon>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

/// Browses `_sill._tcp` until stopped. Every resolution is sent as `Found` and every removal as
/// `Removed`, each followed by `wake()`.
pub fn watch(
    out: crossbeam_channel::Sender<BrowseEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<Watch, NetError> {
    let daemon = ServiceDaemon::new().map_err(|e| NetError::Connect(format!("mDNS: {e}")))?;
    let rx = match daemon.browse(SERVICE) {
        Ok(rx) => rx,
        Err(e) => {
            let _ = daemon.shutdown();
            return Err(NetError::Connect(format!("mDNS: {e}")));
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("transom-watch".into())
        .spawn(move || loop {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let event = match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(ServiceEvent::ServiceResolved(info)) => BrowseEvent::Found(to_found(&info)),
                Ok(ServiceEvent::ServiceRemoved(_ty, fullname)) => {
                    BrowseEvent::Removed { fullname }
                }
                Ok(_) => continue,
                Err(mdns_sd::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            if out.send(event).is_err() {
                break;
            }
            wake();
        });
    let thread = match thread {
        Ok(t) => t,
        Err(e) => {
            let _ = daemon.shutdown();
            return Err(NetError::Connect(format!("mDNS: {e}")));
        }
    };
    Ok(Watch {
        daemon: Some(daemon),
        thread: Some(thread),
        stop,
    })
}

impl Watch {
    /// Shuts the daemon down and joins the reader thread.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.shutdown();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// The saved Mac whose recognition key matches `found.tag`.
pub fn recognize<'a>(found: &FoundMac, saved: &'a [SavedMac]) -> Option<&'a SavedMac> {
    let tag = found.tag.as_deref()?;
    saved.iter().find(|m| {
        m.recognition_key_bytes()
            .is_some_and(|rk| tag_matches(tag, &rk))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_strips_suffix() {
        assert_eq!(
            instance_name("Ryan’s MacBook Pro._sill._tcp.local."),
            "Ryan’s MacBook Pro"
        );
        assert_eq!(instance_name("a\\.b._sill._tcp.local."), "a.b");
        assert_eq!(instance_name("plain"), "plain");
    }
}
