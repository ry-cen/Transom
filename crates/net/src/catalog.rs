//! A one-shot look at what a Mac is showing: its windows and apps.

use crate::identity::DeviceIdentity;
use crate::session::read_message;
use crate::supervisor::{open, Route};
use crate::tls::SESSION_ALPN;
use crate::NetError;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::time::Instant;
use transom_proto::goodbye::goodbye_outcome;
use transom_proto::messages::{decode_goodbye, from_json, to_json, AppInfo, Hello, WindowList};
use transom_proto::wire::{encode_message, host_payload_cap, kind, now_timestamp};

pub async fn fetch_catalog(
    route: &Route,
    id: &DeviceIdentity,
    hello: &Hello,
    timeout: Duration,
) -> Result<(WindowList, Vec<AppInfo>), NetError> {
    let (mut io, _peer, _fp, label) = open(route, id, SESSION_ALPN).await?;
    let msg = encode_message(kind::HELLO, now_timestamp(), false, &to_json(hello));
    io.write_all(&msg).await?;
    io.flush().await?;

    let mut list: Option<WindowList> = None;
    let mut apps: Vec<AppInfo> = Vec::new();
    let mut deadline = Instant::now() + timeout;
    loop {
        let read =
            tokio::time::timeout_at(deadline, read_message(&mut io, host_payload_cap, || {})).await;
        let (header, payload) = match read {
            Err(_) => {
                return match list {
                    Some(l) => Ok((l, apps)),
                    None => Err(NetError::Timeout),
                }
            }
            Ok(Err(e)) => {
                return match list {
                    Some(l) if matches!(e, NetError::Closed) => Ok((l, apps)),
                    _ => Err(e),
                }
            }
            Ok(Ok(m)) => m,
        };
        match header.kind {
            kind::GOODBYE => {
                let g = decode_goodbye(&payload);
                return Err(NetError::Protocol(goodbye_outcome(Some(&g), &label).text));
            }
            kind::WINDOW_LIST if list.is_none() => {
                if let Some(l) = from_json::<WindowList>(&payload) {
                    list = Some(l);
                    deadline = Instant::now() + Duration::from_secs(1);
                }
            }
            kind::APP_LIST => {
                if let Some(a) = from_json::<Vec<AppInfo>>(&payload) {
                    apps = a;
                    if let Some(l) = list {
                        return Ok((l, apps));
                    }
                }
            }
            _ => {}
        }
    }
}
