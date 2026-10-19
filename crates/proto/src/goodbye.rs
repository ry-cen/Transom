//! What to do and say when the host says goodbye. Port of GoodbyePolicy.swift:46-93.

use crate::messages::{goodbye_reason as r, Goodbye};
use crate::safetext;
use crate::version::SillVersion;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoodbyeOutcome {
    pub text: String,
    pub reconnect: bool,
    pub is_notice: bool,
}

/// The host's message: one line of at most 300 chars, or None when nothing printable is left.
fn message(g: &Goodbye) -> Option<String> {
    let t = safetext::label(g.message.as_deref().unwrap_or(""), 300);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// The session ended with `g` (None: no goodbye came). `mac` is what the status line calls the
/// Mac.
pub fn goodbye_outcome(g: Option<&Goodbye>, mac: &str) -> GoodbyeOutcome {
    let plain = |text: String, reconnect: bool| GoodbyeOutcome {
        text,
        reconnect,
        is_notice: false,
    };
    let Some(g) = g else {
        return plain(
            format!("{mac} disconnected. Transom will reconnect when it can reach it."),
            true,
        );
    };
    match g.reason.as_str() {
        r::QUIT => plain(
            format!("{mac} quit Sill. Transom reconnects when it is back."),
            true,
        ),
        r::REMOVED => plain(
            format!("{mac} removed this computer. Pair it again with `transom pair` to use it."),
            false,
        ),
        r::REMOTE_OFF => plain(format!("{mac} turned off Remote Access."), true),
        r::INTERNET_OFF => plain(
            format!(
                "{mac} stopped accepting connections from the internet. Connect through your VPN."
            ),
            true,
        ),
        r::BUSY => plain(format!("{mac} is already serving 8 devices."), true),
        r::PAIRING_REQUIRED => plain(
            format!("{mac} now asks devices to pair. Pair this computer with `transom pair`."),
            false,
        ),
        r::UPDATE => {
            let mut own = format!("Update Transom to keep using {mac}.");
            if let Some(floor) = g.minimum_version.as_deref().and_then(SillVersion::parse) {
                own.push_str(&format!(" It needs version {floor} or later."));
            }
            GoodbyeOutcome {
                text: message(g).unwrap_or(own),
                reconnect: g.reconnect == Some(true),
                is_notice: true,
            }
        }
        _ => {
            let reconnect = g.reconnect == Some(true);
            let own = if reconnect {
                format!("{mac} closed the connection. Transom will reconnect when it can.")
            } else {
                format!("{mac} closed the connection.")
            };
            GoodbyeOutcome {
                text: message(g).unwrap_or(own),
                reconnect,
                is_notice: true,
            }
        }
    }
}
