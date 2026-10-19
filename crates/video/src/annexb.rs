use crate::decoder::VideoError;

/// Appends each NAL unit of `au` (each preceded by a `nal_len`-byte big-endian length, nal_len 1, 2 or 4)
/// to `out` behind a 00 00 00 01 start code. Err on another nal_len, a zero-length NAL, or a length past the end.
pub fn length_prefixed_to_annexb(
    au: &[u8],
    nal_len: u8,
    out: &mut Vec<u8>,
) -> Result<(), VideoError> {
    if !matches!(nal_len, 1 | 2 | 4) {
        return Err(VideoError::BadData("unsupported NAL length size"));
    }
    let n = nal_len as usize;
    let mut i = 0usize;
    while i < au.len() {
        let prefix = au
            .get(i..i + n)
            .ok_or(VideoError::BadData("truncated NAL length"))?;
        let len = prefix.iter().fold(0usize, |a, &b| (a << 8) | b as usize);
        i += n;
        if len == 0 {
            return Err(VideoError::BadData("zero-length NAL"));
        }
        let end = i
            .checked_add(len)
            .ok_or(VideoError::BadData("NAL length overflow"))?;
        let nal = au
            .get(i..end)
            .ok_or(VideoError::BadData("NAL runs past the end"))?;
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        i = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 3] = [0x40, 1, 2];
    const B: [u8; 2] = [0x26, 9];

    fn expected() -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1];
        v.extend_from_slice(&A);
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.extend_from_slice(&B);
        v
    }

    #[test]
    fn nal_len_4() {
        let au = [&[0, 0, 0, 3][..], &A, &[0, 0, 0, 2], &B].concat();
        let mut out = Vec::new();
        length_prefixed_to_annexb(&au, 4, &mut out).unwrap();
        assert_eq!(out, expected());
    }

    #[test]
    fn nal_len_2() {
        let au = [&[0, 3][..], &A, &[0, 2], &B].concat();
        let mut out = Vec::new();
        length_prefixed_to_annexb(&au, 2, &mut out).unwrap();
        assert_eq!(out, expected());
    }

    #[test]
    fn nal_len_1() {
        let au = [&[3][..], &A, &[2], &B].concat();
        let mut out = Vec::new();
        length_prefixed_to_annexb(&au, 1, &mut out).unwrap();
        assert_eq!(out, expected());
    }

    #[test]
    fn truncated_is_err() {
        let mut out = Vec::new();
        assert!(length_prefixed_to_annexb(&[0, 0, 0, 9, 1, 2], 4, &mut out).is_err());
        assert!(length_prefixed_to_annexb(&[0, 0], 4, &mut out).is_err());
        assert!(length_prefixed_to_annexb(&[0, 0, 0, 0], 4, &mut out).is_err());
    }

    #[test]
    fn bad_nal_len_3_is_err() {
        let mut out = Vec::new();
        assert!(length_prefixed_to_annexb(&[0, 0, 1, 7], 3, &mut out).is_err());
    }
}
