//! Identity, storage, discovery, pairing and sessions against a Sill host.

pub mod catalog;
pub mod clock;
pub mod dial;
pub mod discovery;
mod error;
pub mod identity;
#[cfg(feature = "mock")]
pub mod mock;
pub mod pairing;
pub mod session;
pub mod store;
pub mod supervisor;
pub mod tls;

pub use error::NetError;

/// Any byte stream a session runs on: TLS or plain TCP.
pub trait Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Io for T {}
pub type BoxIo = Box<dyn Io>;
