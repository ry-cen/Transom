//! HEVC decoding (FFmpeg: hardware via VideoToolbox, D3D11VA or VAAPI with a software fallback)
//! of the host's length-prefixed access units.
mod annexb;
mod decoder;
mod frame;
mod hw;
pub use annexb::length_prefixed_to_annexb;
pub use decoder::{init, spawn_decoder, Decoder, VideoError};
pub use frame::{DecodedFrame, Plane, Planes};
pub use hw::{parse_frame_threads, Backend, DecoderConfig, HwKind, HwPolicy};
