//! `transom pair`.

use crate::cli::TargetArgs;
use crate::target::{match_instance, BROWSE_TIME};
use crate::text::{net_error, pair_refusal};
use anyhow::{anyhow, bail, Result};
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::runtime::Runtime;
use transom_net::dial::{resolve, HOME_DIAL_TIMEOUT, REMOTE_DEFAULT_PORT, REMOTE_DIAL_TIMEOUT};
use transom_net::discovery::{browse, FoundMac};
use transom_net::identity::DeviceIdentity;
use transom_net::pairing::{ask, cancel, pair_with_code, pair_with_link, PairTarget, Paired};
use transom_net::store::Store;
use transom_net::tls::PinPolicy;
use transom_net::NetError;
use transom_proto::pairing::{check_code, CodeProblem, HomeDoor};
use transom_proto::pairlink::PairLink;

pub struct PairArgs<'a> {
    pub target: &'a TargetArgs,
    pub code: Option<&'a str>,
    pub url: Option<&'a str>,
    pub name: &'a str,
}

fn is_tls(f: &FoundMac) -> bool {
    matches!(f.door, HomeDoor::PairingRequired | HomeDoor::Open)
}

fn per_attempt(remote: bool) -> Duration {
    if remote {
        REMOTE_DIAL_TIMEOUT
    } else {
        HOME_DIAL_TIMEOUT
    }
}

fn other<E: std::fmt::Display>(e: E) -> anyhow::Error {
    anyhow!("{e}")
}

/// The addresses named by the target flags, or None when the flags name no Mac.
fn explicit_addrs(t: &TargetArgs, rt: &Runtime) -> Result<Option<Vec<SocketAddr>>> {
    if let Some(host) = &t.host {
        let port = match (t.port, t.remote) {
            (Some(p), _) => p,
            (None, true) => REMOTE_DEFAULT_PORT,
            (None, false) => bail!(
                "give --port (the home door's port changes at every launch; use --mac to find it by Bonjour) or --remote"
            ),
        };
        return Ok(Some(rt.block_on(resolve(host, port)).map_err(other)?));
    }
    if let Some(name) = &t.mac {
        let found = browse(BROWSE_TIME).map_err(other)?;
        let f = match_instance(&found, name)?;
        if f.door == HomeDoor::Plain {
            bail!("{} does not use pairing; connect with --plain", f.name);
        }
        return Ok(Some(f.addresses.clone()));
    }
    Ok(None)
}

fn read_line() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

fn finish(store: &Store, paired: &Paired) -> Result<i32> {
    store.save_mac(paired.to_saved()).map_err(other)?;
    println!("Paired with {} ({}).", paired.name, paired.mac_id);
    Ok(0)
}

fn print_failure(e: &NetError) -> String {
    match e {
        NetError::PairRefused {
            reason,
            tries_left,
            retry_after,
            message,
        } => pair_refusal(reason, *tries_left, *retry_after, message.as_deref()),
        other => net_error(other),
    }
}

fn pair_by_link(
    t: &TargetArgs,
    url: &str,
    name: &str,
    id: &DeviceIdentity,
    store: &Store,
    rt: &Runtime,
) -> Result<i32> {
    let link =
        PairLink::parse(url).map_err(|e| anyhow!("that pairing link is not valid: {e:?}"))?;
    let mut candidates: Vec<Vec<SocketAddr>> = Vec::new();
    if let Some(addrs) = explicit_addrs(t, rt)? {
        candidates.push(addrs);
    } else {
        for f in browse(BROWSE_TIME)
            .map_err(other)?
            .iter()
            .filter(|f| is_tls(f))
        {
            candidates.push(f.addresses.clone());
        }
        for a in &link.addresses {
            let port = a.port.unwrap_or(link.port);
            match rt.block_on(resolve(&a.host, port)) {
                Ok(addrs) => candidates.push(addrs),
                Err(e) => log::debug!("{}: {e}", a.host),
            }
        }
    }
    let attempt = per_attempt(t.remote);
    let mut last: Option<NetError> = None;
    for addrs in candidates {
        match rt.block_on(pair_with_link(id, &link, addrs, attempt, name)) {
            Ok(paired) => return finish(store, &paired),
            Err(
                e @ (NetError::PinMismatch
                | NetError::Connect(_)
                | NetError::Timeout
                | NetError::Closed),
            ) => last = Some(e),
            Err(e) => bail!("{}", print_failure(&e)),
        }
    }
    match last {
        Some(e) => bail!("could not pair using the link: {}", print_failure(&e)),
        None => bail!("could not find the Mac from the link"),
    }
}

fn pair_by_code(
    t: &TargetArgs,
    code: &str,
    name: &str,
    id: &DeviceIdentity,
    store: &Store,
    rt: &Runtime,
) -> Result<i32> {
    let code = match check_code(code) {
        Ok(c) => c,
        Err(CodeProblem::Length) => bail!("A code has 12 digits."),
        Err(CodeProblem::Typo) => bail!("That code has a typo. Check it against your Mac."),
    };
    let target = PairTarget {
        addrs: single_target(t, rt)?,
        pin: PinPolicy::AnyKey,
        per_attempt: per_attempt(t.remote),
    };
    match rt.block_on(pair_with_code(id, &target, &code, name)) {
        Ok(paired) => finish(store, &paired),
        Err(e) => bail!("{}", print_failure(&e)),
    }
}

/// The one Mac to pair with: from the flags, or the single TLS door on the network.
fn single_target(t: &TargetArgs, rt: &Runtime) -> Result<Vec<SocketAddr>> {
    if let Some(addrs) = explicit_addrs(t, rt)? {
        return Ok(addrs);
    }
    let found = browse(BROWSE_TIME).map_err(other)?;
    let doors: Vec<&FoundMac> = found.iter().filter(|f| is_tls(f)).collect();
    match doors.as_slice() {
        [one] => Ok(one.addresses.clone()),
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

fn pair_interactive(
    t: &TargetArgs,
    name: &str,
    id: &DeviceIdentity,
    store: &Store,
    rt: &Runtime,
) -> Result<i32> {
    let addrs = single_target(t, rt)?;
    let attempt = per_attempt(t.remote);
    let mut pin = PinPolicy::AnyKey;
    let mut shown = false;
    if !t.remote {
        let target = PairTarget {
            addrs: addrs.clone(),
            pin: PinPolicy::AnyKey,
            per_attempt: attempt,
        };
        let (result, fp) = rt
            .block_on(ask(id, &target, name))
            .map_err(|e| anyhow!("{}", print_failure(&e)))?;
        if result.ok {
            println!("unexpected answer");
            return Ok(1);
        }
        let reason = result.reason.clone().unwrap_or_default();
        println!(
            "{}",
            pair_refusal(
                &reason,
                result.tries_left,
                result.retry_after,
                result.message.as_deref()
            )
        );
        match reason.as_str() {
            "shown" => shown = true,
            "openOnMac" => {}
            _ => return Ok(1),
        }
        pin = PinPolicy::Pinned(fp);
    }
    let target = PairTarget {
        addrs,
        pin,
        per_attempt: attempt,
    };
    loop {
        print!("Code (12 digits): ");
        let _ = std::io::stdout().flush();
        let line = read_line().unwrap_or_default();
        if line.is_empty() {
            if shown {
                let _ = rt.block_on(cancel(id, &target, name));
            }
            println!("cancelled");
            return Ok(1);
        }
        let code = match check_code(&line) {
            Ok(c) => c,
            Err(CodeProblem::Length) => {
                println!("A code has 12 digits.");
                continue;
            }
            Err(CodeProblem::Typo) => {
                println!("That code has a typo. Check it against your Mac.");
                continue;
            }
        };
        match rt.block_on(pair_with_code(id, &target, &code, name)) {
            Ok(paired) => return finish(store, &paired),
            Err(e)
                if matches!(
                    &e,
                    NetError::PairRefused { reason, tries_left: Some(n), .. }
                        if reason == "code" && *n > 0
                ) =>
            {
                println!("{}", print_failure(&e))
            }
            Err(e) => {
                println!("{}", print_failure(&e));
                return Ok(1);
            }
        }
    }
}

pub fn run(args: PairArgs<'_>, store: &Store, rt: &Runtime) -> Result<i32> {
    let id = store.identity().map_err(other)?;
    if let Some(url) = args.url {
        return pair_by_link(args.target, url, args.name, &id, store, rt);
    }
    if let Some(code) = args.code {
        return pair_by_code(args.target, code, args.name, &id, store, rt);
    }
    pair_interactive(args.target, args.name, &id, store, rt)
}
