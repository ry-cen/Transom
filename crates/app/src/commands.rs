//! The small commands: identity, discover, list, forget and windows.

use crate::cli::TargetArgs;
use crate::target;
use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::runtime::Runtime;
use transom_net::catalog::fetch_catalog;
use transom_net::discovery::{browse, recognize};
use transom_net::store::Store;
use transom_proto::compat::client_hello;
use transom_proto::identity::b64url_encode;
use transom_proto::messages::StreamSource;
use transom_proto::pairing::HomeDoor;
use transom_proto::safetext;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn identity(store: &Store) -> Result<i32> {
    let id = store.identity().map_err(|e| anyhow!("{e}"))?;
    println!("fingerprint: {}", b64url_encode(&id.fingerprint));
    println!("hex: {}", hex(&id.fingerprint));
    println!("config: {}", store.dir().display());
    Ok(0)
}

fn print_table(rows: &[Vec<String>]) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .map(|r| r.get(c).map_or(0, |s| s.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        let mut line = String::new();
        for (c, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if c + 1 < row.len() {
                let pad = widths[c].saturating_sub(cell.chars().count());
                line.push_str(&" ".repeat(pad + 2));
            }
        }
        println!("{line}");
    }
}

pub fn discover(store: &Store, timeout_secs: u64) -> Result<i32> {
    let found = browse(Duration::from_secs(timeout_secs)).map_err(|e| anyhow!("{e}"))?;
    if found.is_empty() {
        println!("No Mac running Sill found (waited {timeout_secs} s).");
        return Ok(0);
    }
    let saved = store.macs().map_err(|e| anyhow!("{e}"))?;
    let mut rows = vec![vec![
        "NAME".to_string(),
        "ADDRESS".to_string(),
        "PORT".to_string(),
        "DOOR".to_string(),
        "SAVED".to_string(),
    ]];
    for f in &found {
        let door = match f.door {
            HomeDoor::PairingRequired => "pairing required",
            HomeDoor::Open => "open",
            HomeDoor::Plain => "plain",
        };
        rows.push(vec![
            f.name.clone(),
            f.addresses
                .first()
                .map_or_else(|| "-".to_string(), |a| a.ip().to_string()),
            f.port.to_string(),
            door.to_string(),
            recognize(f, &saved).map_or_else(|| "-".to_string(), |m| m.mac_id.clone()),
        ]);
    }
    print_table(&rows);
    Ok(0)
}

pub fn list(store: &Store) -> Result<i32> {
    let macs = store.macs().map_err(|e| anyhow!("{e}"))?;
    if macs.is_empty() {
        println!("No saved Macs. Pair one with: transom pair");
        return Ok(0);
    }
    let mut rows = vec![vec![
        "NAME".to_string(),
        "MAC ID".to_string(),
        "FINGERPRINT".to_string(),
        "LAST ADDRESS".to_string(),
    ]];
    for m in &macs {
        rows.push(vec![
            m.name.clone(),
            m.mac_id.clone(),
            m.fingerprint.chars().take(10).collect(),
            m.last_address.clone().unwrap_or_else(|| "-".to_string()),
        ]);
    }
    print_table(&rows);
    Ok(0)
}

pub fn forget(store: &Store, query: &str) -> Result<i32> {
    match store.forget(query).map_err(|e| anyhow!("{e}"))? {
        Some(m) => {
            println!("Forgot {}.", m.name);
            Ok(0)
        }
        None => {
            println!("No saved Mac matches {query}.");
            Ok(1)
        }
    }
}

pub fn windows(
    target_args: &TargetArgs,
    hello_version: &str,
    name: &str,
    store: &Store,
    rt: &Runtime,
) -> Result<i32> {
    let route = target::resolve(target_args, store, rt)?;
    let id = store.identity().map_err(|e| anyhow!("{e}"))?;
    let hello = client_hello(hello_version, name);
    let result = rt.block_on(fetch_catalog(&route, &id, &hello, Duration::from_secs(10)));
    let (list, _apps) = result.map_err(|e| anyhow!("{}", crate::text::net_error(&e)))?;
    for w in &list.windows {
        let active = matches!(&list.active, StreamSource::Window { id } if *id == w.id);
        let id_text = format!("{}{}", w.id, if active { "*" } else { "" });
        println!(
            "{id_text:>8}  {}  —  {}  ({}x{})",
            safetext::label(&w.app_name, 300),
            safetext::label(&w.title, 300),
            w.width,
            w.height
        );
    }
    let desktop = matches!(list.active, StreamSource::Desktop {});
    println!("desktop{}", if desktop { "*" } else { "" });
    Ok(0)
}
