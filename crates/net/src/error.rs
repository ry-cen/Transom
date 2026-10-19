//! The error type of this crate.

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("could not connect: {0}")]
    Connect(String),
    #[error("timed out")]
    Timeout,
    #[error("TLS: {0}")]
    Tls(String),
    #[error("the Mac presented a different key from the one saved for it")]
    PinMismatch,
    #[error("this computer is not paired with the Mac")]
    NotPaired,
    #[error("the Mac did not agree on the protocol (ALPN)")]
    Alpn,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("the connection closed")]
    Closed,
    #[error("pairing refused ({reason})")]
    PairRefused {
        reason: String,
        tries_left: Option<i64>,
        retry_after: Option<f64>,
        message: Option<String>,
    },
    #[error("the Mac's pairing proof did not check")]
    BadMacProof,
    #[error("{0}")]
    Store(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}
