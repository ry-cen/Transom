//! A mock Sill host for trying Transom without a Mac. It listens on 127.0.0.1 unless
//! `--bonjour` is given.
//!
//! Usage: `cargo run -p transom-net --example mockhost --features mock -- [--bonjour] [--name NAME] [--no-pairing] [video.hevc]`

use transom_net::discovery;
use transom_net::mock::{MockConfig, MockEvent, MockHost};
use transom_proto::host_settings::{HostSettingsState, StreamSettings};
use transom_proto::messages::{to_json, AppInfo};
use transom_proto::pairing::group_code;
use transom_proto::video::host_packets_from_annexb;

const USAGE: &str = "Usage: mockhost [--bonjour] [--name NAME] [--no-pairing] [video.hevc]

  --bonjour     listen on all interfaces and advertise as _sill._tcp
  --name NAME   the Mac's name (default \"Mock Mac\")
  --no-pairing  accept any key; with --bonjour, advertise the Open door (p=0)
  --help        print this help";

fn thumbnail(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(208, 132, image::Rgb(rgb));
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80)
        .encode_image(&img)
        .expect("jpeg encode");
    buf
}

fn icon() -> Vec<u8> {
    use image::ImageEncoder;
    let img = image::RgbaImage::from_pixel(64, 64, image::Rgba([0x9B, 0xB7, 0xFF, 0xFF]));
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), 64, 64, image::ExtendedColorType::Rgba8)
        .expect("png encode");
    buf
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bonjour = false;
    let mut name = "Mock Mac".to_string();
    let mut pairing = true;
    let mut path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--bonjour" => bonjour = true,
            "--no-pairing" => pairing = false,
            "--name" => name = args.next().ok_or("--name needs a value")?,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}").into()),
            _ => path = Some(a),
        }
    }
    let path = path.unwrap_or_else(|| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../video/tests/fixtures/tiny.hevc"
        )
        .to_string()
    });
    let stream = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut cfg = MockConfig {
        mac_name: name.clone(),
        video: host_packets_from_annexb(&stream),
        require_pairing: pairing,
        cursor: None,
        thumbnails: vec![
            (101, thumbnail([0x3A, 0x6E, 0xA5])),
            (102, thumbnail([0xA5, 0x5A, 0x3A])),
        ],
        icons: vec![("com.example.mock".to_string(), icon())],
        apps: vec![AppInfo {
            name: "Mock App".to_string(),
            bundle_id: "com.example.mock".to_string(),
        }],
        host_settings: Some(HostSettingsState {
            settings: StreamSettings {
                max_fps: 60,
                bitrate: 15_000_000,
                capture_scale: 2.0,
                prioritize_speed: false,
                virtual_display: false,
                direct_wireless: None,
            },
            persistent: true,
            virtual_display_available: false,
            virtual_display_note: None,
            software_encoder: false,
            stream: None,
            answering: None,
            away: None,
            link: None,
        }),
        ..MockConfig::default()
    };
    if bonjour {
        cfg.listen = "0.0.0.0:0".parse()?;
    }
    let code = group_code(&cfg.code);
    let host = MockHost::start(cfg).await?;
    let fp: String = host
        .fingerprint
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("mock host listening on {}", host.addr);
    println!("fingerprint: {fp}");
    println!("pairing code: {code}");

    // Kept alive until exit.
    let _daemon = if bonjour {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        let info = mdns_sd::ServiceInfo::new(
            discovery::SERVICE,
            &name,
            "transom-mock.local.",
            "",
            host.addr.port(),
            &[
                ("p", if pairing { "1" } else { "0" }),
                ("r", host.txt_tag().as_str()),
            ][..],
        )?
        .enable_addr_auto();
        daemon.register(info)?;
        println!("advertised as {name} (_sill._tcp)");
        println!(
            "warning: listening on all interfaces; anyone on this network can reach this mock"
        );
        Some(daemon)
    } else {
        None
    };

    let events = host.events().clone();
    std::thread::spawn(move || {
        for ev in events.iter() {
            match ev {
                MockEvent::Input(i) => {
                    println!("Input {}", String::from_utf8_lossy(&to_json(&i)));
                }
                // The pairing request carries the code's proof; never print it.
                MockEvent::Pair(_) => println!("Pair"),
                other => println!("{other:?}"),
            }
        }
    });
    std::future::pending::<()>().await;
    Ok(())
}
