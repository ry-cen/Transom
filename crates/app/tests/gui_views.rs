//! Headless view tests (egui_kittest): the Macs screen, the cards, the bar, the panel and the
//! dialogs, driven without a window. `Model::update` is never called here: the model is built
//! by hand and the views only push `Msg`s.
#![cfg(feature = "gui")]

use egui_kittest::kittest::{by, NodeT, Queryable};
use egui_kittest::Harness;
use std::time::{Duration, Instant};
use transom_app::gui::model::{
    AddCard, Banner, BannerAction, Card, Dialog, IdentityInfo, MacRow, Model, Msg, PairCard,
    PairState, Prefs, PrefsTab, RouteDesc, RowKey, RowWord, Screen, StreamPanel, StreamView, Toast,
    ToastAction, SETTINGS_WAIT,
};
use transom_app::gui::texts;
use transom_app::gui::view::{draw, Drafts, Image, ImageLookup};
use transom_app::session_state::Phase;
use transom_net::discovery::FoundMac;
use transom_net::session::{EndKind, SessionEvent};
use transom_net::store::SavedMac;
use transom_proto::host_settings::{HostSettingsChange, HostSettingsState};
use transom_proto::messages::{StreamSource, WindowInfo, WindowList};
use transom_proto::pairing::HomeDoor;

struct NoImages;

impl ImageLookup for NoImages {
    fn thumbnail(&self, _window_id: u32) -> Option<Image> {
        None
    }
    fn icon(&self, _bundle_id: &str) -> Option<Image> {
        None
    }
}

struct V {
    model: Model,
    drafts: Drafts,
    out: Vec<Msg>,
    now: Instant,
}

fn model(now: Instant) -> Model {
    Model::new(IdentityInfo::default(), Prefs::default(), Vec::new(), now)
}

fn harness(v: V) -> Harness<'static, V> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_ui_state(
            |ui, v: &mut V| {
                let now = v.now;
                draw(ui, &v.model, &mut v.drafts, &NoImages, now, &mut v.out);
            },
            v,
        )
}

fn start(model: Model, now: Instant) -> Harness<'static, V> {
    let mut h = harness(V {
        model,
        drafts: Drafts::default(),
        out: Vec::new(),
        now,
    });
    h.run();
    h
}

/// Like `start`, for screens with a spinner (which asks for repaints without end).
fn start_spinning(model: Model, now: Instant) -> Harness<'static, V> {
    let mut h = harness(V {
        model,
        drafts: Drafts::default(),
        out: Vec::new(),
        now,
    });
    h.run_steps(3);
    h
}

fn saved(id: &str, name: &str) -> SavedMac {
    SavedMac {
        mac_id: id.into(),
        name: name.into(),
        fingerprint: "AAAA".into(),
        recognition_key: "BBBB".into(),
        remote_port: None,
        addresses: Vec::new(),
        issued_at: None,
        last_address: None,
    }
}

fn found(name: &str, door: HomeDoor) -> FoundMac {
    FoundMac {
        name: name.into(),
        fullname: format!("{name}._sill._tcp.local."),
        host: "studio.local.".into(),
        port: 7455,
        addresses: vec!["192.168.1.20:7455".parse().unwrap()],
        door,
        tag: None,
    }
}

fn row(key: RowKey, name: &str, word: RowWord, saved: Option<SavedMac>) -> MacRow {
    MacRow {
        key,
        name: name.into(),
        word,
        detail: String::new(),
        saved,
        found: None,
        has_remote: false,
    }
}

fn saved_row(id: &str, name: &str, word: RowWord) -> MacRow {
    row(RowKey::Saved(id.into()), name, word, Some(saved(id, name)))
}

fn found_row(name: &str, word: RowWord, door: HomeDoor) -> MacRow {
    let mut r = row(
        RowKey::Found(format!("{name}._sill._tcp.local.")),
        name,
        word,
        None,
    );
    r.found = Some(found(name, door));
    r
}

fn pair_card(state: PairState) -> Card {
    Card::Pair(PairCard {
        mac_name: "Studio".into(),
        addrs: Vec::new(),
        state,
        gen: 1,
        pin: None,
        shown: true,
    })
}

fn add_card(link_mode: bool) -> Card {
    Card::Add(AddCard {
        link_mode,
        problem: None,
        pending_link: None,
        state: PairState::Code { problem: None },
        gen: 1,
        typed_address: None,
    })
}

/// A connected stream of "Studio" with two windows ("First" active).
fn stream(now: Instant, local_peer: bool) -> StreamView {
    let mut s = StreamView::new(Some("mac-1".into()), RouteDesc::Home, "Studio");
    let w = |id: u32, title: &str| WindowInfo {
        id,
        title: title.into(),
        app_name: "Mock App".into(),
        bundle_id: "com.example.mock".into(),
        width: 800,
        height: 600,
    };
    s.session.apply(
        SessionEvent::WindowList(WindowList {
            mac_name: "Studio".into(),
            windows: vec![w(101, "First"), w(102, "Second")],
            active: StreamSource::Window { id: 101 },
            launch_id: None,
            host_version: None,
            protocol: None,
            gestures: None,
            clipboard: None,
        }),
        now,
    );
    s.session.apply(
        SessionEvent::Connected {
            mac_name: "Studio".into(),
            peer: "127.0.0.1:7455".parse().unwrap(),
            local_peer,
        },
        now,
    );
    s
}

fn stream_model(now: Instant, local_peer: bool) -> Model {
    let mut m = model(now);
    m.screen = Screen::Stream(stream(now, local_peer));
    m
}

fn text_input(h: &Harness<'_, V>, label: &str) {
    let node = h.get(by().role(egui::accesskit::Role::TextInput).label(label));
    node.focus();
}

fn type_into(h: &mut Harness<'static, V>, label: &str, text: &str) {
    text_input(h, label);
    h.run();
    h.get(by().role(egui::accesskit::Role::TextInput).label(label))
        .type_text(text);
    h.run();
}

#[test]
fn rows_show_words_for_each_kind() {
    let now = Instant::now();
    let mut m = model(now);
    m.rows_mine = vec![
        saved_row("m1", "Studio", RowWord::Home),
        saved_row("m2", "Laptop", RowWord::Remote),
        saved_row("m3", "Gone", RowWord::NotFound),
    ];
    m.rows_other = vec![
        found_row("Door Mac", RowWord::NotPaired, HomeDoor::PairingRequired),
        found_row("Open Mac", RowWord::NotPaired, HomeDoor::Open),
        found_row("Old Mac", RowWord::UpdateSill, HomeDoor::Plain),
    ];
    let h = start(m, now);
    for label in [
        "Studio, Home",
        "Laptop, Remote",
        "Gone, Not found",
        "Door Mac, Not paired",
        "Open Mac, Not paired",
        "Old Mac, Update Sill",
    ] {
        h.get_by_label(label);
    }
}

#[test]
fn row_click_emits_click_row() {
    let now = Instant::now();
    let mut m = model(now);
    m.rows_mine = vec![saved_row("m1", "Studio", RowWord::Home)];
    let mut h = start(m, now);
    h.get_by_label("Studio, Home").click();
    h.run();
    assert!(
        matches!(&h.state().out[..], [Msg::ClickRow { key: RowKey::Saved(id) }] if id == "m1"),
        "{:?}",
        h.state().out
    );
}

#[test]
fn pair_card_typo_disables_pair() {
    let now = Instant::now();
    let mut m = model(now);
    m.screen = Screen::Macs {
        card: Some(pair_card(PairState::Code { problem: None })),
    };
    let mut h = harness(V {
        model: m,
        drafts: Drafts {
            code: "4829 1355 7209".into(),
            ..Default::default()
        },
        out: Vec::new(),
        now,
    });
    h.run();
    h.get_by_label(texts::CODE_TYPO);
    h.get_by_label(texts::PAIR).click();
    h.run();
    assert!(h.state().out.is_empty(), "{:?}", h.state().out);
    h.state_mut().drafts.code = "4829 1355 7208".into();
    h.run();
    h.get_by_label(texts::PAIR).click();
    h.run();
    assert!(
        h.state()
            .out
            .iter()
            .any(|m| matches!(m, Msg::SubmitCode(c) if c == "4829 1355 7208")),
        "{:?}",
        h.state().out
    );
}

#[test]
fn pair_card_cancel_emits_cancel() {
    let now = Instant::now();
    let mut m = model(now);
    m.screen = Screen::Macs {
        card: Some(pair_card(PairState::Code { problem: None })),
    };
    let mut h = start(m, now);
    h.get_by_label(texts::CANCEL).click();
    h.run();
    assert!(
        h.state().out.iter().any(|m| matches!(m, Msg::CancelCard)),
        "{:?}",
        h.state().out
    );
}

#[test]
fn add_card_submits_address_and_code() {
    let now = Instant::now();
    let mut m = model(now);
    m.screen = Screen::Macs {
        card: Some(add_card(false)),
    };
    let mut h = start(m, now);
    type_into(&mut h, texts::ADDRESS, "127.0.0.1:7455");
    type_into(&mut h, texts::CODE, "482913557208");
    assert_eq!(h.state().drafts.code, "4829 1355 7208");
    h.get_by_label(texts::PAIR).click();
    h.run();
    let ok = h.state().out.iter().any(|m| match m {
        Msg::SubmitAddress { address, code } => {
            address == "127.0.0.1:7455"
                && code
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>()
                    == "482913557208"
        }
        _ => false,
    });
    assert!(ok, "{:?}", h.state().out);
}

#[test]
fn bar_tiles_select_window() {
    let now = Instant::now();
    let mut m = stream_model(now, false);
    m.stream_mut().unwrap().bar.pinned = true;
    let mut h = start(m, now);
    h.get_by_label("Mock App — First (showing)");
    let second = h.get_by_label("Mock App — Second");
    second.click();
    h.run();
    assert!(
        h.state()
            .out
            .iter()
            .any(|m| matches!(m, Msg::SelectWindow(102))),
        "{:?}",
        h.state().out
    );
}

#[test]
fn reconnecting_card_try_now() {
    let now = Instant::now();
    let mut m = stream_model(now, false);
    m.stream_mut().unwrap().session.apply(
        SessionEvent::Ended {
            text: String::new(),
            retry_in: Some(Duration::from_secs(4)),
            kind: EndKind::Lost,
        },
        now,
    );
    assert!(matches!(
        m.stream().unwrap().session.phase,
        Phase::Reconnecting { .. }
    ));
    let mut h = start(m, now);
    h.get_by_label("Trying again in 4 s");
    h.get_by_label(texts::TRY_NOW).click();
    h.run();
    assert!(
        h.state().out.iter().any(|m| matches!(m, Msg::RetryNow)),
        "{:?}",
        h.state().out
    );
}

#[test]
fn connecting_card_cancel_disconnects() {
    let now = Instant::now();
    let mut m = model(now);
    m.screen = Screen::Stream(StreamView::new(None, RouteDesc::Home, "Studio"));
    let mut h = start_spinning(m, now);
    h.get_by_label(texts::CANCEL).click();
    h.run_steps(2);
    assert!(
        h.state().out.iter().any(|m| matches!(m, Msg::Disconnect)),
        "{:?}",
        h.state().out
    );
}

#[test]
fn connecting_card_filters_the_mac_string() {
    let now = Instant::now();
    let mut m = model(now);
    let mut s = StreamView::new(None, RouteDesc::Home, "Studio");
    let raw = "Connecting to Evil\u{202E}\u{7}Mac\u{0}\n\u{2026}";
    s.session.status = raw.into();
    m.screen = Screen::Stream(s);
    let expected = transom_proto::safetext::label(raw, 300);
    assert_ne!(expected, raw);
    let h = start_spinning(m, now);
    assert_eq!(h.query_all_by_label(&expected).count(), 1);
    assert_eq!(h.query_all_by_label(raw).count(), 0);
}

#[test]
fn banner_not_paired_has_pair_again() {
    let now = Instant::now();
    let mut m = model(now);
    m.banner = Some(Banner {
        text: texts::not_paired("Studio"),
        actions: vec![BannerAction::PairAgain],
        mac_id: Some("m1".into()),
    });
    let mut h = start(m, now);
    h.get_by_label(&texts::not_paired("Studio"));
    h.get_by_label(texts::PAIR_AGAIN).click();
    h.run();
    h.get_by_label(texts::DISMISS).click();
    h.run();
    let out = &h.state().out;
    assert!(
        out.iter()
            .any(|m| matches!(m, Msg::BannerAction(BannerAction::PairAgain))),
        "{out:?}"
    );
    assert!(
        out.iter().any(|m| matches!(m, Msg::DismissBanner)),
        "{out:?}"
    );
}

#[test]
fn prefs_keymap_radio_emits_setprefs() {
    let now = Instant::now();
    let mut m = model(now);
    m.dialog = Some(Dialog::Preferences {
        tab: PrefsTab::Input,
    });
    let mut h = start(m, now);
    h.get_by_label(texts::KEYMAP_MAC).click();
    h.run();
    let ok = h
        .state()
        .out
        .iter()
        .any(|m| matches!(m, Msg::SetPrefs(p) if p.keymap.as_deref() == Some("mac")));
    assert!(ok, "{:?}", h.state().out);
}

#[test]
fn view_only_chip_when_local_peer() {
    let now = Instant::now();
    let mut m = stream_model(now, true);
    m.stream_mut().unwrap().bar.pinned = true;
    let mut h = start(m, now);
    h.get_by_label(texts::VIEW_ONLY);
    {
        let p = &mut h.state_mut().model.prefs;
        p.allow_local_pointer = true;
        p.allow_local_keyboard = true;
    }
    h.run();
    assert!(h.query_by_label(texts::VIEW_ONLY).is_none());
}

#[test]
fn gear_panel_loads_then_says_update_sill() {
    let now = Instant::now();
    let mut m = stream_model(now, true);
    m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
    let h = start_spinning(m.clone(), now);
    h.get_by_label(texts::THIS_COMPUTER);
    h.get_by_label(texts::KEYMAP_PC);
    h.get_by_label(texts::SEND_KEYBOARD);
    assert_eq!(
        h.get_all_by_label(&texts::disconnect_from("Studio"))
            .count(),
        2
    );
    h.get_by_label(texts::LOADING_SETTINGS);
    assert!(h
        .query_by_role_and_label(egui::accesskit::Role::ComboBox, texts::QUALITY)
        .is_none());

    let h = start_spinning(m, now + SETTINGS_WAIT);
    h.get_by_label(&texts::update_sill_for_settings("Studio"));
    assert!(h.query_by_label(texts::LOADING_SETTINGS).is_none());
}

#[test]
fn footer_get_sill_opens_url() {
    let now = Instant::now();
    let mut h = start(model(now), now);
    h.get_by_label(texts::GET_SILL).click();
    h.run();
    assert!(
        h.state()
            .out
            .iter()
            .any(|m| matches!(m, Msg::OpenUrl(u) if u == texts::GET_SILL_URL)),
        "{:?}",
        h.state().out
    );
}

#[test]
fn about_dialog_shows_version() {
    let now = Instant::now();
    let mut h = start(model(now), now);
    h.get_by_label(texts::ABOUT_TRANSOM).click();
    h.run();
    assert!(
        h.state()
            .out
            .iter()
            .any(|m| matches!(m, Msg::OpenDialog(Dialog::About))),
        "{:?}",
        h.state().out
    );
    h.state_mut().out.clear();
    h.state_mut().model.dialog = Some(Dialog::About);
    h.run();
    h.get_by_label(&texts::about_version());
    h.get_by_label(texts::CLOSE).click();
    h.run();
    assert!(
        h.state().out.iter().any(|m| matches!(m, Msg::CloseDialog)),
        "{:?}",
        h.state().out
    );
}

#[test]
fn details_save_remote_address() {
    let now = Instant::now();
    let mut m = Model::new(
        IdentityInfo::default(),
        Prefs::default(),
        vec![saved("m1", "Studio")],
        now,
    );
    m.dialog = Some(Dialog::MacDetails {
        mac_id: "m1".into(),
        problem: None,
    });
    let mut h = start(m, now);
    type_into(&mut h, texts::REMOTE_ADDRESS, "100.1.2.3");
    h.get_by_label(texts::SAVE).click();
    h.run();
    let ok = h.state().out.iter().any(|m| {
        matches!(m, Msg::SetRemoteHost { mac_id, host: Some(h) } if mac_id == "m1" && h == "100.1.2.3")
    });
    assert!(ok, "{:?}", h.state().out);
}

#[test]
fn escape_closes_dialog() {
    let now = Instant::now();
    let mut m = model(now);
    m.dialog = Some(Dialog::About);
    let mut h = start(m, now);
    h.key_press(egui::Key::Escape);
    h.run();
    let closes = h
        .state()
        .out
        .iter()
        .filter(|m| matches!(m, Msg::CloseDialog))
        .count();
    assert_eq!(closes, 1, "{:?}", h.state().out);
}

/// Full screen can be left with no Mac connected: Escape, F11, the chord's F and a button.
#[test]
fn full_screen_can_be_left_from_the_macs_list() {
    let now = Instant::now();
    let toggles = |h: &Harness<'static, V>| {
        h.state()
            .out
            .iter()
            .filter(|m| matches!(m, Msg::ToggleFullscreen))
            .count()
    };
    let full = || {
        let mut m = model(now);
        m.fullscreen = true;
        m
    };

    let mut h = start(full(), now);
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(toggles(&h), 1, "Escape: {:?}", h.state().out);

    let mut h = start(full(), now);
    h.key_press(egui::Key::F11);
    h.run();
    assert_eq!(toggles(&h), 1, "F11: {:?}", h.state().out);

    let mut h = start(full(), now);
    let chord = egui::Modifiers {
        alt: true,
        ctrl: true,
        shift: true,
        mac_cmd: false,
        command: !cfg!(target_os = "macos"),
    };
    h.key_press_modifiers(chord, egui::Key::F);
    h.run();
    assert_eq!(toggles(&h), 1, "chord F: {:?}", h.state().out);

    let mut h = start(full(), now);
    h.get_by_label(texts::LEAVE_FULL_SCREEN).click();
    h.run();
    assert_eq!(toggles(&h), 1, "button: {:?}", h.state().out);

    // Not in full screen: no button, and Escape does nothing.
    let mut h = start(model(now), now);
    assert!(h.query_by_label(texts::LEAVE_FULL_SCREEN).is_none());
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(toggles(&h), 0, "{:?}", h.state().out);
}

/// The Mac settings panel shows what the Mac runs, and the software encoder in the error colour.
#[test]
fn mac_settings_shows_the_running_stream() {
    let now = Instant::now();
    let mut m = stream_model(now, false);
    let hs = serde_json::from_str(
        r#"{"settings": {"maxFPS": 120, "bitrate": 15000000, "captureScale": 2,
            "prioritizeSpeed": false, "virtualDisplay": false},
          "persistent": true, "virtualDisplayAvailable": false, "softwareEncoder": true,
          "stream": {"width": 5120, "height": 2880, "fps": 120, "mbps": 30,
            "onVirtualDisplay": false}}"#,
    )
    .unwrap();
    let s = m.stream_mut().unwrap();
    s.session.apply(SessionEvent::HostSettings(hs), now);
    s.panel = Some(StreamPanel::MacSettings);
    let h = start(m, now);
    h.get_by_label(&texts::now_streaming(5120, 2880, 120, 30));
    h.get_by_label(&texts::software_encoder("Studio"));
    assert!(h.query_by_label(texts::SOFTWARE_ENCODER).is_none());

    let mut m = stream_model(now, false);
    m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
    let h = start_spinning(m, now);
    assert!(h.query_by_label(texts::STREAM_NONE).is_none());
    assert!(h.query_by_label(texts::NOT_STREAMING).is_none());

    let mut m = stream_model(now, false);
    with_settings(
        &mut m,
        r#"{"settings": {"maxFPS": 120, "bitrate": 15000000, "captureScale": 2,
            "prioritizeSpeed": false, "virtualDisplay": false},
          "persistent": true, "virtualDisplayAvailable": false, "softwareEncoder": false}"#,
        now,
    );
    m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
    let h = start(m, now);
    h.get_by_label(texts::NOT_STREAMING);
}

/// The bar shows Sill's link report while the Mac says the link is behind, and nothing else.
#[test]
fn bar_shows_the_link_report_while_behind() {
    let now = Instant::now();
    let with_link = |link: &str| {
        let mut m = stream_model(now, false);
        let json = format!(
            r#"{{"settings": {{"maxFPS": 120, "bitrate": 15000000, "captureScale": 2,
                "prioritizeSpeed": false, "virtualDisplay": false}},
              "persistent": true, "virtualDisplayAvailable": false, "softwareEncoder": false
              {link}}}"#
        );
        let hs = serde_json::from_str(&json).unwrap();
        let s = m.stream_mut().unwrap();
        s.session.apply(SessionEvent::HostSettings(hs), now);
        s.bar.pinned = true;
        m
    };
    let behind = r#", "link": {"state": "behind", "withheldPerSecond": 23, "bitrate": 15000000,
        "carriedKbps": 9500, "suggestedBitrate": 8000000, "suggestedCaptureScale": 1}"#;
    let h = start(with_link(behind), now);
    h.get_by_label(&texts::link_chip(23));

    for other in [
        "",
        r#", "link": {"state": "stalled", "withheldPerSecond": 5, "bitrate": 15000000}"#,
    ] {
        let h = start(with_link(other), now);
        assert!(h.query_by_label(&texts::link_chip(5)).is_none());
        assert!(h.query_by_label(&texts::link_chip(23)).is_none());
    }
}

#[test]
fn every_dialog_and_card_renders_with_default_fonts() {
    let now = Instant::now();
    let base = |dialog: Option<Dialog>| {
        let mut m = Model::new(
            IdentityInfo {
                fingerprint_b64: "abc".into(),
                fingerprint_hex: "ab cd".into(),
                config_dir: "/tmp/transom".into(),
            },
            Prefs::default(),
            vec![saved("m1", "Studio")],
            now,
        );
        m.dialog = dialog;
        m
    };
    let mut models = Vec::new();
    for tab in [
        PrefsTab::General,
        PrefsTab::Input,
        PrefsTab::Streaming,
        PrefsTab::Advanced,
    ] {
        models.push(base(Some(Dialog::Preferences { tab })));
    }
    models.push(base(Some(Dialog::ConfirmForget {
        mac_id: "m1".into(),
        then_pair: false,
    })));
    models.push(base(Some(Dialog::ConfirmForget {
        mac_id: "m1".into(),
        then_pair: true,
    })));
    models.push(base(Some(Dialog::ConfirmPlain {
        found: found("Old", HomeDoor::Plain),
    })));
    let mut m = base(None);
    m.screen = Screen::Macs {
        card: Some(Card::OpenDoor {
            found: found("Open", HomeDoor::Open),
        }),
    };
    models.push(m);
    for card in [
        add_card(false),
        add_card(true),
        pair_card(PairState::Asking),
        pair_card(PairState::Working),
        pair_card(PairState::Failed {
            text: texts::wrong_code(Some(2)),
        }),
    ] {
        let mut m = base(None);
        m.screen = Screen::Macs { card: Some(card) };
        models.push(m);
    }
    let mut m = base(None);
    m.rows_mine = vec![saved_row("m1", "Studio", RowWord::Home)];
    m.status = texts::forgot("Studio");
    models.push(m);
    let mut m = stream_model(now, true);
    m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
    m.toasts = vec![Toast {
        text: texts::everyone_sees("Studio"),
        until: now + Duration::from_secs(3),
        action: None,
    }];
    models.push(m);
    let mut m = stream_model(now, false);
    with_settings(&mut m, FULL, now);
    m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
    m.toasts = vec![Toast {
        text: texts::link_behind("Studio"),
        until: now + Duration::from_secs(3),
        action: Some(ToastAction::LowerQuality),
    }];
    models.push(m);
    for m in models {
        let mut h = harness(V {
            model: m,
            drafts: Drafts::default(),
            out: Vec::new(),
            now,
        });
        h.run_steps(3);
    }
}

#[test]
fn zoom_shortcut_emits_clamped_setprefs() {
    let now = Instant::now();
    let mut h = start(model(now), now);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
    h.run();
    let z = h.state().out.iter().find_map(|m| match m {
        Msg::SetPrefs(p) => Some(p.ui_zoom),
        _ => None,
    });
    assert_eq!(z, Some(0.9), "{:?}", h.state().out);
}

#[test]
fn row_menu_offers_items_and_emits_row_menu() {
    use transom_app::gui::model::RowMenuItem;
    let now = Instant::now();
    let mut m = model(now);
    let mut r = saved_row("m1", "Studio", RowWord::Home);
    r.found = Some(found("Studio", HomeDoor::PairingRequired));
    m.rows_mine = vec![r];
    let mut h = start(m, now);
    h.get_by_label(&texts::row_menu_label("Studio")).click();
    h.run();
    h.get_by_label(texts::MENU_CONNECT);
    h.get_by_label(texts::MENU_CONNECT_HOME);
    assert!(h.query_by_label(texts::MENU_CONNECT_REMOTE).is_none());
    h.get_by_label(texts::MENU_DETAILS).click();
    h.run();
    let out = &h.state().out;
    assert!(
        out.iter().any(|m| matches!(
            m,
            Msg::RowMenu { key: RowKey::Saved(id), item: RowMenuItem::Details } if id == "m1"
        )),
        "{out:?}"
    );
    assert!(
        !out.iter().any(|m| matches!(m, Msg::ClickRow { .. })),
        "{out:?}"
    );
}

#[test]
fn arrow_down_moves_focus_between_rows() {
    let now = Instant::now();
    let mut m = model(now);
    m.rows_mine = vec![
        saved_row("m1", "Studio", RowWord::Home),
        saved_row("m2", "Laptop", RowWord::Remote),
    ];
    let mut h = start(m, now);
    h.get_by_label("Studio, Home").focus();
    h.run();
    assert!(h.get_by_label("Studio, Home").is_focused());
    h.key_press(egui::Key::ArrowDown);
    h.run();
    assert!(
        h.get_by_label("Laptop, Remote").is_focused(),
        "focus did not move to the second row"
    );
}

const FULL: &str = r#"{
  "settings": {"maxFPS": 120, "bitrate": 15000000, "captureScale": 2,
               "prioritizeSpeed": false, "virtualDisplay": false, "directWireless": false},
  "persistent": false,
  "virtualDisplayAvailable": true,
  "softwareEncoder": true,
  "stream": {"width": 2880, "height": 1800, "fps": 60, "mbps": 15, "onVirtualDisplay": false},
  "away": {"homeBitrate": 40000000, "homeCaptureScale": 2, "awayBitrate": 8000000,
           "awayCaptureScale": 1, "thisConnectionAway": true, "awayRunning": true}
}"#;

fn with_settings(m: &mut Model, json: &str, now: Instant) {
    let hs: HostSettingsState = serde_json::from_str(json).unwrap();
    m.stream_mut()
        .unwrap()
        .session
        .apply(SessionEvent::HostSettings(hs), now);
}

fn panel_with(json: &str, now: Instant) -> Harness<'static, V> {
    let mut m = stream_model(now, false);
    with_settings(&mut m, json, now);
    m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
    start(m, now)
}

fn combo<'a>(h: &'a Harness<'static, V>) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, texts::QUALITY)
}

fn single(out: &[Msg]) -> HostSettingsChange {
    assert_eq!(out.len(), 1, "{out:?}");
    match &out[0] {
        Msg::ChangeSettings(c) => c.clone(),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn streaming_section_shows_the_macs_values() {
    let now = Instant::now();
    let h = panel_with(FULL, now);
    assert_eq!(
        combo(&h).accesskit_node().value().as_deref(),
        Some("Balanced — 15 Mbps")
    );
    for l in [
        texts::RETINA,
        texts::STANDARD,
        texts::FPS_60,
        texts::FPS_120,
        texts::PRIORITIZE_SPEED,
        texts::VIRTUAL_DISPLAY,
        texts::NOT_PERSISTENT,
        texts::DIRECT_WIRELESS,
    ] {
        h.get_by_label(l);
    }
    for l in [
        texts::virtual_display_note("Studio"),
        texts::applies_to_every_device("Studio"),
        texts::now_streaming(2880, 1800, 60, 15),
        texts::counts_as_away("Studio"),
        texts::direct_wireless_note("Studio"),
    ] {
        h.get_by_label(&l);
    }
    assert!(h.state().out.is_empty(), "{:?}", h.state().out);
}

#[test]
fn custom_bitrate_is_shown_not_offered() {
    let now = Instant::now();
    let json = FULL.replace("15000000", "12000000");
    let mut h = panel_with(&json, now);
    assert_eq!(
        combo(&h).accesskit_node().value().as_deref(),
        Some("Custom — 12 Mbps")
    );
    combo(&h).click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Low — 4 Mbps");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Extreme — 150 Mbps");
    assert!(h
        .query_by_role_and_label(egui::accesskit::Role::Button, "Custom — 12 Mbps")
        .is_none());
}

#[test]
fn quality_pick_emits_one_field() {
    let now = Instant::now();
    let mut h = panel_with(FULL, now);
    combo(&h).click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "High — 25 Mbps")
        .click();
    h.run();
    assert_eq!(
        single(&h.state().out),
        HostSettingsChange {
            bitrate: Some(25_000_000),
            ..Default::default()
        }
    );
}

#[test]
fn radios_and_switches_emit_one_field_each() {
    let now = Instant::now();
    let cases: [(&str, HostSettingsChange); 5] = [
        (
            texts::STANDARD,
            HostSettingsChange {
                capture_scale: Some(1.0),
                ..Default::default()
            },
        ),
        (
            texts::FPS_60,
            HostSettingsChange {
                max_fps: Some(60),
                ..Default::default()
            },
        ),
        (
            texts::PRIORITIZE_SPEED,
            HostSettingsChange {
                prioritize_speed: Some(true),
                ..Default::default()
            },
        ),
        (
            texts::VIRTUAL_DISPLAY,
            HostSettingsChange {
                virtual_display: Some(true),
                ..Default::default()
            },
        ),
        (
            texts::DIRECT_WIRELESS,
            HostSettingsChange {
                direct_wireless: Some(true),
                ..Default::default()
            },
        ),
    ];
    for (label, want) in cases {
        let mut h = panel_with(FULL, now);
        h.get_by_label(label).click();
        h.run();
        assert_eq!(single(&h.state().out), want, "{label}");
    }
}

#[test]
fn virtual_display_disabled_only_when_unavailable_and_off() {
    let now = Instant::now();
    let note = "Start SillHost with --virtual-display.";
    let off = FULL.replace(
        "\"virtualDisplayAvailable\": true",
        &format!("\"virtualDisplayAvailable\": false, \"virtualDisplayNote\": \"{note}\""),
    );
    let mut h = panel_with(&off, now);
    assert!(h
        .get_by_label(texts::VIRTUAL_DISPLAY)
        .accesskit_node()
        .is_disabled());
    h.get_by_label(note);
    assert!(h
        .query_by_label(&texts::virtual_display_note("Studio"))
        .is_none());
    h.get_by_label(texts::VIRTUAL_DISPLAY).click();
    h.run();
    assert!(h.state().out.is_empty(), "{:?}", h.state().out);

    let on = off.replace("\"virtualDisplay\": false", "\"virtualDisplay\": true");
    let mut h = panel_with(&on, now);
    assert!(!h
        .get_by_label(texts::VIRTUAL_DISPLAY)
        .accesskit_node()
        .is_disabled());
    h.get_by_label(texts::VIRTUAL_DISPLAY).click();
    h.run();
    assert_eq!(
        single(&h.state().out),
        HostSettingsChange {
            virtual_display: Some(false),
            ..Default::default()
        }
    );
}

#[test]
fn no_direct_wireless_row_without_the_field() {
    let now = Instant::now();
    let json = FULL.replace(r#", "directWireless": false"#, "");
    let h = panel_with(&json, now);
    assert!(h.query_by_label(texts::DIRECT_WIRELESS).is_none());
    assert!(h
        .query_by_label(&texts::direct_wireless_note("Studio"))
        .is_none());
}

#[test]
fn pending_pick_renders_with_spinner() {
    let now = Instant::now();
    let mut m = stream_model(now, false);
    with_settings(&mut m, FULL, now);
    let s = m.stream_mut().unwrap();
    s.panel = Some(StreamPanel::MacSettings);
    s.session.settings.pick(
        &HostSettingsChange {
            max_fps: Some(60),
            ..Default::default()
        },
        1,
        now,
    );
    let early = m.clone();
    let h = start_spinning(m, now + Duration::from_millis(400));
    assert_eq!(
        h.query_all_by_role(egui::accesskit::Role::ProgressIndicator)
            .count(),
        1
    );
    let h2 = start_spinning(early, now + Duration::from_millis(299));
    assert_eq!(
        h2.query_all_by_role(egui::accesskit::Role::ProgressIndicator)
            .count(),
        0
    );
    h.get_by_label(texts::RETINA);
    let fps60 = h.get_by_label(texts::FPS_60);
    assert_eq!(
        fps60.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

#[test]
fn panel_open_while_not_connected_shows_only_not_streaming() {
    let now = Instant::now();
    let mut m = stream_model(now, false);
    with_settings(&mut m, FULL, now);
    let s = m.stream_mut().unwrap();
    s.panel = Some(StreamPanel::MacSettings);
    s.session.apply(
        SessionEvent::Ended {
            text: String::new(),
            retry_in: Some(Duration::from_secs(4)),
            kind: EndKind::Lost,
        },
        now,
    );
    assert!(!s.session.is_connected());
    let h = start(m, now);
    h.get_by_label(texts::NOT_STREAMING);
    assert!(h
        .query_all_by_role(egui::accesskit::Role::ProgressIndicator)
        .next()
        .is_none());
    assert!(h
        .query_by_role_and_label(egui::accesskit::Role::ComboBox, texts::QUALITY)
        .is_none());
    assert!(h.query_by_label(texts::LOADING_SETTINGS).is_none());
}

#[test]
fn link_toast_button_emits_lower_quality() {
    let now = Instant::now();
    let toast = |action| Toast {
        text: texts::link_behind("Studio"),
        until: now + Duration::from_secs(3),
        action,
    };
    let mut m = stream_model(now, false);
    m.toasts = vec![toast(Some(ToastAction::LowerQuality))];
    let mut h = start(m, now);
    h.get_by_label(texts::LOWER_QUALITY).click();
    h.run();
    assert!(
        h.state()
            .out
            .iter()
            .any(|m| matches!(m, Msg::ToastAction(ToastAction::LowerQuality))),
        "{:?}",
        h.state().out
    );

    let mut m = stream_model(now, false);
    m.toasts = vec![toast(None)];
    let h = start(m, now);
    assert!(h.query_by_label(texts::LOWER_QUALITY).is_none());
}
