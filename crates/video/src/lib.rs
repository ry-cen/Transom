//! HEVC decoding (FFmpeg, software) of the host's length-prefixed access units.
mod annexb;
mod decoder;
pub use annexb::length_prefixed_to_annexb;
pub use decoder::{init, spawn_decoder, DecodedFrame, Decoder, VideoError};
