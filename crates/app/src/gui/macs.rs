//! The Macs screen's rows and the plans to connect to them (docs/ui-spec.md sections 3.3 and
//! 3.9). Pure. WP-B3 implements; the signature of `rows` is frozen (wave A).

use crate::gui::model::{MacRow, Prefs};
use transom_net::discovery::FoundMac;
use transom_net::store::SavedMac;

/// The rows of "MY MACS" (saved, by name) and "OTHER MACS ON THIS NETWORK" (found, unsaved, by
/// name). WP-B3.
pub fn rows(saved: &[SavedMac], found: &[FoundMac], prefs: &Prefs) -> (Vec<MacRow>, Vec<MacRow>) {
    let _ = (saved, found, prefs);
    (Vec::new(), Vec::new())
}
