//! The Macs screen's rows and the plans to connect to them (docs/ui-spec.md sections 3.3 and
//! 3.9). Pure. The signature of `rows` is frozen (wave A).

use crate::gui::model::{ConnectPlan, MacRow, Prefs, RouteDesc, RowKey, RowWord};
use crate::gui::texts;
use transom_net::dial::REMOTE_DEFAULT_PORT;
use transom_net::discovery::{recognize, FoundMac};
use transom_net::store::SavedMac;
use transom_net::supervisor::{Endpoint, Route, Trust};
use transom_proto::address::parse_address;
use transom_proto::pairing::HomeDoor;
use transom_proto::safetext::{label, LABEL_LIMIT};

/// `host:port`, with an IPv6 host in brackets.
pub fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The found Mac this saved Mac is (discovery::recognize).
pub fn found_for<'a>(mac: &SavedMac, found: &'a [FoundMac]) -> Option<&'a FoundMac> {
    let one = std::slice::from_ref(mac);
    found.iter().find(|f| recognize(f, one).is_some())
}

/// Spec 3.9, in order: (host, port, via). The manual address first (via None), then the
/// reported addresses that are not "lan".
pub fn remote_hosts(mac: &SavedMac, prefs: &Prefs) -> Vec<(String, u16, Option<String>)> {
    let fallback = mac.remote_port.unwrap_or(REMOTE_DEFAULT_PORT);
    let mut out = Vec::new();
    if let Some(manual) = prefs
        .macs
        .get(&mac.mac_id)
        .and_then(|p| p.remote_host.as_deref())
    {
        if let Ok(p) = parse_address(manual) {
            out.push((p.host, p.port.unwrap_or(fallback), None));
        }
    }
    for a in mac.addresses.iter().filter(|a| a.kind != "lan") {
        let via = if a.via.is_empty() {
            None
        } else {
            Some(a.via.clone())
        };
        out.push((a.host.clone(), a.port.unwrap_or(fallback), via));
    }
    out
}

/// The Remote plan, or None with no remote address.
pub fn plan_remote(mac: &SavedMac, prefs: &Prefs) -> Option<ConnectPlan> {
    let hosts: Vec<(String, u16)> = remote_hosts(mac, prefs)
        .into_iter()
        .map(|(h, p, _)| (h, p))
        .collect();
    if hosts.is_empty() {
        None
    } else {
        Some(ConnectPlan::Remote {
            mac: mac.clone(),
            hosts,
        })
    }
}

/// The row's primary connect plan (Home or Remote rows only; None otherwise).
pub fn plan(row: &MacRow, prefs: &Prefs) -> Option<ConnectPlan> {
    let mac = row.saved.as_ref()?;
    match row.word {
        RowWord::Home => Some(ConnectPlan::Home(mac.clone())),
        RowWord::Remote => plan_remote(mac, prefs),
        _ => None,
    }
}

/// The supervisor route and its description; None when a saved Mac's fingerprint is unreadable.
pub fn route_for(plan: &ConnectPlan, prefs: &Prefs) -> Option<(Route, RouteDesc)> {
    match plan {
        ConnectPlan::Home(mac) => Some((Route::SavedHome { mac: mac.clone() }, RouteDesc::Home)),
        ConnectPlan::Remote { mac, hosts } => {
            let fp = mac.fingerprint_bytes()?;
            let via = remote_hosts(mac, prefs).first().and_then(|h| h.2.clone());
            Some((
                Route::Direct {
                    endpoint: Endpoint::Hosts(hosts.clone()),
                    trust: Trust::Pinned(fp),
                    label: mac.name.clone(),
                    saved: Some(mac.clone()),
                    remote: true,
                },
                RouteDesc::Remote { via },
            ))
        }
        ConnectPlan::Unpinned(f) => Some((
            Route::Direct {
                endpoint: Endpoint::Addrs(f.addresses.clone()),
                trust: Trust::AnyKey,
                label: f.name.clone(),
                saved: None,
                remote: false,
            },
            RouteDesc::Unpinned,
        )),
        ConnectPlan::Plain(f) => Some((
            Route::Direct {
                endpoint: Endpoint::Addrs(f.addresses.clone()),
                trust: Trust::Plain,
                label: f.name.clone(),
                saved: None,
                remote: false,
            },
            RouteDesc::Plain,
        )),
    }
}

/// (mac_id, display name) of a plan.
pub fn plan_mac(plan: &ConnectPlan) -> (Option<String>, String) {
    match plan {
        ConnectPlan::Home(m) | ConnectPlan::Remote { mac: m, .. } => {
            (Some(m.mac_id.clone()), texts::mac_name(&m.name))
        }
        ConnectPlan::Unpinned(f) | ConnectPlan::Plain(f) => (None, texts::mac_name(&f.name)),
    }
}

fn first_ip(f: &FoundMac) -> String {
    f.addresses
        .first()
        .map(|a| a.ip().to_string())
        .unwrap_or_default()
}

/// The rows of "MY MACS" (saved, by name) and "OTHER MACS ON THIS NETWORK" (found, unsaved, by
/// name).
pub fn rows(saved: &[SavedMac], found: &[FoundMac], prefs: &Prefs) -> (Vec<MacRow>, Vec<MacRow>) {
    let mut mine: Vec<MacRow> = saved
        .iter()
        .map(|mac| {
            let hosts = remote_hosts(mac, prefs);
            let f = found_for(mac, found);
            let (word, detail) = if let Some(f) = f {
                (RowWord::Home, first_ip(f))
            } else if let Some((h, p, via)) = hosts.first() {
                let hp = host_port(&label(h, 255), *p);
                let detail = match via {
                    Some(v) => format!("{hp} \u{b7} {}", label(v, LABEL_LIMIT)),
                    None => hp,
                };
                (RowWord::Remote, detail)
            } else {
                (RowWord::NotFound, String::new())
            };
            MacRow {
                key: RowKey::Saved(mac.mac_id.clone()),
                name: label(&mac.name, LABEL_LIMIT),
                word,
                detail,
                saved: Some(mac.clone()),
                found: f.cloned(),
                has_remote: !hosts.is_empty(),
            }
        })
        .collect();
    mine.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.key_text().cmp(b.key_text()))
    });

    let mut other: Vec<(&FoundMac, MacRow)> = found
        .iter()
        .filter(|f| recognize(f, saved).is_none())
        .map(|f| {
            let word = match f.door {
                HomeDoor::PairingRequired | HomeDoor::Open => RowWord::NotPaired,
                HomeDoor::Plain => {
                    if prefs.allow_plain {
                        RowWord::Unencrypted
                    } else {
                        RowWord::UpdateSill
                    }
                }
            };
            (
                f,
                MacRow {
                    key: RowKey::Found(f.fullname.clone()),
                    name: label(&f.name, LABEL_LIMIT),
                    word,
                    detail: first_ip(f),
                    saved: None,
                    found: Some(f.clone()),
                    has_remote: false,
                },
            )
        })
        .collect();
    other.sort_by(|a, b| {
        a.1.name
            .cmp(&b.1.name)
            .then_with(|| a.0.fullname.cmp(&b.0.fullname))
    });
    (mine, other.into_iter().map(|(_, r)| r).collect())
}

impl MacRow {
    fn key_text(&self) -> &str {
        match &self.key {
            RowKey::Saved(s) | RowKey::Found(s) => s,
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use transom_net::store::SavedAddress;
    use transom_proto::identity::b64url_encode;
    use transom_proto::pairing::recognition_tag;

    pub fn saved(id: &str, name: &str, key_byte: u8) -> SavedMac {
        SavedMac {
            mac_id: id.into(),
            name: name.into(),
            fingerprint: b64url_encode(&[7u8; 32]),
            recognition_key: b64url_encode(&[key_byte; 32]),
            remote_port: None,
            addresses: Vec::new(),
            issued_at: None,
            last_address: None,
        }
    }

    pub fn found(name: &str, key_byte: Option<u8>, door: HomeDoor, ip: &str) -> FoundMac {
        FoundMac {
            name: name.into(),
            fullname: format!("{name}._sill._tcp.local."),
            host: format!("{name}.local."),
            port: 7000,
            addresses: vec![format!("{ip}:7000").parse().unwrap()],
            door,
            tag: key_byte.map(|b| recognition_tag(&[b; 32], [1, 2, 3, 4, 5, 6])),
        }
    }

    pub fn addr(host: &str, port: Option<u16>, kind: &str, via: &str) -> SavedAddress {
        SavedAddress {
            host: host.into(),
            port,
            kind: kind.into(),
            via: via.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::gui::model::MacPrefs;

    #[test]
    fn rows_words_for_each_kind() {
        let mut home = saved("HOME", "Alpha", 1);
        home.addresses
            .push(addr("100.1.1.1", None, "tailscale", "Tailscale"));
        let mut remote = saved("REMOTE", "Bravo", 2);
        remote
            .addresses
            .push(addr("100.2.2.2", Some(9000), "tailscale", "Tailscale"));
        let none = saved("NONE", "Charlie", 3);
        let found_list = vec![
            found("Alpha", Some(1), HomeDoor::PairingRequired, "192.168.1.5"),
            found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6"),
            found("Echo", None, HomeDoor::Plain, "192.168.1.7"),
            found("Foxtrot", None, HomeDoor::Open, "192.168.1.8"),
        ];
        let prefs = Prefs::default();
        let (mine, other) = rows(
            &[none.clone(), remote.clone(), home.clone()],
            &found_list,
            &prefs,
        );
        assert_eq!(mine.len(), 3);
        assert_eq!(mine[0].name, "Alpha");
        assert_eq!(mine[0].word, RowWord::Home);
        assert_eq!(mine[0].detail, "192.168.1.5");
        assert!(mine[0].has_remote);
        assert!(mine[0].found.is_some());
        assert_eq!(mine[1].word, RowWord::Remote);
        assert_eq!(mine[1].detail, "100.2.2.2:9000 \u{b7} Tailscale");
        assert!(mine[1].has_remote);
        assert_eq!(mine[2].word, RowWord::NotFound);
        assert_eq!(mine[2].detail, "");
        assert!(!mine[2].has_remote);
        assert_eq!(other.len(), 3);
        assert_eq!(other[0].name, "Delta");
        assert_eq!(other[0].word, RowWord::NotPaired);
        assert_eq!(other[0].detail, "192.168.1.6");
        assert!(matches!(&other[0].key, RowKey::Found(f) if f.starts_with("Delta")));
        assert_eq!(other[1].word, RowWord::UpdateSill);
        assert_eq!(other[2].word, RowWord::NotPaired);
        let prefs = Prefs {
            allow_plain: true,
            ..Prefs::default()
        };
        let (_, other) = rows(&[home], &found_list, &prefs);
        assert_eq!(other[1].word, RowWord::Unencrypted);
    }

    #[test]
    fn remote_hosts_order_and_ports() {
        let mut mac = saved("M", "Mac", 1);
        mac.addresses = vec![
            addr("192.168.1.9", None, "lan", ""),
            addr("100.9.9.9", None, "tailscale", "Tailscale"),
            addr("example.net", Some(8000), "internet", ""),
        ];
        let mut prefs = Prefs::default();
        prefs.macs.insert(
            "M".into(),
            MacPrefs {
                remote_host: Some("100.1.2.3".into()),
            },
        );
        assert_eq!(
            remote_hosts(&mac, &prefs),
            vec![
                ("100.1.2.3".to_string(), 7455, None),
                ("100.9.9.9".to_string(), 7455, Some("Tailscale".to_string())),
                ("example.net".to_string(), 8000, None),
            ]
        );
        mac.remote_port = Some(7777);
        let h = remote_hosts(&mac, &prefs);
        assert_eq!(h[0].1, 7777);
        assert_eq!(h[1].1, 7777);
        prefs.macs.get_mut("M").unwrap().remote_host = Some("[::1]:9".into());
        assert_eq!(remote_hosts(&mac, &prefs)[0], ("::1".to_string(), 9, None));
        prefs.macs.get_mut("M").unwrap().remote_host = Some("not an address".into());
        assert_eq!(remote_hosts(&mac, &prefs).len(), 2);
    }

    #[test]
    fn route_for_each_plan() {
        let mut mac = saved("M", "Mac", 1);
        mac.addresses = vec![addr("100.9.9.9", None, "tailscale", "Tailscale")];
        let prefs = Prefs::default();
        let (r, d) = route_for(&ConnectPlan::Home(mac.clone()), &prefs).unwrap();
        assert!(matches!(r, Route::SavedHome { .. }));
        assert_eq!(d, RouteDesc::Home);

        let plan = plan_remote(&mac, &prefs).unwrap();
        let (r, d) = route_for(&plan, &prefs).unwrap();
        assert_eq!(
            d,
            RouteDesc::Remote {
                via: Some("Tailscale".into())
            }
        );
        match r {
            Route::Direct {
                endpoint: Endpoint::Hosts(h),
                trust: Trust::Pinned(fp),
                remote: true,
                saved: Some(_),
                ..
            } => {
                assert_eq!(h, vec![("100.9.9.9".to_string(), 7455)]);
                assert_eq!(fp, [7u8; 32]);
            }
            other => panic!("unexpected {other:?}"),
        }
        let mut bad = mac.clone();
        bad.fingerprint = "!!".into();
        let plan = ConnectPlan::Remote {
            mac: bad,
            hosts: vec![("h".into(), 1)],
        };
        assert!(route_for(&plan, &prefs).is_none());
        assert!(plan_remote(&saved("N", "N", 2), &prefs).is_none());

        let f = found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6");
        let (r, d) = route_for(&ConnectPlan::Unpinned(f.clone()), &prefs).unwrap();
        assert_eq!(d, RouteDesc::Unpinned);
        assert!(matches!(
            r,
            Route::Direct {
                endpoint: Endpoint::Addrs(_),
                trust: Trust::AnyKey,
                saved: None,
                remote: false,
                ..
            }
        ));
        let (r, d) = route_for(&ConnectPlan::Plain(f.clone()), &prefs).unwrap();
        assert_eq!(d, RouteDesc::Plain);
        assert!(matches!(
            r,
            Route::Direct {
                trust: Trust::Plain,
                ..
            }
        ));
        assert_eq!(
            plan_mac(&ConnectPlan::Plain(f)),
            (None, "Delta".to_string())
        );
        assert_eq!(
            plan_mac(&ConnectPlan::Home(mac)),
            (Some("M".to_string()), "Mac".to_string())
        );
    }
}
