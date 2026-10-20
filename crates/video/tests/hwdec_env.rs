use transom_proto::video::{host_packets_from_annexb, VideoPacket};
use transom_video::{init, Backend, DecodedFrame, Decoder, DecoderConfig, HwPolicy};

fn packets() -> Vec<VideoPacket> {
    init().unwrap();
    host_packets_from_annexb(include_bytes!("fixtures/tiny.hevc"))
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
fn env_zero_forces_software() {
    // SAFETY: the only test in this binary, so no other thread reads the environment concurrently.
    unsafe { std::env::set_var("TRANSOM_HWDEC", "0") };
    assert_eq!(DecoderConfig::from_env().hw, HwPolicy::Off);
    let mut dec = Decoder::new().unwrap();
    assert_eq!(dec.backend(), Backend::Software);
    let n = packets()
        .iter()
        .filter_map(|p| feed(&mut dec, p))
        .filter(|f| (f.width, f.height) == (128, 96))
        .count();
    assert!(n >= 1);
}
