//! ParameterSets payload. Port of StreamMessage.swift:147-174.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterSets {
    pub nal_length_size: u8,
    pub sets: Vec<Vec<u8>>,
}

impl ParameterSets {
    /// None if shorter than 2 bytes or a set runs past the end; trailing bytes are ignored.
    pub fn decode(d: &[u8]) -> Option<Self> {
        let nal_length_size = *d.first()?;
        let count = usize::from(*d.get(1)?);
        let mut i = 2usize;
        let mut sets = Vec::with_capacity(count);
        for _ in 0..count {
            let len = usize::from(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]));
            i += 2;
            let end = i.checked_add(len)?;
            sets.push(d.get(i..end)?.to_vec());
            i = end;
        }
        Some(ParameterSets {
            nal_length_size,
            sets,
        })
    }

    /// A set longer than 65535 bytes is written truncated to 65535 (never happens).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![
            self.nal_length_size,
            u8::try_from(self.sets.len()).unwrap_or(u8::MAX),
        ];
        for s in self.sets.iter().take(usize::from(u8::MAX)) {
            let s = &s[..s.len().min(usize::from(u16::MAX))];
            out.extend_from_slice(&(s.len() as u16).to_be_bytes());
            out.extend_from_slice(s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_roundtrip() {
        let p = ParameterSets {
            nal_length_size: 4,
            sets: vec![vec![1, 2, 3], vec![4, 5, 6, 7, 8], vec![]],
        };
        assert_eq!(ParameterSets::decode(&p.encode()), Some(p));
    }

    #[test]
    fn params_truncated_is_none() {
        assert_eq!(ParameterSets::decode(&[]), None);
        assert_eq!(ParameterSets::decode(&[4]), None);
        assert_eq!(ParameterSets::decode(&[4, 1, 0, 3, 1, 2]), None);
        assert_eq!(ParameterSets::decode(&[4, 1, 0]), None);
    }

    #[test]
    fn params_trailing_ignored() {
        let p = ParameterSets::decode(&[4, 1, 0, 2, 9, 8, 77, 66]).unwrap();
        assert_eq!(p.sets, vec![vec![9, 8]]);
    }
}
