//! Message framing. Port of StreamMessage.swift:3-134.

pub const HEADER_LEN: usize = 14;

/// Message kinds (StreamMessage.swift:5-80).
pub mod kind {
    pub const PARAMETER_SETS: u8 = 0;
    pub const FRAME: u8 = 1;
    pub const WINDOW_LIST: u8 = 2;
    pub const THUMBNAIL: u8 = 3;
    pub const APP_ICON: u8 = 4;
    pub const APP_LIST: u8 = 5;
    pub const SELECT_SOURCE: u8 = 6;
    pub const LAUNCH_APP: u8 = 7;
    pub const INPUT: u8 = 8;
    pub const VIEWPORT: u8 = 9;
    pub const PING: u8 = 10;
    pub const PONG: u8 = 11;
    pub const CLIENT_STATS: u8 = 12;
    pub const TICK: u8 = 13;
    pub const CURSOR_SHAPE: u8 = 14;
    pub const WINDOW_COMMAND: u8 = 15;
    pub const HOST_SETTINGS: u8 = 16;
    pub const CHANGE_SETTINGS: u8 = 17;
    pub const MAC_INFO: u8 = 18;
    pub const PAIR_REQUEST: u8 = 19;
    pub const PAIR_RESULT: u8 = 20;
    pub const PAIRING_WANTED: u8 = 21;
    pub const GOODBYE: u8 = 22;
    pub const HELLO: u8 = 23;
    pub const MAC_MENU: u8 = 24;
    pub const PRESS_MENU_ITEM: u8 = 25;
    pub const MAC_POINTER: u8 = 26;
    pub const FETCH_MENU: u8 = 27;
    pub const GESTURE: u8 = 28;
    pub const CLIPBOARD: u8 = 32;
}

/// Client-to-host payload cap (StreamMessage.swift:95-101).
pub const MAX_CLIENT_PAYLOAD: usize = 1 << 20;
/// Pairing message payload cap.
pub const MAX_PAIRING_PAYLOAD: usize = 4096;
/// Host frame (kind 1) payload cap.
pub const MAX_FRAME_PAYLOAD: usize = 32 << 20;
/// Any other host message payload cap.
pub const MAX_OTHER_HOST_PAYLOAD: usize = 4 << 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub kind: u8,
    pub timestamp: f64,
    pub is_keyframe: bool,
    pub len: u32,
}

impl Header {
    /// Port of StreamMessage.swift:113-134. The keyframe flag is true only when the byte is 1.
    pub fn parse(b: &[u8; HEADER_LEN]) -> Header {
        let ts = u64::from_be_bytes([b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8]]);
        Header {
            kind: b[0],
            timestamp: f64::from_bits(ts),
            is_keyframe: b[9] == 1,
            len: u32::from_be_bytes([b[10], b[11], b[12], b[13]]),
        }
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let t = self.timestamp.to_bits().to_be_bytes();
        let l = self.len.to_be_bytes();
        [
            self.kind,
            t[0],
            t[1],
            t[2],
            t[3],
            t[4],
            t[5],
            t[6],
            t[7],
            u8::from(self.is_keyframe),
            l[0],
            l[1],
            l[2],
            l[3],
        ]
    }
}

/// The cap a client applies to a host message of `kind` (StreamMessage.swift:95-101).
pub fn host_payload_cap(kind: u8) -> usize {
    if kind == kind::FRAME {
        MAX_FRAME_PAYLOAD
    } else {
        MAX_OTHER_HOST_PAYLOAD
    }
}

/// Header + payload. Never panics; the caller keeps payloads under `MAX_CLIENT_PAYLOAD`.
pub fn encode_message(kind: u8, timestamp: f64, is_keyframe: bool, payload: &[u8]) -> Vec<u8> {
    let h = Header {
        kind,
        timestamp,
        is_keyframe,
        len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
    };
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&h.encode());
    out.extend_from_slice(payload);
    out
}

/// Wall clock, seconds since 1970 (0.0 if the clock is before 1970).
pub fn now_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = Header {
            kind: 1,
            timestamp: 1.5,
            is_keyframe: true,
            len: 7,
        };
        assert_eq!(Header::parse(&h.encode()), h);
    }

    #[test]
    fn header_byte_layout() {
        assert_eq!(
            encode_message(10, 2.0, false, &[1, 2]),
            vec![10, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 2]
        );
    }

    #[test]
    fn keyframe_only_when_one() {
        let mut b = Header {
            kind: 1,
            timestamp: 0.0,
            is_keyframe: true,
            len: 0,
        }
        .encode();
        b[9] = 2;
        assert!(!Header::parse(&b).is_keyframe);
    }

    #[test]
    fn caps() {
        assert_eq!(host_payload_cap(1), 32 << 20);
        assert_eq!(host_payload_cap(2), 4 << 20);
    }
}
