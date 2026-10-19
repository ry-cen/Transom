//! What Transom tells the host about itself (Compatibility.swift:98-117).

use crate::messages::Hello;

pub const PROTOCOL: i64 = 1;
/// The Sill app version Transom reports in its hello (the upstream release it was verified
/// against).
pub const COMPAT_APP_VERSION: &str = "0.5.1";

pub fn os_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Unknown"
    }
}

pub fn default_device_label() -> String {
    format!("Transom ({})", os_name())
}

pub fn client_hello(app_version: &str, device: &str) -> Hello {
    Hello {
        app_version: Some(app_version.to_string()),
        build: Some(format!("transom {}", env!("CARGO_PKG_VERSION"))),
        protocol: Some(PROTOCOL),
        device: Some(device.to_string()),
    }
}

pub fn pair_model() -> String {
    format!("transom-{}", os_name().to_lowercase())
}
