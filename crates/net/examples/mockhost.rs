//! A mock Sill host on 127.0.0.1 for trying Transom without a Mac.
//!
//! Usage: `cargo run -p transom-net --example mockhost --features mock [video.hevc]`

use transom_net::mock::{MockConfig, MockEvent, MockHost};
use transom_proto::messages::to_json;
use transom_proto::pairing::group_code;
use transom_proto::video::host_packets_from_annexb;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../video/tests/fixtures/tiny.hevc"
        )
        .to_string()
    });
    let stream = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let cfg = MockConfig {
        video: host_packets_from_annexb(&stream),
        require_pairing: true,
        cursor: None,
        ..MockConfig::default()
    };
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

    let events = host.events().clone();
    std::thread::spawn(move || {
        for ev in events.iter() {
            match ev {
                MockEvent::Input(i) => {
                    println!("Input {}", String::from_utf8_lossy(&to_json(&i)));
                }
                other => println!("{other:?}"),
            }
        }
    });
    std::future::pending::<()>().await;
    Ok(())
}
