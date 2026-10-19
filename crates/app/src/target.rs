//! Turning the TARGET flags into a route.

use crate::cli::TargetArgs;
use anyhow::{anyhow, bail, Result};
use std::time::Duration;
use tokio::runtime::Runtime;
use transom_net::dial::REMOTE_DEFAULT_PORT;
use transom_net::discovery::{browse, recognize, FoundMac};
use transom_net::store::{SavedMac, Store};
use transom_net::supervisor::{Endpoint, Route, Trust};
use transom_proto::pairing::HomeDoor;

/// How long to listen for Bonjour answers.
pub const BROWSE_TIME: Duration = Duration::from_secs(3);

/// The found Mac named `name`: a case-insensitive match first, then a unique case-insensitive
/// prefix.
pub fn match_instance<'a>(found: &'a [FoundMac], name: &str) -> Result<&'a FoundMac> {
    let lower = name.to_lowercase();
    if let Some(f) = found.iter().find(|f| f.name.to_lowercase() == lower) {
        return Ok(f);
    }
    let prefixed: Vec<&FoundMac> = found
        .iter()
        .filter(|f| f.name.to_lowercase().starts_with(&lower))
        .collect();
    match prefixed.as_slice() {
        [one] => Ok(one),
        [] => bail!(
            "no Mac named \"{name}\" found on this network{}",
            names_suffix(found)
        ),
        many => bail!(
            "\"{name}\" matches several Macs: {}",
            many.iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn names_suffix(found: &[FoundMac]) -> String {
    if found.is_empty() {
        String::new()
    } else {
        format!(
            " (found: {})",
            found
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// The route for a Mac found by Bonjour that is not saved.
fn route_for_found(f: &FoundMac, plain: bool, saved: &[SavedMac]) -> Result<Route> {
    if f.door != HomeDoor::Plain {
        if let Some(mac) = recognize(f, saved) {
            return Ok(Route::SavedHome { mac: mac.clone() });
        }
    }
    match f.door {
        HomeDoor::PairingRequired => {
            bail!("{0} requires pairing: transom pair --mac \"{0}\"", f.name)
        }
        HomeDoor::Open => {
            eprintln!("the Mac's key is not pinned; pair it to pin it");
            Ok(Route::Direct {
                endpoint: Endpoint::Addrs(f.addresses.clone()),
                trust: Trust::AnyKey,
                label: f.name.clone(),
                saved: None,
                remote: false,
            })
        }
        HomeDoor::Plain => {
            if !plain {
                bail!(
                    "{} has an unencrypted door; pass --plain to connect anyway",
                    f.name
                );
            }
            Ok(Route::Direct {
                endpoint: Endpoint::Addrs(f.addresses.clone()),
                trust: Trust::Plain,
                label: f.name.clone(),
                saved: None,
                remote: false,
            })
        }
    }
}

pub fn resolve(t: &TargetArgs, store: &Store, _rt: &Runtime) -> Result<Route> {
    if let Some(host) = &t.host {
        let port = match (t.port, t.remote) {
            (Some(p), _) => p,
            (None, true) => REMOTE_DEFAULT_PORT,
            (None, false) => bail!(
                "give --port (the home door's port changes at every launch; use --mac to find it by Bonjour) or --remote"
            ),
        };
        let saved = match &t.mac {
            Some(name) => store.find(name).map_err(|e| anyhow!("{e}"))?,
            None => None,
        };
        let label = saved
            .as_ref()
            .map(|m| m.name.clone())
            .or_else(|| t.mac.clone())
            .unwrap_or_else(|| host.clone());
        let trust = if let Some(m) = &saved {
            Trust::Pinned(
                m.fingerprint_bytes()
                    .ok_or_else(|| anyhow!("the saved fingerprint of {} is damaged", m.name))?,
            )
        } else if t.any_key {
            Trust::AnyKey
        } else if t.plain {
            Trust::Plain
        } else {
            bail!("an address needs a saved Mac (--mac NAME), --any-key or --plain");
        };
        return Ok(Route::Direct {
            endpoint: Endpoint::Host(host.clone(), port),
            trust,
            label,
            saved,
            remote: t.remote,
        });
    }

    if let Some(name) = &t.mac {
        if let Some(mac) = store.find(name).map_err(|e| anyhow!("{e}"))? {
            return Ok(Route::SavedHome { mac });
        }
        let found = browse(BROWSE_TIME).map_err(|e| anyhow!("{e}"))?;
        let f = match_instance(&found, name)?;
        let saved = store.macs().map_err(|e| anyhow!("{e}"))?;
        return route_for_found(f, t.plain, &saved);
    }

    let saved = store.macs().map_err(|e| anyhow!("{e}"))?;
    match saved.as_slice() {
        [one] => Ok(Route::SavedHome { mac: one.clone() }),
        [] => {
            let found = browse(BROWSE_TIME).map_err(|e| anyhow!("{e}"))?;
            match found.as_slice() {
                [one] => route_for_found(one, t.plain, &saved),
                [] => bail!("no Mac running Sill found on this network"),
                many => bail!(
                    "several Macs found: {}; use --mac",
                    many.iter()
                        .map(|f| f.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        many => bail!(
            "several saved Macs: use --mac ({})",
            many.iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_net::store::SavedMac;
    use transom_proto::identity::b64url_encode;

    fn found(door: HomeDoor, tag: Option<String>) -> FoundMac {
        FoundMac {
            name: "Studio".into(),
            fullname: "Studio._sill._tcp.local.".into(),
            host: "studio.local.".into(),
            port: 1234,
            addresses: vec!["192.0.2.1:1234".parse().unwrap()],
            door,
            tag,
        }
    }

    fn saved() -> SavedMac {
        SavedMac {
            mac_id: "AAAA1111BBBB2222".into(),
            name: "Studio".into(),
            fingerprint: b64url_encode(&[1u8; 32]),
            recognition_key: b64url_encode(&[2u8; 32]),
            remote_port: None,
            addresses: vec![],
            issued_at: None,
            last_address: None,
        }
    }

    #[test]
    fn open_door_of_a_saved_mac_is_dialled_pinned() {
        let s = saved();
        let tag = transom_proto::pairing::recognition_tag(&[2u8; 32], [7u8; 6]);
        let f = found(HomeDoor::Open, Some(tag));
        assert!(recognize(&f, std::slice::from_ref(&s)).is_some());
        match route_for_found(&f, false, std::slice::from_ref(&s)).unwrap() {
            Route::SavedHome { mac } => assert_eq!(mac.mac_id, s.mac_id),
            other => panic!("unpinned route: {other:?}"),
        }
        let unknown = found(HomeDoor::Open, None);
        assert!(matches!(
            route_for_found(&unknown, false, &[s]).unwrap(),
            Route::Direct {
                trust: Trust::AnyKey,
                ..
            }
        ));
    }
}
