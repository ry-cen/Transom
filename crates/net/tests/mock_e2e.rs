//! End-to-end tests of transom-net against the in-process mock host.

use crossbeam_channel::Receiver;
use std::sync::Arc;
use std::time::Duration;
use transom_net::identity::DeviceIdentity;
use transom_net::mock::{MockConfig, MockEvent, MockHost};
use transom_net::pairing::{ask, pair_with_code, pair_with_link, PairTarget};
use transom_net::session::{
    run_session, Outputs, SessionCommand, SessionEnd, SessionEvent, SessionOptions, SourceChoice,
};
use transom_net::supervisor::{
    open, spawn_supervisor, Endpoint, Route, SessionHandle, SupervisorConfig, Trust,
};
use transom_net::tls::{PinPolicy, SESSION_ALPN};
use transom_net::NetError;
use transom_proto::compat::client_hello;
use transom_proto::identity::mac_id;
use transom_proto::messages::{Goodbye, InputEvent, PointerAction, StreamSource};
use transom_proto::pairlink::PairLink;
use transom_proto::video::{host_packets_from_annexb, VideoPacket};

const TINY: &[u8] = include_bytes!("../../video/tests/fixtures/tiny.hevc");

async fn next_matching<T>(
    rx: &Receiver<T>,
    within: Duration,
    mut want: impl FnMut(&T) -> bool,
) -> Option<T> {
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        while let Ok(v) = rx.try_recv() {
            if want(&v) {
                return Some(v);
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

const WAIT: Duration = Duration::from_secs(5);

fn target(host: &MockHost, pin: PinPolicy) -> PairTarget {
    PairTarget {
        addrs: vec![host.addr],
        pin,
        per_attempt: Duration::from_secs(3),
    }
}

fn route(host: &MockHost, trust: Trust) -> Route {
    Route::Direct {
        endpoint: Endpoint::Addrs(vec![host.addr]),
        trust,
        label: "Mock Mac".to_string(),
        saved: None,
        remote: false,
    }
}

fn options() -> SessionOptions {
    SessionOptions {
        hello: client_hello("0.5.1", "Transom (Test)"),
        viewport: None,
        source: SourceChoice::Auto,
        stats_device: "Transom (Linux)".to_string(),
    }
}

fn outputs() -> (Outputs, Receiver<SessionEvent>, Receiver<VideoPacket>) {
    let (etx, erx) = crossbeam_channel::unbounded();
    let (vtx, vrx) = crossbeam_channel::bounded(256);
    (
        Outputs {
            events: etx,
            video: vtx,
            wake: Arc::new(|| {}),
        },
        erx,
        vrx,
    )
}

struct Running {
    handle: SessionHandle,
    events: Receiver<SessionEvent>,
    video: Receiver<VideoPacket>,
}

fn start_session(host: &MockHost, id: &DeviceIdentity, trust: Trust) -> Running {
    let (out, events, video) = outputs();
    let handle = spawn_supervisor(
        &tokio::runtime::Handle::current(),
        SupervisorConfig {
            route: route(host, trust),
            options: options(),
            identity: id.clone(),
            store: None,
        },
        out,
    );
    Running {
        handle,
        events,
        video,
    }
}

async fn connected(r: &Running) {
    let c = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::Connected { .. })
    })
    .await;
    assert!(c.is_some(), "no Connected event");
}

fn id() -> DeviceIdentity {
    DeviceIdentity::generate().unwrap().0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_code_succeeds() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    let paired = pair_with_code(
        &id,
        &target(&host, PinPolicy::AnyKey),
        "4829 1355 7208",
        "Test",
    )
    .await
    .unwrap();
    assert_eq!(paired.fingerprint, host.fingerprint);
    assert_eq!(paired.mac_id, mac_id(&host.fingerprint));
    assert_eq!(paired.name, "Mock Mac");
    assert_eq!(paired.recognition_key, host.recognition_key.to_vec());
    assert!(host.paired().contains(&id.fingerprint));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_wrong_code_reports_tries() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    let r = pair_with_code(
        &id,
        &target(&host, PinPolicy::AnyKey),
        "111111111111",
        "Test",
    )
    .await;
    match r {
        Err(NetError::PairRefused {
            reason, tries_left, ..
        }) => {
            assert_eq!(reason, "code");
            assert_eq!(tries_left, Some(2));
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(host.paired().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_link_succeeds() {
    let cfg = MockConfig::default();
    let secret = cfg.qr_secret;
    let host = MockHost::start(cfg).await.unwrap();
    let id = id();
    let link = PairLink {
        mac_id: mac_id(&host.fingerprint),
        fingerprint: host.fingerprint,
        secret,
        name: "Mock".to_string(),
        port: host.addr.port(),
        addresses: vec![],
    };
    let paired = pair_with_link(&id, &link, vec![host.addr], Duration::from_secs(3), "Test")
        .await
        .unwrap();
    assert_eq!(paired.fingerprint, host.fingerprint);
    assert!(host.paired().contains(&id.fingerprint));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_is_shown() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    let (result, fp) = ask(&id, &target(&host, PinPolicy::AnyKey), "Test")
        .await
        .unwrap();
    assert!(!result.ok);
    assert_eq!(result.reason.as_deref(), Some("shown"));
    assert_eq!(fp, host.fingerprint);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_first_then_auto_desktop() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    let first = next_matching(host.events(), WAIT, |_| true).await;
    assert!(matches!(first, Some(MockEvent::FirstKind(23))), "{first:?}");
    let hello = next_matching(host.events(), WAIT, |e| matches!(e, MockEvent::Hello(_))).await;
    match hello {
        Some(MockEvent::Hello(h)) => assert_eq!(h.protocol, Some(1)),
        other => panic!("no hello: {other:?}"),
    }
    let select = next_matching(host.events(), WAIT, |e| {
        matches!(e, MockEvent::Select(StreamSource::Desktop {}))
    })
    .await;
    assert!(select.is_some());
    connected(&r).await;
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn video_reaches_channel() {
    let cfg = MockConfig {
        video: host_packets_from_annexb(TINY),
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    let mut seen: Vec<VideoPacket> = Vec::new();
    let frame = next_matching(&r.video, Duration::from_secs(3), |p| {
        seen.push(p.clone());
        matches!(p, VideoPacket::Frame { .. })
    })
    .await;
    assert_eq!(seen.first(), Some(&VideoPacket::Reset));
    match frame {
        Some(VideoPacket::Frame { is_keyframe, .. }) => assert!(is_keyframe),
        other => panic!("no frame: {other:?}"),
    }
    assert!(seen
        .iter()
        .any(|p| matches!(p, VideoPacket::ParameterSets { .. })));
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unpaired_key_is_not_paired() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    let finished = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::Finished { .. })
    })
    .await;
    match finished {
        Some(SessionEvent::Finished { text }) => {
            assert!(text.contains("Pair it first"), "{text}")
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pin_mismatch_reported() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let result = open(&route(&host, Trust::Pinned([9; 32])), &id, SESSION_ALPN).await;
    match result {
        Err(NetError::PinMismatch) => {}
        Err(e) => panic!("unexpected error: {e}"),
        Ok(_) => panic!("connected despite the pin mismatch"),
    }
    let first = next_matching(host.events(), Duration::from_millis(500), |e| {
        matches!(e, MockEvent::FirstKind(_))
    })
    .await;
    assert!(first.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pings_and_stats() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    connected(&r).await;
    let end = tokio::time::Instant::now() + Duration::from_millis(2500);
    let (mut pings, mut stats, mut client_rtt) = (0, 0, false);
    while tokio::time::Instant::now() < end && !(pings >= 4 && stats >= 1 && client_rtt) {
        while let Ok(e) = host.events().try_recv() {
            match e {
                MockEvent::Ping => pings += 1,
                MockEvent::Stats(_) => stats += 1,
                _ => {}
            }
        }
        while let Ok(e) = r.events.try_recv() {
            if matches!(
                e,
                SessionEvent::Stats {
                    rtt_ms: Some(_),
                    ..
                }
            ) {
                client_rtt = true;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(pings >= 4, "pings: {pings}");
    assert!(stats >= 1, "stats: {stats}");
    assert!(client_rtt, "no client Stats with an rtt");
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_header_ends_session() {
    let cfg = MockConfig {
        oversized_after_hello: true,
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let (io, peer, fp, _) = open(
        &route(&host, Trust::Pinned(host.fingerprint)),
        &id,
        SESSION_ALPN,
    )
    .await
    .ok()
    .unwrap();
    let (out, _events, _video) = outputs();
    let (_tx, mut cmds) = tokio::sync::mpsc::unbounded_channel::<SessionCommand>();
    let mut opts = options();
    let mut noop = |_: &_, _: [u8; 32]| {};
    let end = tokio::time::timeout(
        WAIT,
        run_session(io, peer, fp, &mut opts, &mut cmds, &out, &mut noop),
    )
    .await
    .expect("run_session hung");
    assert!(
        matches!(
            end,
            SessionEnd::Lost {
                error: NetError::Protocol(_),
                ..
            }
        ),
        "{end:?}"
    );
}

fn goodbye(reason: &str) -> Goodbye {
    Goodbye {
        reason: reason.to_string(),
        message: None,
        minimum_version: None,
        reconnect: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goodbye_removed_finishes() {
    let cfg = MockConfig {
        goodbye_after_select: Some(goodbye("removed")),
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    let finished = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::Finished { .. })
    })
    .await;
    match finished {
        Some(SessionEvent::Finished { text }) => assert!(text.contains("removed"), "{text}"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goodbye_quit_reconnects() {
    let cfg = MockConfig {
        goodbye_after_select: Some(goodbye("quit")),
        goodbye_only_first: true,
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    connected(&r).await;
    let ended = next_matching(&r.events, WAIT, |e| matches!(e, SessionEvent::Ended { .. })).await;
    match ended {
        Some(SessionEvent::Ended { retry_in, .. }) => assert!(retry_in.is_some()),
        other => panic!("unexpected: {other:?}"),
    }
    connected(&r).await;
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moves_are_coalesced() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    connected(&r).await;
    for i in 0..50 {
        r.handle.send(SessionCommand::Input(InputEvent::Pointer {
            action: PointerAction::Move,
            x: i as f64 / 50.0,
            y: 0.5,
        }));
    }
    r.handle.send(SessionCommand::Input(InputEvent::Pointer {
        action: PointerAction::LeftDown,
        x: 0.5,
        y: 0.5,
    }));
    let mut moves: Vec<f64> = Vec::new();
    let down = next_matching(host.events(), WAIT, |e| match e {
        MockEvent::Input(InputEvent::Pointer {
            action: PointerAction::Move,
            x,
            ..
        }) => {
            moves.push(*x);
            false
        }
        MockEvent::Input(InputEvent::Pointer {
            action: PointerAction::LeftDown,
            ..
        }) => true,
        _ => false,
    })
    .await;
    assert!(down.is_some(), "no LeftDown reached the host");
    assert!(moves.len() < 50, "moves: {}", moves.len());
    assert_eq!(moves.last().copied(), Some(49.0 / 50.0));
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_session_works() {
    let cfg = MockConfig {
        tls: false,
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let id = id();
    let r = start_session(&host, &id, Trust::Plain);
    connected(&r).await;
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inputs_before_close_reach_the_host() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let id = id();
    host.pair_key(id.fingerprint);
    let r = start_session(&host, &id, Trust::Pinned(host.fingerprint));
    connected(&r).await;
    r.handle.send(SessionCommand::Input(InputEvent::Pointer {
        action: PointerAction::LeftUp,
        x: 0.5,
        y: 0.5,
    }));
    r.handle.close();
    let join = r.handle.take_join().unwrap();
    tokio::time::timeout(Duration::from_secs(2), join)
        .await
        .expect("supervisor did not stop")
        .unwrap();
    let up = next_matching(host.events(), WAIT, |e| {
        matches!(
            e,
            MockEvent::Input(InputEvent::Pointer {
                action: PointerAction::LeftUp,
                ..
            })
        )
    })
    .await;
    assert!(up.is_some(), "the button-up sent before Close was lost");
}
