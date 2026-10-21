//! Protocol pieces the graphical app uses, against the in-process mock host (127.0.0.1 only).

use crossbeam_channel::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transom_net::identity::DeviceIdentity;
use transom_net::mock::{MockConfig, MockEvent, MockHost};
use transom_net::session::{
    EndKind, Outputs, SessionCommand, SessionEvent, SessionOptions, SourceChoice,
};
use transom_net::supervisor::{
    spawn_supervisor, Endpoint, Route, SessionHandle, SupervisorConfig, Trust,
};
use transom_proto::compat::client_hello;
use transom_proto::host_settings::{HostSettingsChange, HostSettingsState, StreamSettings};
use transom_proto::messages::{AppInfo, Goodbye, WindowAction, WindowCommand};
use transom_proto::video::VideoPacket;

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
    // Held so the video channel stays open.
    _video: Receiver<VideoPacket>,
}

fn start_route(route: Route, id: &DeviceIdentity) -> Running {
    let (out, events, video) = outputs();
    let handle = spawn_supervisor(
        &tokio::runtime::Handle::current(),
        SupervisorConfig {
            route,
            options: options(),
            identity: id.clone(),
            store: None,
        },
        out,
    );
    Running {
        handle,
        events,
        _video: video,
    }
}

fn start_session(host: &MockHost, id: &DeviceIdentity, trust: Trust) -> Running {
    start_route(
        Route::Direct {
            endpoint: Endpoint::Addrs(vec![host.addr]),
            trust,
            label: "Mock Mac".to_string(),
            saved: None,
            remote: false,
        },
        id,
    )
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

fn goodbye(reason: &str) -> Goodbye {
    Goodbye {
        reason: reason.to_string(),
        message: None,
        minimum_version: None,
        reconnect: None,
    }
}

fn settings_state() -> HostSettingsState {
    HostSettingsState {
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
    }
}

const JPEG: [u8; 5] = [0xFF, 0xD8, 1, 2, 3];

fn rich_config() -> MockConfig {
    MockConfig {
        require_pairing: false,
        thumbnails: vec![(101, JPEG.to_vec())],
        icons: vec![("com.example.mock".to_string(), b"\x89PNG..".to_vec())],
        apps: vec![AppInfo {
            name: "Mock App".to_string(),
            bundle_id: "com.example.mock".to_string(),
        }],
        host_settings: Some(settings_state()),
        ..MockConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thumbnails_icons_apps_and_settings_arrive() {
    let host = MockHost::start(rich_config()).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    connected(&r).await;
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        while let Ok(e) = r.events.try_recv() {
            seen.push(e);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(seen
        .iter()
        .any(|e| matches!(e, SessionEvent::AppList(a) if a.len() == 1)));
    assert!(seen.iter().any(|e| matches!(
        e,
        SessionEvent::AppIcon { bundle_id, png }
            if bundle_id == "com.example.mock" && png == b"\x89PNG.."
    )));
    assert!(seen.iter().any(|e| matches!(
        e,
        SessionEvent::Thumbnail { id: 101, jpeg } if jpeg == &JPEG
    )));
    assert!(seen
        .iter()
        .any(|e| matches!(e, SessionEvent::HostSettings(s) if s.settings.max_fps == 60)));
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn change_settings_is_answered_with_token() {
    let host = MockHost::start(rich_config()).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    connected(&r).await;
    let first = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::HostSettings(_))
    })
    .await;
    assert!(first.is_some(), "no initial HostSettings");
    r.handle
        .send(SessionCommand::ChangeSettings(HostSettingsChange {
            token: Some(7),
            max_fps: Some(120),
            ..Default::default()
        }));
    let ev = next_matching(host.events(), WAIT, |e| {
        matches!(e, MockEvent::ChangeSettings(_))
    })
    .await;
    match ev {
        Some(MockEvent::ChangeSettings(c)) => assert_eq!(c.max_fps, Some(120)),
        other => panic!("unexpected: {other:?}"),
    }
    let answer = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::HostSettings(_))
    })
    .await;
    match answer {
        Some(SessionEvent::HostSettings(s)) => {
            assert_eq!(s.answering, Some(7));
            assert_eq!(s.settings.max_fps, 120);
        }
        other => panic!("unexpected: {other:?}"),
    }
    r.handle.close();
}

async fn settings_answer(r: &Running, host: &MockHost, c: HostSettingsChange) -> HostSettingsState {
    connected(r).await;
    let first = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::HostSettings(_))
    })
    .await;
    assert!(first.is_some(), "no initial HostSettings");
    r.handle.send(SessionCommand::ChangeSettings(c));
    let ev = next_matching(host.events(), WAIT, |e| {
        matches!(e, MockEvent::ChangeSettings(_))
    })
    .await;
    assert!(ev.is_some(), "the mock never saw the change");
    match next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::HostSettings(_))
    })
    .await
    {
        Some(SessionEvent::HostSettings(s)) => s,
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn out_of_choice_change_is_refused_with_token() {
    let host = MockHost::start(rich_config()).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    let s = settings_answer(
        &r,
        &host,
        HostSettingsChange {
            token: Some(3),
            max_fps: Some(90),
            bitrate: Some(25_000_000),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(s.answering, Some(3));
    assert_eq!(s.settings.max_fps, 60);
    assert_eq!(s.settings.bitrate, 25_000_000);
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn virtual_display_refused_when_unavailable() {
    let host = MockHost::start(rich_config()).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    let s = settings_answer(
        &r,
        &host,
        HostSettingsChange {
            token: Some(4),
            virtual_display: Some(true),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(s.answering, Some(4));
    assert!(!s.settings.virtual_display);
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_mock_records_but_never_answers() {
    let host = MockHost::start(MockConfig {
        settings_silent: true,
        ..rich_config()
    })
    .await
    .unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    connected(&r).await;
    let first = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::HostSettings(_))
    })
    .await;
    assert!(first.is_some(), "no initial HostSettings");
    r.handle
        .send(SessionCommand::ChangeSettings(HostSettingsChange {
            token: Some(5),
            max_fps: Some(120),
            ..Default::default()
        }));
    let ev = next_matching(host.events(), WAIT, |e| {
        matches!(e, MockEvent::ChangeSettings(_))
    })
    .await;
    assert!(matches!(ev, Some(MockEvent::ChangeSettings(_))));
    let answer = next_matching(&r.events, Duration::from_millis(1500), |e| {
        matches!(e, SessionEvent::HostSettings(_))
    })
    .await;
    assert!(answer.is_none(), "a silent mock answered: {answer:?}");
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stats_carry_the_worst_round_trip() {
    let host = MockHost::start(rich_config()).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    connected(&r).await;
    let ev = next_matching(&r.events, Duration::from_secs(5), |e| {
        matches!(
            e,
            SessionEvent::Stats {
                rtt_ms: Some(_),
                rtt_max_ms: Some(_),
                ..
            }
        )
    })
    .await;
    match ev {
        Some(SessionEvent::Stats {
            rtt_ms: Some(m),
            rtt_max_ms: Some(x),
            ..
        }) => assert!(x >= m, "max {x} < median {m}"),
        other => panic!("no stats with a round trip: {other:?}"),
    }
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_now_cuts_backoff() {
    let cfg = MockConfig {
        require_pairing: false,
        goodbye_after_select: Some(goodbye("quit")),
        goodbye_only_first: true,
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    connected(&r).await;
    let ended = next_matching(&r.events, WAIT, |e| matches!(e, SessionEvent::Ended { .. })).await;
    match ended {
        Some(SessionEvent::Ended {
            retry_in: Some(d), ..
        }) => assert!(d >= Duration::from_secs(2), "{d:?}"),
        other => panic!("unexpected: {other:?}"),
    }
    r.handle.send(SessionCommand::RetryNow);
    let t0 = Instant::now();
    connected(&r).await;
    assert!(
        t0.elapsed() < Duration::from_millis(1500),
        "took {:?}",
        t0.elapsed()
    );
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosts_endpoint_tries_next_host() {
    let host = MockHost::start(MockConfig {
        require_pairing: false,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let route = Route::Direct {
        endpoint: Endpoint::Hosts(vec![
            ("127.0.0.1".into(), 1),
            ("127.0.0.1".into(), host.addr.port()),
        ]),
        trust: Trust::AnyKey,
        label: "Mock Mac".to_string(),
        saved: None,
        remote: true,
    };
    let r = start_route(route, &id());
    let c = next_matching(&r.events, Duration::from_secs(10), |e| {
        matches!(e, SessionEvent::Connected { .. })
    })
    .await;
    match c {
        Some(SessionEvent::Connected { peer, .. }) => assert_eq!(peer.port(), host.addr.port()),
        other => panic!("unexpected: {other:?}"),
    }
    r.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removed_goodbye_finishes_with_kind() {
    let cfg = MockConfig {
        require_pairing: false,
        goodbye_after_select: Some(goodbye("removed")),
        ..MockConfig::default()
    };
    let host = MockHost::start(cfg).await.unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    let finished = next_matching(&r.events, WAIT, |e| {
        matches!(e, SessionEvent::Finished { .. })
    })
    .await;
    match finished {
        Some(SessionEvent::Finished {
            kind: EndKind::Goodbye { reason, .. },
            ..
        }) => assert_eq!(reason, "removed"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unpaired_key_finishes_not_paired() {
    let host = MockHost::start(MockConfig {
        require_pairing: true,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    let finished = next_matching(&r.events, Duration::from_secs(10), |e| {
        matches!(e, SessionEvent::Finished { .. })
    })
    .await;
    assert!(
        matches!(
            finished,
            Some(SessionEvent::Finished {
                kind: EndKind::NotPaired,
                ..
            })
        ),
        "unexpected: {finished:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_and_window_command_reach_mock() {
    let host = MockHost::start(MockConfig {
        require_pairing: false,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let r = start_session(&host, &id(), Trust::AnyKey);
    connected(&r).await;
    r.handle
        .send(SessionCommand::LaunchApp("com.example.mock".to_string()));
    r.handle.send(SessionCommand::WindowCommand(WindowCommand {
        id: 102,
        action: WindowAction::Close,
    }));
    let launch = next_matching(host.events(), WAIT, |e| matches!(e, MockEvent::Launch(_))).await;
    match launch {
        Some(MockEvent::Launch(b)) => assert_eq!(b, "com.example.mock"),
        other => panic!("unexpected: {other:?}"),
    }
    let wc = next_matching(host.events(), WAIT, |e| {
        matches!(e, MockEvent::WindowCommand(_))
    })
    .await;
    match wc {
        Some(MockEvent::WindowCommand(c)) => {
            assert_eq!(c.id, 102);
            assert_eq!(c.action, WindowAction::Close);
        }
        other => panic!("unexpected: {other:?}"),
    }
    r.handle.close();
}
