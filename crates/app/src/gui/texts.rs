//! Every user-visible string of the graphical app, one id per string (docs/ui-spec.md section
//! 5; ids marked "brief" are additions of the Phase 1 brief, to be merged into the spec). The
//! CLI's texts stay in `crate::text`.
//!
//! Rules: "…" is U+2026 and apostrophes are curly (’). Every Mac-supplied value (names, titles,
//! messages, reasons) is passed through `safetext::label` here, so callers pass raw strings.

use crate::gui::model::{PairFailure, RouteDesc, RowWord};
use transom_net::session::EndKind;
use transom_proto::address::AddressError;
use transom_proto::host_settings::LinkReport;
use transom_proto::messages::goodbye_reason as gr;
use transom_proto::safetext::{label, LABEL_LIMIT};
use transom_proto::version::SillVersion;

/// Opened by "Get Sill" (LIST9) and the update banner.
pub const GET_SILL_URL: &str = "https://getsill.app/download";
/// Opened by the About dialog's first link (DLG8).
pub const SOURCE_URL: &str = "https://github.com/ry-cen/sill_port";
/// Opened by the About dialog's second link (DLG8).
pub const SILL_URL: &str = "https://getsill.app";

/// A Mac's name as the copy shows it: safe-text filtered, "Mac" when nothing is left.
pub fn mac_name(raw: &str) -> String {
    let t = label(raw, LABEL_LIMIT);
    if t.is_empty() {
        "Mac".to_string()
    } else {
        t
    }
}

fn whole_secs(secs: Option<f64>) -> u64 {
    secs.filter(|s| s.is_finite())
        .unwrap_or(0.0)
        .max(0.0)
        .ceil() as u64
}

// ---- Window ------------------------------------------------------------------------------

/// LIST26 (brief): the window title outside streaming.
pub const WINDOW_TITLE: &str = "Transom";

/// LIST27 (brief): the window title while streaming.
pub fn window_title_streaming(mac: &str) -> String {
    format!("{} — Transom", mac_name(mac))
}

// ---- 5.1 Macs screen (LIST) and Mac details (DET) ----------------------------------------

/// LIST1: the welcome line.
pub const WELCOME: &str = "Transom shows a Mac’s screen here and sends your mouse and keyboard to it. Your Mac needs the free Sill app.";
/// LIST2: status until the first Mac is found.
pub const LOOKING: &str = "Looking for Macs on this network…";
/// LIST3: after 5 s with no Mac found.
pub const NO_MAC_HINT: &str = "No Mac yet? Open Sill on your Mac, on the same network as this computer. Away from home, use Add a Mac… with its address and a code.";
/// LIST4: the screen title.
pub const MACS_TITLE: &str = "Connect to a Mac";
/// LIST5: section heading.
pub const MY_MACS: &str = "MY MACS";
/// LIST6: section heading.
pub const OTHER_MACS: &str = "OTHER MACS ON THIS NETWORK";
/// LIST7: the last row.
pub const ADD_A_MAC: &str = "Add a Mac…";
/// LIST8: footer.
pub const NEEDS_SILL: &str = "Needs the free Sill app on your Mac.";
/// LIST9: footer link and the update banner's button; opens `GET_SILL_URL`.
pub const GET_SILL: &str = "Get Sill";
/// LIST10: footer link; opens the About dialog.
pub const ABOUT_TRANSOM: &str = "About Transom";
/// LIST11: top-right button, and the Preferences dialog's title.
pub const PREFERENCES: &str = "Preferences";
/// LIST12: row word.
pub const WORD_HOME: &str = "Home";
/// LIST13: row word.
pub const WORD_REMOTE: &str = "Remote";
/// LIST14: row word.
pub const WORD_NOT_FOUND: &str = "Not found";
/// LIST15: row word.
pub const WORD_NOT_PAIRED: &str = "Not paired";
/// LIST16: row word (plain door, unencrypted connections off).
pub const WORD_UPDATE_SILL: &str = "Update Sill";
/// LIST17: row word (plain door, unencrypted connections on).
pub const WORD_UNENCRYPTED: &str = "Unencrypted";
/// LIST18: row menu.
pub const MENU_CONNECT: &str = "Connect";
/// LIST19: row menu, only when found.
pub const MENU_CONNECT_HOME: &str = "Connect at Home";
/// LIST20: row menu, only with a remote address.
pub const MENU_CONNECT_REMOTE: &str = "Connect Remotely";
/// LIST21: row menu.
pub const MENU_DETAILS: &str = "Details…";

/// LIST22: row menu and the details dialog.
pub fn menu_forget(mac: &str) -> String {
    format!("Forget {}…", mac_name(mac))
}

/// LIST23: the open-door card.
pub const CONNECT_WITHOUT_PAIRING: &str = "Connect without pairing";

/// LIST24: status line after forgetting.
pub fn forgot(mac: &str) -> String {
    format!("Forgot {}.", mac_name(mac))
}

/// LIST25 (brief): accessible label of a row's "⋯" button.
pub fn row_menu_label(mac: &str) -> String {
    format!("More for {}", mac_name(mac))
}

/// The row word (LIST12-LIST17).
pub fn row_word(w: &RowWord) -> &'static str {
    match w {
        RowWord::Home => WORD_HOME,
        RowWord::Remote => WORD_REMOTE,
        RowWord::NotFound => WORD_NOT_FOUND,
        RowWord::NotPaired => WORD_NOT_PAIRED,
        RowWord::UpdateSill => WORD_UPDATE_SILL,
        RowWord::Unencrypted => WORD_UNENCRYPTED,
    }
}

/// DET1: the remote address field's hint line.
pub fn remote_address_hint(mac: &str) -> String {
    let m = mac_name(mac);
    format!("Away from home, Transom reaches {m} at this address (for example its Tailscale address). Leave it empty to use the addresses {m} reports.")
}

/// DET2.
pub const REPORTED_ADDRESSES: &str = "Addresses the Mac reported";
/// DET3: no reported addresses.
pub const NO_REPORTED_ADDRESSES: &str =
    "None yet. They arrive while connected, if Remote Access is on.";
/// DET4.
pub const REMOTE_ADDRESS: &str = "Remote address";
/// DET5: label.
pub const NAME: &str = "Name";
/// DET6: label.
pub const MAC_ID: &str = "Mac ID";
/// DET7: label.
pub const KEY_FINGERPRINT: &str = "Key fingerprint";

/// DET8 (brief): one reported address, `host:port · via · kind`, safe-text filtered.
pub fn reported_address(host: &str, port: Option<u16>, via: &str, kind: &str) -> String {
    let host = label(host, 255);
    let hp = match port {
        Some(p) => format!("{host}:{p}"),
        None => host,
    };
    format!(
        "{hp} · {} · {}",
        label(via, LABEL_LIMIT),
        label(kind, LABEL_LIMIT)
    )
}

// ---- 5.2 Pairing at home (PAIR) ----------------------------------------------------------

/// PAIR1: the pairing card's title.
pub fn pair_title(mac: &str) -> String {
    format!("Pair with {}", mac_name(mac))
}

/// PAIR2: state Asking.
pub fn asking(mac: &str) -> String {
    format!("Asking {} for a code…", mac_name(mac))
}

/// PAIR3: state Code, shown by the Mac.
pub fn code_shown(mac: &str) -> String {
    format!("{} is showing a code. Type it here.", mac_name(mac))
}

/// PAIR4: the ask was answered "openOnMac".
pub fn code_not_shown(mac: &str) -> String {
    format!("{} didn’t show a code by itself. On the Mac, choose Pair iPhone or iPad… in the Sill menu, then type the code here.", mac_name(mac))
}

/// PAIR5: a code of the wrong length.
pub const CODE_LENGTH: &str = "A code has 12 digits.";
/// PAIR6: the check digit fails.
pub const CODE_TYPO: &str = "That code has a typo. Check it against your Mac.";

/// PAIR7: state Working.
pub fn pairing_with(mac: &str) -> String {
    format!("Pairing with {}…", mac_name(mac))
}

/// PAIR8: pairing succeeded.
pub fn paired_with(mac: &str) -> String {
    format!("Paired with {}.", mac_name(mac))
}

/// PAIR9: refusal "code" (same text as the CLI's). Without a count: "Wrong code.".
pub fn wrong_code(tries_left: Option<i64>) -> String {
    match tries_left {
        Some(n) => format!("Wrong code. {n} tries left."),
        None => "Wrong code.".to_string(),
    }
}

/// PAIR10: refusal "closed", also the remote door with no open pairing window.
pub fn pairing_closed(mac: &str) -> String {
    format!("{}’s pairing window is closed. On the Mac, choose Pair iPhone or iPad… in the Sill menu, then try again.", mac_name(mac))
}

/// PAIR11: refusal "expired".
pub const CODE_EXPIRED: &str = "That code expired. Pair again to get a new one.";

/// PAIR12: refusal "stopped".
pub fn pairing_stopped(mac: &str) -> String {
    format!("{} stopped pairing after too many wrong codes. On the Mac, choose Pair iPhone or iPad… in the Sill menu, then try again.", mac_name(mac))
}

/// PAIR13: refusal "busy".
pub fn pairing_busy(mac: &str, retry_after: Option<f64>) -> String {
    format!(
        "{} is busy. Try again in {} s.",
        mac_name(mac),
        whole_secs(retry_after)
    )
}

/// PAIR14: refusal "locked".
pub fn pairing_locked(mac: &str) -> String {
    format!("Unlock {}, then try again.", mac_name(mac))
}

/// PAIR15: the Mac's proof of the code was bad.
pub fn bad_proof(mac: &str) -> String {
    format!(
        "Pairing didn’t finish: {} couldn’t show it knows the code.",
        mac_name(mac)
    )
}

/// PAIR16: timeout, connect failure or closed connection while pairing.
pub fn no_answer(mac: &str) -> String {
    format!(
        "{} didn’t answer. Check that Sill is open on it, then try again.",
        mac_name(mac)
    )
}

/// PAIR17: an unknown refusal, with the Mac's message, else with its reason.
pub fn refused_other(mac: &str, reason: &str, message: Option<&str>) -> String {
    let msg = message.map(|m| label(m, 300)).unwrap_or_default();
    if msg.is_empty() {
        format!(
            "{} refused to pair ({}).",
            mac_name(mac),
            label(reason, LABEL_LIMIT)
        )
    } else {
        format!("{} said: {msg}", mac_name(mac))
    }
}

/// PAIR18 (brief): the code field's placeholder.
pub const CODE_PLACEHOLDER: &str = "0000 0000 0000";

/// The inline text for a failed ask or pairing (PAIR9-PAIR17).
pub fn pair_failure(mac: &str, f: &PairFailure) -> String {
    match f {
        PairFailure::Refused {
            reason,
            tries_left,
            retry_after,
            message,
        } => match reason.as_str() {
            "code" => wrong_code(*tries_left),
            "closed" => pairing_closed(mac),
            "expired" => CODE_EXPIRED.to_string(),
            "stopped" => pairing_stopped(mac),
            "busy" => pairing_busy(mac, *retry_after),
            "locked" => pairing_locked(mac),
            other => refused_other(mac, other, message.as_deref()),
        },
        PairFailure::BadProof => bad_proof(mac),
        PairFailure::Net(_) => no_answer(mac),
    }
}

// ---- 5.3 Add a Mac (ADD) -----------------------------------------------------------------

/// ADD1: card title.
pub const ADD_TITLE: &str = "Add a Mac";
/// ADD2.
pub const ADD_INTRO: &str =
    "Pair once, and Transom reaches your Mac from anywhere, through your VPN or the internet.";
/// ADD3.
pub const ADD_HOW: &str =
    "On your Mac, choose Pair iPhone or iPad… in the Sill menu. Type the address and code it shows.";
/// ADD4: field label.
pub const ADDRESS: &str = "Address";
/// ADD5: field label (also on the pairing card).
pub const CODE: &str = "Code";
/// ADD6: the address placeholder.
pub const ADDRESS_PLACEHOLDER: &str = "100.101.102.103 or mac.example.net";
/// ADD7.
pub const USE_LINK: &str = "Use a Pairing Link Instead";
/// ADD8.
pub const USE_ADDRESS: &str = "Use an Address and Code Instead";
/// ADD9: the link field's placeholder.
pub const LINK_PLACEHOLDER: &str = "sill://pair?…";

/// ADD10: the link confirmation.
pub fn confirm_link(link_name: &str, first_address: &str) -> String {
    format!(
        "Pair with {} at {}? Only pair with a link your own Mac shows.",
        mac_name(link_name),
        label(first_address, 255)
    )
}

/// ADD11: a link that does not parse.
pub const NOT_A_LINK: &str = "That isn’t a Sill pairing link. It starts with sill://pair.";

/// ADD12 (brief): an address the parser refuses.
pub fn bad_address(e: &AddressError) -> String {
    format!("Check the address: {e}.")
}

/// ADD13 (brief): the link field's label.
pub const PAIRING_LINK: &str = "Pairing link";

// ---- 5.4 Doors and confirmations (DOOR, DLG) ---------------------------------------------

/// DOOR1: the open-door warning.
pub fn open_door_warning(mac: &str) -> String {
    let m = mac_name(mac);
    format!("{m} lets any device connect without pairing. Transom can’t check that it is really {m}. Pair it to be sure.")
}

/// DOOR2: tooltip on a plain-door row while unencrypted connections are off.
pub fn sill_too_old(mac: &str) -> String {
    format!(
        "{}’s Sill is too old for Transom. Update Sill on the Mac.",
        mac_name(mac)
    )
}

/// DOOR3: confirm for a plain door.
pub fn confirm_plain(mac: &str) -> String {
    format!("{} uses an unencrypted connection. Anyone on this network can see the stream and your typing. Connect anyway?", mac_name(mac))
}

/// DOOR4: Preferences › Advanced.
pub const PLAIN_WARNING: &str = "Only for test builds of Sill. Unencrypted connections can be watched by anyone on the network.";

/// DLG1: forget confirm.
pub fn confirm_forget(mac: &str) -> String {
    let m = mac_name(mac);
    format!("Forget {m}? Transom removes its key. To use it again, pair again. {m} still lists this computer until you remove it in Sill’s Devices.")
}

/// DLG2: confirm after a key change.
pub fn confirm_forget_and_pair(mac: &str) -> String {
    format!("Forget {} and pair again? Only do this if you know why its key changed, for example Sill was reinstalled.", mac_name(mac))
}

/// DLG3: the destructive button of DLG1 and DLG2.
pub const FORGET: &str = "Forget";
/// DLG4: the confirm button of DOOR3.
pub const CONNECT: &str = "Connect";

/// DLG5: About dialog.
pub fn about_version() -> String {
    format!("Transom {}", env!("CARGO_PKG_VERSION"))
}

/// DLG6: About dialog.
pub const ABOUT_INDEPENDENT: &str =
    "An independent client for Sill. Not part of Sill and not endorsed by its author.";
/// DLG7: About dialog (licence and the credit line of NOTICE).
pub const ABOUT_LICENSE: &str = "Apache License 2.0. A client for Sill (github.com/Saffsanity/sill), © 2026 Noah Saffer, licensed under the Apache License 2.0.";
/// DLG8: About dialog, first link (opens `SOURCE_URL`).
pub const ABOUT_LINK_SOURCE: &str = "github.com/ry-cen/sill_port";
/// DLG8: About dialog, second link (opens `SILL_URL`).
pub const ABOUT_LINK_SILL: &str = "getsill.app";

// ---- 5.5 Connection states (CONN) --------------------------------------------------------

/// CONN1: the connecting card, before the first status.
pub fn connecting_to(mac: &str) -> String {
    format!("Connecting to {}…", mac_name(mac))
}

/// CONN2: the waiting card, after 1.5 s.
pub const WAITING_FOR_PICTURE: &str =
    "Waiting for the first picture… The Mac sends a full picture about every 4 seconds.";

/// CONN3: the reconnecting card's countdown.
pub fn trying_again_in(secs: u64) -> String {
    format!("Trying again in {secs} s")
}

// ---- 5.6 Bar (BAR) and Apps (APPS) -------------------------------------------------------

/// BAR1: bar button and its accessible label (Phase 2).
pub const APPS: &str = "Apps";
/// BAR2: bar button.
pub const DESKTOP: &str = "Desktop";
/// BAR3: the Desktop button's accessible label.
pub const SHOW_DESKTOP: &str = "Show the desktop";

fn rtt_text(rtt_ms: Option<f64>) -> String {
    rtt_ms
        .filter(|r| r.is_finite())
        .map_or_else(|| "–".to_string(), |r| format!("{r:.0}"))
}

/// BAR4: the stats pill.
pub fn stats_pill(rtt_ms: Option<f64>, fps: u32) -> String {
    format!("{} ms · {fps} fps", rtt_text(rtt_ms))
}

/// BAR5: the stats tooltip.
pub fn stats_tooltip(rtt_ms: Option<f64>, fps: u32, route: &RouteDesc) -> String {
    let how = match route {
        RouteDesc::Home => "connected at home",
        RouteDesc::Remote { .. } => "connected remotely",
        RouteDesc::Unpinned | RouteDesc::Plain => "connected without a pinned key",
    };
    format!(
        "Round trip {} ms, {fps} frames per second, {how}",
        rtt_text(rtt_ms)
    )
}

fn mbps(bits_per_second: i64) -> String {
    let m = bits_per_second as f64 / 1_000_000.0;
    if (m - m.round()).abs() < 0.05 {
        format!("{m:.0} Mbps")
    } else {
        format!("{m:.1} Mbps")
    }
}

fn scale_name(scale: f64) -> &'static str {
    if scale >= 2.0 {
        "Retina"
    } else {
        "Standard"
    }
}

/// BAR25: the chip shown while the Mac reports the link behind (`LinkReport.isBehind`).
pub fn link_chip(withheld_per_second: i64) -> String {
    format!("Network behind · {withheld_per_second} dropped/s")
}

/// BAR26: the chip's tooltip: what the Mac withholds, what the link carried, and Sill's
/// suggestion.
pub fn link_tooltip(link: &LinkReport) -> String {
    let mut t = format!(
        "The Mac is dropping {} frames a second: the network to this computer cannot carry {} \
         per 60 fps.",
        link.withheld_per_second,
        mbps(link.bitrate)
    );
    if let Some(k) = link.carried_kbps {
        t.push_str(&format!(" It carried {}.", mbps(k.saturating_mul(1000))));
    }
    match (link.suggested_bitrate, link.suggested_capture_scale) {
        (Some(b), Some(c)) => t.push_str(&format!(
            " Sill suggests {} at {} resolution.",
            mbps(b),
            scale_name(c)
        )),
        (Some(b), None) => t.push_str(&format!(" Sill suggests {}.", mbps(b))),
        (None, Some(c)) => t.push_str(&format!(
            " Sill suggests {} resolution at the same bitrate.",
            scale_name(c)
        )),
        (None, None) => t.push_str(" Sill has no lower quality to suggest."),
    }
    t.push_str(" Change it in Mac settings, or use a wired connection.");
    t
}

/// BAR6: the gear's accessible label.
pub const MAC_SETTINGS: &str = "Mac settings";
/// BAR7: button and accessible label.
pub const FULL_SCREEN: &str = "Full screen";
/// BAR8: the same button in full screen.
pub const LEAVE_FULL_SCREEN: &str = "Leave full screen";

/// BAR9: the Disconnect button's accessible label; the settings panel's footer.
pub fn disconnect_from(mac: &str) -> String {
    format!("Disconnect from {}", mac_name(mac))
}

/// BAR10: a tile's tooltip and accessible label; BAR11 " (showing)" is added to the active one.
pub fn tile_label(app: &str, title: &str, active: bool) -> String {
    let app = label(app, LABEL_LIMIT);
    let title = label(title, 120);
    let base = if title.is_empty() {
        app
    } else {
        format!("{app} — {title}")
    };
    if active {
        format!("{base}{SHOWING}")
    } else {
        base
    }
}

/// BAR11: added to the active tile's accessible label.
pub const SHOWING: &str = " (showing)";

/// BAR12: toast at the first window pick in a session.
pub fn everyone_sees(mac: &str) -> String {
    format!("Everyone watching {} now sees this window.", mac_name(mac))
}

/// BAR13: thumbnail menu (Phase 2).
pub const WINDOW_SHOW: &str = "Show";
/// BAR14: thumbnail menu (Phase 2).
pub const WINDOW_MINIMIZE: &str = "Minimize on the Mac";
/// BAR15: thumbnail menu (Phase 2).
pub const WINDOW_FULL_SCREEN: &str = "Full Screen on the Mac";
/// BAR16: thumbnail menu (Phase 2).
pub const WINDOW_CLOSE: &str = "Close on the Mac";
/// BAR17: chip in the bar.
pub const VIEW_ONLY: &str = "View only";
/// BAR18: the chip's tooltip.
pub const VIEW_ONLY_WHY: &str = "View only: this Mac is the computer you are using. Sending its own input back to it would loop.";
/// BAR19: the link notice's button (Phase 2).
pub const LOWER_QUALITY: &str = "Lower Quality";

/// BAR20: the link notice (Phase 2).
pub fn link_behind(mac: &str) -> String {
    format!(
        "The connection can’t keep up, so {} is skipping frames.",
        mac_name(mac)
    )
}

/// BAR21 (brief): the handle's accessible label while the bar is not pinned.
pub const SHOW_BAR: &str = "Show the bar";
/// BAR22 (brief): the handle's accessible label while the bar is pinned.
pub const HIDE_BAR: &str = "Hide the bar";
/// BAR23 (brief): the link in the View only tooltip; opens Preferences › Input.
pub const INPUT_PREFERENCES: &str = "Input preferences…";

/// APPS1: search placeholder (Phase 2).
pub fn search_apps(mac: &str) -> String {
    format!("Search apps on {}", mac_name(mac))
}

/// APPS2: section heading (Phase 2).
pub const OPEN_NOW: &str = "Open Now";
/// APPS3: section heading (Phase 2).
pub const ALL_APPS: &str = "All Apps";

/// APPS4 / APPS5: a row's trailing word (Phase 2).
pub fn window_count(n: usize) -> String {
    if n == 1 {
        "1 window".to_string()
    } else {
        format!("{n} windows")
    }
}

/// APPS6: toast after a launch (Phase 2).
pub fn opening_app(app: &str, mac: &str) -> String {
    format!("Opening {} on {}…", label(app, LABEL_LIMIT), mac_name(mac))
}

// ---- 5.7 Settings panel (SET) and Preferences (PREF) -------------------------------------

/// SET1: the panel header, connected at home.
pub fn connected_at_home(address: &str) -> String {
    format!("Connected at home · {}", label(address, 255))
}

/// SET2: the panel header, connected remotely.
pub fn connected_remotely(address: &str, via: Option<&str>) -> String {
    match via.map(|v| label(v, LABEL_LIMIT)).filter(|v| !v.is_empty()) {
        Some(v) => format!("Connected remotely · {} ({v})", label(address, 255)),
        None => format!("Connected remotely · {}", label(address, 255)),
    }
}

/// SET3: the panel header without a pinned key.
pub const CONNECTED_UNPINNED: &str = "Connected without a pinned key";
/// SET4 (Phase 2).
pub const QUALITY: &str = "Quality";

/// SET5 / SET6: the Quality combo's title (Phase 2).
pub fn quality_title(bps: i64) -> String {
    transom_proto::host_settings::bitrate_title(bps)
}

/// SET7 (Phase 2).
pub const RESOLUTION: &str = "Resolution";
/// SET8.
pub const RETINA: &str = "Retina";
/// SET9.
pub const STANDARD: &str = "Standard";
/// SET10 (Phase 2).
pub const FRAME_RATE: &str = "Frame Rate";
/// SET11.
pub const FPS_60: &str = "60 fps";
/// SET12.
pub const FPS_120: &str = "120 fps";
/// SET13.
pub const PRIORITIZE_SPEED: &str = "Prioritize Encoding Speed";

/// SET14: under SET13.
pub fn prioritize_speed_note(mac: &str) -> String {
    format!(
        "Lower latency when {} is busy, at a softer picture.",
        mac_name(mac)
    )
}

/// SET15.
pub const VIRTUAL_DISPLAY: &str = "Virtual Display";

/// SET16: under SET15 when the Mac gives no note.
pub fn virtual_display_note(mac: &str) -> String {
    format!("The window you pick moves onto an invisible display on {} while it streams, so it keeps updating when covered.", mac_name(mac))
}

/// SET17: under the Streaming group.
pub fn applies_to_every_device(mac: &str) -> String {
    format!(
        "Applies to every device streaming from {}. The stream restarts for a moment.",
        mac_name(mac)
    )
}

/// SET18: toast after a refused pick.
pub fn kept_setting(mac: &str) -> String {
    format!("{} kept its setting.", mac_name(mac))
}

/// SET19: read-only line.
pub fn now_streaming(width: i64, height: i64, fps: i64, mbps: i64) -> String {
    format!("Now streaming {width}×{height} · {fps} fps · {mbps} Mbps")
}

/// SET20.
pub const NOT_STREAMING: &str = "Not streaming";

/// SET21: the Mac reports the software encoder.
pub fn software_encoder(mac: &str) -> String {
    format!("{}’s hardware encoder isn’t answering, so it streams with the software encoder, up to 60 fps at Standard.", mac_name(mac))
}

/// SET22: settings are not persistent.
pub const NOT_PERSISTENT: &str = "SillHost keeps these until it quits.";

/// SET23: this connection is away.
pub fn counts_as_away(mac: &str) -> String {
    format!("This connection counts as away from home: Quality and Resolution here set {}’s away quality.", mac_name(mac))
}

/// SET24.
pub const LOADING_SETTINGS: &str = "Loading settings…";

/// SET25: no settings within 2 s of connecting.
pub fn update_sill_for_settings(mac: &str) -> String {
    format!(
        "Update Sill on {} to change these from here.",
        mac_name(mac)
    )
}

/// SET26.
pub const DIRECT_WIRELESS: &str = "Direct Wireless Connection";

/// SET27: under SET26. "Wi-Fi" has an ASCII hyphen: the spec’s U+2011 has no glyph in egui’s
/// default fonts.
pub fn direct_wireless_note(mac: &str) -> String {
    format!(
        "Lets iPhones and iPads reach {} without a shared Wi-Fi network. Transom doesn’t use it.",
        mac_name(mac)
    )
}

/// SET28: panel switch.
pub const SEND_POINTER: &str = "Send Pointer";
/// SET29: panel switch.
pub const SEND_KEYBOARD: &str = "Send Keyboard";
/// SET30 (Phase 2).
pub const TEXT_SIZE: &str = "Text Size";
/// SET31: Text Size choice.
pub const TEXT_SIZE_OFF: &str = "Off";
/// SET32: panel row label.
pub const KEYMAP: &str = "Keymap";
/// SET33 (brief): section D's heading.
pub const THIS_COMPUTER: &str = "This Computer";

/// PREF1: General.
pub const DEVICE_NAME: &str = "Name on your Macs";
/// PREF2: under PREF1.
pub const DEVICE_NAME_NOTE: &str = "Macs you already paired keep showing the old name.";
/// PREF3: General. (The spec's straight apostrophe is made curly, per section 5's rule.)
pub const THIS_COMPUTERS_KEY: &str = "This computer’s key";
/// PREF4: General.
pub const CONFIG_FOLDER: &str = "Config folder";
/// PREF5: tab.
pub const TAB_GENERAL: &str = "General";
/// PREF6: tab.
pub const TAB_INPUT: &str = "Input";
/// PREF7: tab.
pub const TAB_STREAMING: &str = "Streaming";
/// PREF8: tab.
pub const TAB_ADVANCED: &str = "Advanced";
/// PREF9: radio, also in the settings panel.
pub const KEYMAP_PC: &str = "PC layout: Ctrl acts as Command";
/// PREF10: radio, also in the settings panel.
pub const KEYMAP_MAC: &str = "Mac layout: Command is Command, Ctrl is Control";
/// PREF11: Input.
pub const WHEN_LOCAL: &str = "When the Mac is this computer";
/// PREF12: Input.
pub const SEND_THE_POINTER: &str = "Send the pointer";
/// PREF13: Input.
pub const SEND_THE_KEYBOARD: &str = "Send the keyboard";
/// PREF14: next to Send the keyboard / Send Keyboard.
pub const LOCAL_KEYBOARD_WARNING: &str =
    "Keys you type would reach this Mac twice. Don’t type into the Transom window while this is on.";
/// PREF15: Streaming.
pub const FPS_TO_ASK: &str = "Frame rate to ask for";
/// PREF16: Streaming.
pub const SHOW_STATS: &str = "Show round trip and frame rate in the bar";
/// PREF17: Streaming (Phase 2).
pub const KEEP_BAR_OPEN: &str = "Keep the bar open";
/// PREF18: Advanced.
pub const ALLOW_PLAIN: &str = "Allow unencrypted connections";
/// PREF19: Advanced.
pub const HELLO_VERSION: &str = "Sill version to report";
/// PREF20: Advanced.
pub const WRITE_LOG: &str = "Write a log file";
/// PREF21: log level.
pub const LOG_INFO: &str = "Info";
/// PREF22: log level.
pub const LOG_DEBUG: &str = "Debug";
/// PREF23 (brief): under the log file settings.
pub const NEXT_LAUNCH: &str = "Takes effect the next time Transom starts.";
/// PREF24 (brief): the hex disclosure of the key fingerprint.
pub const SHOW_HEX: &str = "Hex";
/// PREF25 (brief): Input, the heading of the two layout radios.
pub const MODIFIER_LAYOUT: &str = "Modifier layout";
/// PREF26 (brief): the mapping table's header row (the README's table).
pub const KEYMAP_TABLE_HEADER: [&str; 3] = ["Key you press", "PC layout", "Mac layout"];
/// PREF27 (brief): the mapping table's rows (the README's table).
pub const KEYMAP_TABLE: [[&str; 3]; 4] = [
    ["Ctrl", "Command", "Control"],
    ["Win / Super", "Control", "Command"],
    ["Alt", "Option", "Option"],
    ["Shift", "Shift", "Shift"],
];

/// PREF28 (brief): the log file's path line.
pub fn log_path(path: &str) -> String {
    format!("Log file: {path}")
}

// ---- Shared buttons (BTN) ----------------------------------------------------------------

/// BTN1.
pub const CANCEL: &str = "Cancel";
/// BTN2.
pub const CLOSE: &str = "Close";
/// BTN3.
pub const DONE: &str = "Done";
/// BTN4.
pub const TRY_AGAIN: &str = "Try Again";
/// BTN5.
pub const TRY_NOW: &str = "Try Now";
/// BTN6.
pub const DISCONNECT: &str = "Disconnect";
/// BTN7.
pub const COPY: &str = "Copy";
/// BTN8.
pub const PAIR: &str = "Pair";
/// BTN9.
pub const SAVE: &str = "Save";
/// BTN10.
pub const CLEAR: &str = "Clear";
/// BTN11.
pub const RESET: &str = "Reset";
/// BTN12 (brief): the banner's close button, accessible label.
pub const DISMISS: &str = "Dismiss";

// ---- 5.8 Ended states and goodbye reasons (END) ------------------------------------------

/// END1: connection lost.
pub fn lost(mac: &str) -> String {
    format!(
        "{} disconnected. Transom reconnects when it can reach it.",
        mac_name(mac)
    )
}

/// END2: goodbye `quit`.
pub fn quit_sill(mac: &str) -> String {
    format!(
        "{} quit Sill. Transom reconnects when it is back.",
        mac_name(mac)
    )
}

/// END3: goodbye `busy`. The Mac sends no limit: `busy` comes only from the remote door, which
/// caps remote sessions at 8 (DoorPolicy.swift:16-17, 110), and Sill's own client says the same
/// (GoodbyePolicy.swift:67).
pub fn busy(mac: &str) -> String {
    format!("{} is already serving 8 devices.", mac_name(mac))
}

/// END4: goodbye `remoteOff`.
pub fn remote_off(mac: &str) -> String {
    format!("{} turned off Remote Access.", mac_name(mac))
}

/// END5: goodbye `internetOff`.
pub fn internet_off(mac: &str) -> String {
    format!(
        "{} stopped accepting connections from the internet. Connect through your VPN.",
        mac_name(mac)
    )
}

/// END6: goodbye `removed`.
pub fn removed(mac: &str) -> String {
    format!(
        "{} removed this computer. Pair again to use it.",
        mac_name(mac)
    )
}

/// END7: goodbye `pairingRequired`.
pub fn pairing_required(mac: &str) -> String {
    format!(
        "{} now asks devices to pair. Pair this computer to use it.",
        mac_name(mac)
    )
}

/// END8: goodbye `update` without a message; the floor is added when the Mac gives one.
pub fn update_transom(mac: &str, minimum_version: Option<&str>) -> String {
    let mut t = format!("Update Transom to keep using {}.", mac_name(mac));
    if let Some(v) = minimum_version.and_then(SillVersion::parse) {
        t.push_str(&format!(" It needs version {v} or later."));
    }
    t
}

/// END9: any other goodbye without a message.
pub fn closed_connection(mac: &str) -> String {
    format!("{} closed the connection.", mac_name(mac))
}

/// END10: not paired.
pub fn not_paired(mac: &str) -> String {
    format!(
        "{} doesn’t know this computer. Pair it again.",
        mac_name(mac)
    )
}

/// END11: key changed.
pub fn key_changed(mac: &str) -> String {
    format!("{} presented a different key from the one saved. If Sill was reinstalled or the Mac was reset, forget it and pair again. Otherwise, don’t connect.", mac_name(mac))
}

/// END12: banner button.
pub const CONNECT_AT_HOME: &str = "Connect at Home";
/// END13: banner button.
pub const PAIR_AGAIN: &str = "Pair Again";
/// END14: banner button.
pub const PAIR_BUTTON: &str = "Pair";
/// END15: banner button.
pub const FORGET_AND_PAIR: &str = "Forget and Pair Again…";

/// The text for an ended or finished session (section 5.8). Only `ConnectFailed` uses
/// `supervisor_text` (filtered, at most 300 chars); the supervisor's connect errors name no
/// commands.
pub fn end_text(kind: &EndKind, mac: &str, supervisor_text: &str) -> String {
    let own_message = |m: &Option<String>| {
        m.as_deref()
            .map(|m| label(m, 300))
            .filter(|m| !m.is_empty())
    };
    match kind {
        EndKind::ConnectFailed => {
            let t = label(supervisor_text, 300);
            if t.is_empty() {
                lost(mac)
            } else {
                t
            }
        }
        EndKind::Lost => lost(mac),
        EndKind::NotPaired => not_paired(mac),
        EndKind::PinMismatch => key_changed(mac),
        EndKind::Goodbye {
            reason,
            message,
            minimum_version,
        } => match reason.as_str() {
            gr::QUIT => quit_sill(mac),
            gr::BUSY => busy(mac),
            gr::REMOTE_OFF => remote_off(mac),
            gr::INTERNET_OFF => internet_off(mac),
            gr::REMOVED => removed(mac),
            gr::PAIRING_REQUIRED => pairing_required(mac),
            gr::UPDATE => own_message(message)
                .unwrap_or_else(|| update_transom(mac, minimum_version.as_deref())),
            _ => own_message(message).unwrap_or_else(|| closed_connection(mac)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_tooltip_reads_the_report() {
        let mut l = LinkReport {
            state: "behind".into(),
            withheld_per_second: 23,
            bitrate: 15_000_000,
            carried_kbps: Some(9_500),
            suggested_bitrate: Some(8_000_000),
            suggested_capture_scale: Some(1.0),
        };
        let t = link_tooltip(&l);
        assert!(t.contains("dropping 23 frames"), "{t}");
        assert!(t.contains("15 Mbps per 60 fps"), "{t}");
        assert!(t.contains("It carried 9.5 Mbps."), "{t}");
        assert!(t.contains("8 Mbps at Standard resolution"), "{t}");
        l.suggested_bitrate = None;
        l.suggested_capture_scale = None;
        l.carried_kbps = None;
        let t = link_tooltip(&l);
        assert!(t.contains("no lower quality"), "{t}");
        assert!(!t.contains("carried"), "{t}");
    }

    fn goodbye(reason: &str, message: Option<&str>, min: Option<&str>) -> EndKind {
        EndKind::Goodbye {
            reason: reason.to_string(),
            message: message.map(String::from),
            minimum_version: min.map(String::from),
        }
    }

    fn all_kinds() -> Vec<EndKind> {
        let mut v = vec![
            EndKind::ConnectFailed,
            EndKind::Lost,
            EndKind::NotPaired,
            EndKind::PinMismatch,
        ];
        for r in [
            gr::QUIT,
            gr::BUSY,
            gr::REMOTE_OFF,
            gr::INTERNET_OFF,
            gr::REMOVED,
            gr::PAIRING_REQUIRED,
            gr::UPDATE,
            "mystery",
        ] {
            v.push(goodbye(r, None, None));
        }
        v
    }

    fn failures() -> Vec<PairFailure> {
        let mut v: Vec<PairFailure> = [
            "code", "closed", "expired", "stopped", "busy", "locked", "mystery",
        ]
        .iter()
        .map(|r| PairFailure::Refused {
            reason: r.to_string(),
            tries_left: None,
            retry_after: None,
            message: None,
        })
        .collect();
        v.push(PairFailure::BadProof);
        v.push(PairFailure::Net("timed out".into()));
        v
    }

    #[test]
    fn every_end_kind_has_text_without_commands() {
        for k in all_kinds() {
            let t = end_text(&k, "Studio", "Studio was not found on this network");
            assert!(!t.is_empty(), "{k:?}");
            assert!(!t.contains("transom "), "{k:?}: {t}");
        }
    }

    #[test]
    fn every_pair_failure_has_text_without_commands() {
        for f in failures() {
            let t = pair_failure("Studio", &f);
            assert!(!t.is_empty(), "{f:?}");
            assert!(!t.contains("transom "), "{f:?}: {t}");
        }
    }

    #[test]
    fn wrong_code_matches_the_cli() {
        let f = PairFailure::Refused {
            reason: "code".into(),
            tries_left: Some(2),
            retry_after: None,
            message: None,
        };
        assert_eq!(pair_failure("Studio", &f), "Wrong code. 2 tries left.");
        assert_eq!(
            pair_failure("Studio", &f),
            crate::text::pair_refusal("code", Some(2), None, None)
        );
    }

    #[test]
    fn busy_rounds_up() {
        assert_eq!(
            pairing_busy("Studio", Some(2.1)),
            "Studio is busy. Try again in 3 s."
        );
        assert_eq!(
            pairing_busy("Studio", None),
            "Studio is busy. Try again in 0 s."
        );
    }

    #[test]
    fn end_texts() {
        assert_eq!(
            end_text(&goodbye(gr::REMOVED, None, None), "Studio", ""),
            "Studio removed this computer. Pair again to use it."
        );
        assert_eq!(
            end_text(&goodbye(gr::UPDATE, None, Some("0.6.1")), "Studio", ""),
            "Update Transom to keep using Studio. It needs version 0.6.1 or later."
        );
        assert_eq!(
            end_text(
                &goodbye(gr::UPDATE, Some(" Please\nupdate "), None),
                "Studio",
                ""
            ),
            "Please update"
        );
        assert_eq!(
            end_text(&goodbye("mystery", None, None), "Studio", ""),
            "Studio closed the connection."
        );
        assert_eq!(
            end_text(
                &EndKind::ConnectFailed,
                "Studio",
                "Studio was not found\non this network"
            ),
            "Studio was not found on this network"
        );
        assert_eq!(
            end_text(&EndKind::Lost, "Studio", "ignored"),
            "Studio disconnected. Transom reconnects when it can reach it."
        );
    }

    #[test]
    fn mac_names_are_filtered() {
        assert_eq!(pair_title("  Stu\u{202E}dio\n"), "Pair with Studio");
        assert_eq!(pair_title("\u{200B}"), "Pair with Mac");
    }

    #[test]
    fn tiles() {
        assert_eq!(tile_label("Mock App", "First", false), "Mock App — First");
        assert_eq!(
            tile_label("Mock App", "First", true),
            "Mock App — First (showing)"
        );
        assert_eq!(tile_label("Finder", "", false), "Finder");
    }

    #[test]
    fn stats() {
        assert_eq!(stats_pill(Some(17.6), 60), "18 ms · 60 fps");
        assert_eq!(stats_pill(None, 0), "– ms · 0 fps");
        assert_eq!(
            stats_tooltip(Some(18.0), 60, &RouteDesc::Home),
            "Round trip 18 ms, 60 frames per second, connected at home"
        );
    }

    #[test]
    fn countdown() {
        assert_eq!(trying_again_in(4), "Trying again in 4 s");
    }

    #[test]
    fn copy_uses_curly_apostrophes_and_ellipses() {
        let samples = [
            WELCOME.to_string(),
            NO_MAC_HINT.to_string(),
            code_not_shown("Studio"),
            pairing_closed("Studio"),
            bad_proof("Studio"),
            no_answer("Studio"),
            NOT_A_LINK.to_string(),
            open_door_warning("Studio"),
            sill_too_old("Studio"),
            confirm_forget("Studio"),
            not_paired("Studio"),
            key_changed("Studio"),
            THIS_COMPUTERS_KEY.to_string(),
            LOCAL_KEYBOARD_WARNING.to_string(),
            software_encoder("Studio"),
        ];
        for s in samples {
            assert!(!s.contains('\''), "{s}");
            assert!(!s.contains("..."), "{s}");
        }
    }
}
