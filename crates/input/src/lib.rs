//! Maps winit keyboard, mouse and scroll events to Sill input events.
//!
//! Pure: no I/O, no threads. `KeyMapper` turns physical keys into HID-usage key events
//! (upstream `KeyStrokes.swift:157-232`), `PointerMapper` turns window positions, buttons and
//! wheel deltas into frame fractions (upstream `Input.swift:6-38`).

mod keys;
mod pointer;

pub use keys::{hid_usage, KeyMapper, KeymapMode};
pub use pointer::PointerMapper;
