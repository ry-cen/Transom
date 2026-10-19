use std::sync::{Arc, Mutex};

use crossbeam_channel::bounded;
use transom_proto::video::{host_packets_from_annexb, VideoPacket};
use transom_video::{init, spawn_decoder, Decoder};

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

fn feed(dec: &mut Decoder, p: &VideoPacket) -> Option<transom_video::DecodedFrame> {
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
    let mut dec = Decoder::new().unwrap();
    let mut n = 0;
    for p in &pk {
        if let Some(f) = feed(&mut dec, p) {
            n += 1;
            assert_eq!((f.width, f.height), (128, 96));
            assert!(f.full_range);
            assert!(!f.bt601);
            assert!(f.y.len() >= f.y_stride as usize * 96);
            assert!(f.u.len() >= f.u_stride as usize * 48);
            assert!(f.v.len() >= f.v_stride as usize * 48);
        }
    }
    assert_eq!(n, 20);
}

#[test]
fn waits_for_keyframe_after_a_drop() {
    let pk = packets();
    let mut dec = Decoder::new().unwrap();
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
    let mut dec = Decoder::new().unwrap();
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
    let mut dec = Decoder::new().unwrap();
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
