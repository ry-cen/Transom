//! Binary payloads. Ports of Switcher.swift:95-125 and Viewport.swift:55-72.

/// windowID(4, BE) + JPEG. None unless longer than 4 bytes.
pub fn decode_thumbnail(d: &[u8]) -> Option<(u32, &[u8])> {
    if d.len() <= 4 {
        return None;
    }
    Some((u32::from_be_bytes([d[0], d[1], d[2], d[3]]), &d[4..]))
}

/// bundleIDLength(2, BE) + bundleID UTF-8 + PNG.
pub fn decode_icon(d: &[u8]) -> Option<(String, &[u8])> {
    if d.len() <= 2 {
        return None;
    }
    let len = usize::from(u16::from_be_bytes([d[0], d[1]]));
    if d.len() <= 2 + len {
        return None;
    }
    let id = std::str::from_utf8(&d[2..2 + len]).ok()?;
    Some((id.to_string(), &d[2 + len..]))
}

/// hotX, hotY, width, height as f32 BE (Mac points), then PNG. Valid only when longer than 16.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorShape {
    pub hot_x: f32,
    pub hot_y: f32,
    pub width: f32,
    pub height: f32,
    pub png: Vec<u8>,
}

impl CursorShape {
    pub fn decode(d: &[u8]) -> Option<Self> {
        if d.len() <= 16 {
            return None;
        }
        let f = |i: usize| f32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
        Some(CursorShape {
            hot_x: f(0),
            hot_y: f(4),
            width: f(8),
            height: f(12),
            png: d[16..].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.png.len());
        for v in [self.hot_x, self.hot_y, self.width, self.height] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&self.png);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let c = CursorShape {
            hot_x: 1.5,
            hot_y: 2.5,
            width: 16.0,
            height: 24.0,
            png: vec![0x89, 0x50],
        };
        assert_eq!(CursorShape::decode(&c.encode()), Some(c));
    }

    #[test]
    fn cursor_16_bytes_is_none() {
        assert_eq!(CursorShape::decode(&[0u8; 16]), None);
    }

    #[test]
    fn icon_decode() {
        let d = [0, 3, b'a', b'.', b'b', 9];
        assert_eq!(decode_icon(&d), Some(("a.b".to_string(), &[9u8][..])));
        assert_eq!(decode_icon(&[0, 3, b'a', b'.', b'b']), None);
    }

    #[test]
    fn thumbnail_decode() {
        assert_eq!(
            decode_thumbnail(&[0, 0, 1, 2, 0xff]),
            Some((258, &[0xffu8][..]))
        );
        assert_eq!(decode_thumbnail(&[0, 0, 1, 2]), None);
    }
}
