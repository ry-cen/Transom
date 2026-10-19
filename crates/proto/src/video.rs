//! Video as the host sends it, handed from the session (transom-net) to the decoder
//! (transom-video) over a channel. Upstream: StreamMessage.swift kinds 0 and 1, ParameterSets.

/// One step of the video stream.
#[derive(Clone, Debug, PartialEq)]
pub enum VideoPacket {
    /// Kind 0: the HEVC parameter sets (VPS, SPS, PPS, as VideoToolbox lists them) as raw NAL
    /// units without start codes, and the size of the big-endian length prefix in front of each
    /// NAL unit of the frames that follow (normally 4). The host repeats them before every
    /// keyframe; only a change of bytes means a new stream.
    ParameterSets {
        nal_length_size: u8,
        sets: Vec<Vec<u8>>,
    },
    /// Kind 1: one access unit: NAL units each preceded by a `nal_length_size`-byte big-endian
    /// length (hvcC style). `timestamp` is the host's wall clock at encode, seconds since 1970.
    Frame {
        data: Vec<u8>,
        is_keyframe: bool,
        timestamp: f64,
    },
    /// The session ended, or a new one starts: forget the decoder and wait for parameter sets.
    Reset,
}

/// HEVC NAL unit type: the 6 bits after the forbidden zero bit.
pub fn hevc_nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| (b >> 1) & 0x3f)
}

/// Splits an Annex B byte stream (00 00 01 or 00 00 00 01 start codes) into NAL units, without
/// their start codes and without trailing zero bytes.
pub fn split_annexb(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() {
            starts[k + 1] - 3
        } else {
            stream.len()
        };
        while e > s && stream[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&stream[s..e]);
        }
    }
    out
}

/// The packets a Sill host would send for an Annex B HEVC elementary stream whose pictures are
/// one slice each (as `ffmpeg -c:v libx265 -f hevc` writes): the stream's first VPS, SPS and PPS
/// as a `ParameterSets` (nal_length_size 4) before every keyframe, then one `Frame` per VCL NAL
/// unit (types 0..=31), a keyframe for the IRAP types 16..=23, timestamp 0. SEI and other
/// non-VCL units are dropped. A test and mock-host helper.
pub fn host_packets_from_annexb(stream: &[u8]) -> Vec<VideoPacket> {
    let nals = split_annexb(stream);
    let mut sets: Vec<Vec<u8>> = Vec::new();
    for t in [32u8, 33, 34] {
        if let Some(n) = nals.iter().find(|n| hevc_nal_type(n) == Some(t)) {
            sets.push(n.to_vec());
        }
    }
    let mut out = Vec::new();
    for n in nals {
        let Some(t) = hevc_nal_type(n) else { continue };
        if t > 31 {
            continue;
        }
        let is_keyframe = (16..=23).contains(&t);
        if is_keyframe {
            out.push(VideoPacket::ParameterSets {
                nal_length_size: 4,
                sets: sets.clone(),
            });
        }
        let mut data = (n.len() as u32).to_be_bytes().to_vec();
        data.extend_from_slice(n);
        out.push(VideoPacket::Frame {
            data,
            is_keyframe,
            timestamp: 0.0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_both_start_code_lengths() {
        let s = [
            0, 0, 0, 1, 0x40, 0x01, 0xAA, 0, 0, 1, 0x42, 0x01, 0xBB, 0x00,
        ];
        let n = split_annexb(&s);
        assert_eq!(n, vec![&[0x40, 0x01, 0xAA][..], &[0x42, 0x01, 0xBB][..]]);
        assert_eq!(hevc_nal_type(n[0]), Some(32));
        assert_eq!(hevc_nal_type(n[1]), Some(33));
    }

    #[test]
    fn host_packets_put_parameter_sets_before_each_keyframe() {
        // VPS(32) SPS(33) PPS(34) IDR_W_RADL(19) TRAIL_R(1) IDR(19)
        let mut s = Vec::new();
        for header in [
            [0x40u8, 0x01],
            [0x42, 0x01],
            [0x44, 0x01],
            [0x26, 0x01],
            [0x02, 0x01],
            [0x26, 0x01],
        ] {
            s.extend_from_slice(&[0, 0, 0, 1]);
            s.extend_from_slice(&header);
            s.push(0x80);
        }
        let p = host_packets_from_annexb(&s);
        assert_eq!(p.len(), 5);
        assert!(
            matches!(&p[0], VideoPacket::ParameterSets { nal_length_size: 4, sets } if sets.len() == 3)
        );
        assert!(
            matches!(&p[1], VideoPacket::Frame { is_keyframe: true, data, .. } if data[..4] == [0, 0, 0, 3])
        );
        assert!(matches!(
            &p[2],
            VideoPacket::Frame {
                is_keyframe: false,
                ..
            }
        ));
        assert!(matches!(&p[3], VideoPacket::ParameterSets { .. }));
        assert!(matches!(
            &p[4],
            VideoPacket::Frame {
                is_keyframe: true,
                ..
            }
        ));
    }
}
