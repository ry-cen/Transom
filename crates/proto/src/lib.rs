//! Sill's wire protocol, as the Mac host speaks it (upstream: Sources/StreamProtocol). Pure: no
//! I/O, no threads, no clock but `wire::now_timestamp`.
//!
//! Modules: `geometry` and `video` (frozen, from wave 0), plus `wire` (framing), `params`
//! (parameter sets), `messages` (JSON payloads), `blobs` (binary payloads), `identity`,
//! `pairing`, `pairlink`, `address`, `version`, `safetext`, `compat` and `goodbye`.

pub mod address;
pub mod blobs;
pub mod compat;
pub mod geometry;
pub mod goodbye;
pub mod identity;
pub mod messages;
pub mod pairing;
pub mod pairlink;
pub mod params;
pub mod safetext;
pub mod version;
pub mod video;
pub mod wire;
