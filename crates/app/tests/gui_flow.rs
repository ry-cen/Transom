//! The graphical app's model and effects against the in-process mock host, with no window
//! (127.0.0.1 only; no Bonjour).
#![cfg(feature = "gui")]

use crossbeam_channel::Receiver;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use transom_app::gui::effects::{Effects, Platform};
use transom_app::gui::model::{AddField, Card, IdentityInfo, Model, Msg, Prefs, Screen};
use transom_app::session_state::Phase;
use transom_app::stream::Wake;
use transom_net::mock::{MockConfig, MockEvent, MockHost};
use transom_net::store::Store;
use transom_proto::identity::{b64url_encode, mac_id};
use transom_proto::messages::StreamSource;

const CODE: &str = "482913557208";

struct NullPlatform;

impl Platform for NullPlatform {
    fn set_fullscreen(&mut self, _on: bool) {}
    fn copy_text(&mut self, _s: &str) {}
    fn open_url(&mut self, _url: &str) {}
}

struct Rig {
    model: Model,
    effects: Effects,
    rx: Receiver<Msg>,
    store: Store,
    dir: PathBuf,
}

fn rig() -> Rig {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Tests run in parallel and the clock is coarse: a counter keeps the folders apart.
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("transom-flow-{}-{nanos}-{n}", std::process::id()));
    let store = Store::open(Some(dir.clone())).unwrap();
    let identity = store.identity().unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let noop: Wake = Arc::new(|| {});
    let effects = Effects::new(
        Handle::current(),
        store.clone(),
        identity,
        tx,
        noop.clone(),
        noop.clone(),
        noop,
    );
    let model = Model::new(
        IdentityInfo::default(),
        Prefs::default(),
        Vec::new(),
        Instant::now(),
    );
    Rig {
        model,
        effects,
        rx,
        store,
        dir,
    }
}

impl Rig {
    fn send(&mut self, msg: Msg) {
        let fx = self.model.update(msg, Instant::now());
        let prefs = self.model.prefs.clone();
        for e in fx {
            self.effects.run(e, &prefs, &mut NullPlatform);
        }
    }

    /// Delivers queued messages and session events, and ticks when a deadline passed, until
    /// `until` holds or `within` runs out. Returns whether it held.
    async fn pump(&mut self, until: impl Fn(&Model) -> bool, within: Duration) -> bool {
        let end = Instant::now() + within;
        while Instant::now() < end {
            let msgs: Vec<Msg> = self.rx.try_iter().collect();
            for m in msgs {
                self.send(m);
            }
            for ev in self.effects.session_events() {
                self.send(Msg::Session(ev));
            }
            let now = Instant::now();
            if self.model.next_deadline(now).is_some_and(|d| d <= now) {
                self.send(Msg::Tick);
            }
            if until(&self.model) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        until(&self.model)
    }

    async fn add_and_connect(&mut self, host: &MockHost, code: &str) {
        self.send(Msg::OpenAdd);
        self.send(Msg::SubmitAddress {
            address: format!("127.0.0.1:{}", host.addr.port()),
            code: code.to_string(),
        });
        let ok = self.pump(connected, Duration::from_secs(15)).await;
        assert!(ok, "the stream did not connect");
    }

    fn finish(mut self) {
        self.effects.close(Duration::ZERO);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn connected(m: &Model) -> bool {
    m.stream()
        .is_some_and(|s| matches!(s.session.phase, Phase::Connected))
}

async fn next_mock_event(
    host: &MockHost,
    within: Duration,
    mut want: impl FnMut(&MockEvent) -> bool,
) -> Option<MockEvent> {
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        while let Ok(v) = host.events().try_recv() {
            if want(&v) {
                return Some(v);
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_mac_by_address_pairs_saves_and_connects() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let mut r = rig();
    r.add_and_connect(&host, CODE).await;
    let id = mac_id(&host.fingerprint);
    let macs = r.store.macs().unwrap();
    assert!(
        macs.iter()
            .any(|m| m.mac_id == id && m.fingerprint == b64url_encode(&host.fingerprint)),
        "the Mac is not in the store"
    );
    let prefs = Prefs::load(&r.store);
    assert_eq!(
        prefs.macs.get(&id).and_then(|m| m.remote_host.clone()),
        Some(format!("127.0.0.1:{}", host.addr.port()))
    );
    assert!(matches!(r.model.screen, Screen::Stream(_)));
    r.finish();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn select_window_reaches_mock() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let mut r = rig();
    r.add_and_connect(&host, CODE).await;
    r.send(Msg::SelectWindow(102));
    // Keep delivering events while the mock sees the selection.
    let seen = next_mock_event(&host, Duration::from_secs(5), |e| {
        matches!(e, MockEvent::Select(StreamSource::Window { id: 102 }))
    })
    .await;
    assert!(seen.is_some(), "the mock did not see Select(Window 102)");
    r.finish();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_code_shows_tries_left() {
    let host = MockHost::start(MockConfig::default()).await.unwrap();
    let mut r = rig();
    r.send(Msg::OpenAdd);
    r.send(Msg::SubmitAddress {
        address: format!("127.0.0.1:{}", host.addr.port()),
        code: "000000000000".to_string(),
    });
    let has_problem = |m: &Model| {
        matches!(
            &m.screen,
            Screen::Macs { card: Some(Card::Add(a)) }
                if matches!(&a.problem, Some((AddField::Code | AddField::Card, _)))
        )
    };
    assert!(
        r.pump(has_problem, Duration::from_secs(15)).await,
        "no problem shown"
    );
    let Screen::Macs {
        card: Some(Card::Add(a)),
    } = &r.model.screen
    else {
        panic!("not the Add card");
    };
    let text = &a.problem.as_ref().unwrap().1;
    assert!(text.contains("2 tries left"), "got: {text}");
    r.finish();
}
