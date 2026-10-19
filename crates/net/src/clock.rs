//! A monotonic clock in seconds, for ping round trips and liveness.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Seconds since the first call in this process.
pub fn mono_seconds() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}
