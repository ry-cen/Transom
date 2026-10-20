use std::sync::{Arc, Mutex};

use crossbeam_channel::bounded;
use transom_proto::video::{host_packets_from_annexb, VideoPacket};
use transom_video::{
    init, spawn_decoder, Backend, DecodedFrame, Decoder, DecoderConfig, HwKind, HwPolicy, Planes,
};

fn sw() -> Decoder {
    Decoder::with_config(DecoderConfig::software()).unwrap()
}

fn check_planes(f: &DecodedFrame) {
    match f.planes() {
        Planes::I420 { y, u, v } => {
            assert!(y.stride >= 128);
            assert!(y.data.len() >= y.stride as usize * 96);
            assert!(u.stride >= 64 && v.stride >= 64);
            assert!(u.data.len() >= u.stride as usize * 48);
            assert!(v.data.len() >= v.stride as usize * 48);
        }
        Planes::Nv12 { y, uv } => {
            assert!(y.stride >= 128);
            assert!(y.data.len() >= y.stride as usize * 96);
            assert!(uv.stride >= 128);
            assert!(uv.data.len() >= uv.stride as usize * 48);
        }
    }
}

fn foreign() -> HwKind {
    if cfg!(target_os = "macos") {
        HwKind::D3d11va
    } else {
        HwKind::VideoToolbox
    }
}

fn decode_all(cfg: DecoderConfig) -> (Decoder, Vec<DecodedFrame>) {
    let pk = packets();
    let mut dec = Decoder::with_config(cfg).unwrap();
    let mut out = Vec::new();
    for p in &pk {
        if let Some(f) = feed(&mut dec, p) {
            out.push(f);
        }
    }
    (dec, out)
}

fn packets() -> Vec<VideoPacket> {
    init().unwrap();
    host_packets_from_annexb(include_bytes!("fixtures/tiny.hevc"))
}

/// Index of the Frame packet for picture `n` (0..20).
fn frame_index(pk: &[VideoPacket], n: usize) -> usize {
    pk.iter()
        .enumerate()
        .filter(|(_, p)| matches!(p, VideoPacket::Frame { .. }))
        .nth(n)
        .unwrap()
        .0
}

fn feed(dec: &mut Decoder, p: &VideoPacket) -> Option<DecodedFrame> {
    match p {
        VideoPacket::ParameterSets {
            nal_length_size,
            sets,
        } => {
            dec.set_parameter_sets(*nal_length_size, sets);
            None
        }
        VideoPacket::Frame {
            data,
            is_keyframe,
            timestamp,
        } => dec.decode(data, *is_keyframe, *timestamp).unwrap(),
        VideoPacket::Reset => {
            dec.reset().unwrap();
            None
        }
    }
}

#[test]
fn decodes_every_frame() {
    let pk = packets();
    let mut dec = sw();
    let mut n = 0;
    for p in &pk {
        if let Some(f) = feed(&mut dec, p) {
            n += 1;
            assert_eq!((f.width, f.height), (128, 96));
            assert!(f.full_range);
            assert!(!f.bt601);
            check_planes(&f);
            assert!(matches!(f.planes(), Planes::I420 { .. }));
            assert_eq!(dec.backend(), Backend::Software);
        }
    }
    assert_eq!(n, 20);
}

#[test]
fn waits_for_keyframe_after_a_drop() {
    let pk = packets();
    let mut dec = sw();
    let mut last = None;
    for p in &pk[..=frame_index(&pk, 1)] {
        last = feed(&mut dec, p);
    }
    assert!(last.is_some());
    dec.wait_for_keyframe();
    for n in 3..10 {
        assert!(feed(&mut dec, &pk[frame_index(&pk, n)]).is_none());
    }
    let f10 = frame_index(&pk, 10);
    assert!(matches!(pk[f10 - 1], VideoPacket::ParameterSets { .. }));
    assert!(feed(&mut dec, &pk[f10 - 1]).is_none());
    assert!(feed(&mut dec, &pk[f10]).is_some());
}

#[test]
fn repeated_parameter_sets_do_not_reset() {
    let pk = packets();
    let mut dec = sw();
    for p in &pk[..=frame_index(&pk, 0)] {
        feed(&mut dec, p);
    }
    if let VideoPacket::ParameterSets {
        nal_length_size,
        sets,
    } = &pk[0]
    {
        dec.set_parameter_sets(*nal_length_size, sets);
    } else {
        panic!("first packet is not parameter sets");
    }
    assert!(feed(&mut dec, &pk[frame_index(&pk, 1)]).is_some());
}

#[test]
fn nothing_before_parameter_sets() {
    let pk = packets();
    let mut dec = sw();
    let VideoPacket::Frame { data, .. } = &pk[frame_index(&pk, 0)] else {
        panic!("not a frame");
    };
    assert!(dec.decode(data, true, 0.0).unwrap().is_none());
}

#[test]
fn spawn_decoder_delivers() {
    let pk = packets();
    let (tx, rx) = bounded(64);
    let got: Arc<Mutex<Vec<(u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = got.clone();
    let h = spawn_decoder(rx, move |f| {
        sink.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((f.width, f.height));
    });
    for p in pk {
        tx.send(p).unwrap();
    }
    drop(tx);
    h.join().unwrap();
    let got = got.lock().unwrap_or_else(|e| e.into_inner());
    assert!(!got.is_empty());
    assert_eq!(*got.last().unwrap(), (128, 96));
}

#[test]
fn unavailable_hw_device_falls_back_to_software() {
    let mut dec = Decoder::with_config(DecoderConfig {
        hw: HwPolicy::Only(foreign()),
        sw_frame_threads: 0,
    })
    .unwrap();
    assert_eq!(dec.backend(), Backend::Software);
    let mut n = 0;
    for p in &packets() {
        if let Some(f) = feed(&mut dec, p) {
            n += 1;
            assert_eq!((f.width, f.height), (128, 96));
        }
    }
    assert_eq!(n, 20);
    assert_eq!(dec.backend(), Backend::Software);
}

#[test]
fn auto_policy_decodes_fixture() {
    let (dec, frames) = decode_all(DecoderConfig {
        hw: HwPolicy::Auto,
        sw_frame_threads: 0,
    });
    eprintln!("auto policy backend: {}", dec.backend());
    assert!(frames.len() >= 10, "{} frames", frames.len());
    for f in &frames {
        assert_eq!((f.width, f.height), (128, 96));
        assert!(f.full_range && !f.bt601);
        check_planes(f);
    }
    match dec.backend() {
        Backend::Software => {}
        Backend::Hardware(k) => assert_eq!(Some(k), HwKind::native()),
    }
}

#[test]
fn frame_threads_deliver_with_lag() {
    let (_, frames) = decode_all(DecoderConfig {
        hw: HwPolicy::Off,
        sw_frame_threads: 4,
    });
    let n = frames.len();
    assert!((16..20).contains(&n), "{n} frames");
    assert!(frames.iter().all(|f| (f.width, f.height) == (128, 96)));
}

#[test]
fn hardware_matches_software_pixels() {
    let (_, s) = decode_all(DecoderConfig::software());
    let (_, a) = decode_all(DecoderConfig {
        hw: HwPolicy::Auto,
        sw_frame_threads: 0,
    });
    eprintln!("software frames {}, auto frames {}", s.len(), a.len());
    assert!(!s.is_empty() && !a.is_empty());
    for (si, ai) in [(0, 0), (s.len() - 1, a.len() - 1)] {
        let Planes::Nv12 { y: ay, uv } = a[ai].planes() else {
            eprintln!("auto frames not NV12; pixel comparison skipped");
            return;
        };
        eprintln!("comparing frame {si} (sw) with {ai} (auto)");
        let Planes::I420 { y: sy, u, v } = s[si].planes() else {
            panic!("software is not I420");
        };
        for r in 0..96 {
            let (x, z) = (r * ay.stride as usize, r * sy.stride as usize);
            assert_eq!(ay.data[x..x + 128], sy.data[z..z + 128], "Y row {r}");
        }
        for r in 0..48 {
            let ro = r * uv.stride as usize;
            let (uo, vo) = (r * u.stride as usize, r * v.stride as usize);
            for c in 0..64 {
                assert_eq!(uv.data[ro + 2 * c], u.data[uo + c], "U {r},{c}");
                assert_eq!(uv.data[ro + 2 * c + 1], v.data[vo + c], "V {r},{c}");
            }
        }
    }
}
