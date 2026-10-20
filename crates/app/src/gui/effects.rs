//! Carries out the model's `Effect`s: pairing tasks, the stream, the store, prefs, clipboard,
//! URLs. Results come back as `Msg`s (docs/ui-spec.md section 8.2). Nothing here logs an
//! `Effect` or a `Msg`: codes and link secrets are inside them.

use crate::gui::model::{Effect, Msg, PairFailure, Prefs};
use crate::pairflow::{self, per_attempt, LinkPairError};
use crate::stream::{ChordRules, StreamCore, StreamOpts, Wake};
use crossbeam_channel::Sender;
use std::time::Duration;
use tokio::runtime::Handle;
use transom_net::dial::resolve;
use transom_net::identity::DeviceIdentity;
use transom_net::session::{SessionEvent, SourceChoice};
use transom_net::store::Store;
use transom_net::tls::PinPolicy;

/// What the effects runner needs from the window system.
pub trait Platform {
    fn set_fullscreen(&mut self, on: bool);
    fn copy_text(&mut self, s: &str);
    fn open_url(&mut self, url: &str);
}

pub struct Effects {
    rt: Handle,
    store: Store,
    identity: DeviceIdentity,
    msgs: Sender<Msg>,
    wake_msg: Wake,
    wake_net: Wake,
    wake_frame: Wake,
    stream: Option<StreamCore>,
}

/// Sends the answer of a pairing task, then wakes the UI.
fn answer(msgs: &Sender<Msg>, wake: &Wake, msg: Msg) {
    let _ = msgs.send(msg);
    wake();
}

impl Effects {
    pub fn new(
        rt: Handle,
        store: Store,
        identity: DeviceIdentity,
        msgs: Sender<Msg>,
        wake_msg: Wake,
        wake_net: Wake,
        wake_frame: Wake,
    ) -> Effects {
        Effects {
            rt,
            store,
            identity,
            msgs,
            wake_msg,
            wake_net,
            wake_frame,
            stream: None,
        }
    }

    pub fn stream(&self) -> Option<&StreamCore> {
        self.stream.as_ref()
    }

    pub fn stream_mut(&mut self) -> Option<&mut StreamCore> {
        self.stream.as_mut()
    }

    /// The running stream's session events. After a `MacInfo` (the supervisor rewrote
    /// macs.json) it also queues `Msg::SavedMacs` with the store's Macs.
    pub fn session_events(&mut self) -> Vec<SessionEvent> {
        let Some(core) = self.stream.as_mut() else {
            return Vec::new();
        };
        let events = core.take_events();
        if events
            .iter()
            .any(|e| matches!(e, SessionEvent::MacInfo { .. }))
        {
            self.send_saved();
        }
        events
    }

    /// Closes the stream, if any, waiting up to `wait` for it to flush (only when called off a
    /// runtime thread).
    pub fn close(&mut self, wait: Duration) {
        if let Some(core) = self.stream.take() {
            core.close(&self.rt, wait);
        }
    }

    fn send_saved(&self) {
        let macs = self.store.macs().unwrap_or_default();
        answer(&self.msgs, &self.wake_msg, Msg::SavedMacs(macs));
    }

    pub fn run(&mut self, e: Effect, prefs: &Prefs, platform: &mut dyn Platform) {
        match e {
            Effect::Ask { gen, addrs } => {
                let (id, name) = (self.identity.clone(), prefs.device_name());
                let (msgs, wake) = (self.msgs.clone(), self.wake_msg.clone());
                self.rt.spawn(async move {
                    let r = pairflow::ask(&id, addrs, per_attempt(false), &name)
                        .await
                        .map_err(|e| PairFailure::from_net(&e));
                    answer(&msgs, &wake, Msg::PairAsked(gen, r));
                });
            }
            Effect::PairCode {
                gen,
                addrs,
                pin,
                code,
                remote,
            } => {
                let (id, name) = (self.identity.clone(), prefs.device_name());
                let (msgs, wake) = (self.msgs.clone(), self.wake_msg.clone());
                self.rt.spawn(async move {
                    let policy = pin.map_or(PinPolicy::AnyKey, PinPolicy::Pinned);
                    let r =
                        pairflow::pair_code(&id, addrs, policy, per_attempt(remote), &code, &name)
                            .await
                            .map_err(|e| PairFailure::from_net(&e));
                    answer(&msgs, &wake, Msg::PairDone(gen, r));
                });
            }
            Effect::PairCodeAt {
                gen,
                host,
                port,
                code,
            } => {
                let (id, name) = (self.identity.clone(), prefs.device_name());
                let (msgs, wake) = (self.msgs.clone(), self.wake_msg.clone());
                self.rt.spawn(async move {
                    let r = match resolve(&host, port).await {
                        Ok(addrs) => pairflow::pair_code(
                            &id,
                            addrs,
                            PinPolicy::AnyKey,
                            per_attempt(true),
                            &code,
                            &name,
                        )
                        .await
                        .map_err(|e| PairFailure::from_net(&e)),
                        Err(e) => Err(PairFailure::from_net(&e)),
                    };
                    answer(&msgs, &wake, Msg::PairDone(gen, r));
                });
            }
            Effect::PairLink { gen, link } => {
                let (id, name) = (self.identity.clone(), prefs.device_name());
                let (msgs, wake) = (self.msgs.clone(), self.wake_msg.clone());
                self.rt.spawn(async move {
                    let r = match pairflow::link_candidates(&link, None).await {
                        Err(e) => Err(PairFailure::from_net(&e)),
                        Ok(candidates) => {
                            pairflow::pair_link(&id, &link, candidates, per_attempt(true), &name)
                                .await
                                .map_err(|e| match e {
                                    LinkPairError::Refused(e)
                                    | LinkPairError::Exhausted(Some(e)) => {
                                        PairFailure::from_net(&e)
                                    }
                                    LinkPairError::Exhausted(None) => {
                                        PairFailure::Net("no candidate".to_string())
                                    }
                                })
                        }
                    };
                    answer(&msgs, &wake, Msg::PairDone(gen, r));
                });
            }
            Effect::CancelAsk { addrs, pin } => {
                let (id, name) = (self.identity.clone(), prefs.device_name());
                self.rt.spawn(async move {
                    pairflow::cancel(
                        &id,
                        addrs,
                        PinPolicy::Pinned(pin),
                        per_attempt(false),
                        &name,
                    )
                    .await;
                });
            }
            Effect::SaveMac(m) => {
                if let Err(e) = self.store.save_mac(m) {
                    log::warn!("could not save the Mac: {e}");
                }
                self.send_saved();
            }
            Effect::Forget(id) => {
                if let Err(e) = self.store.forget(&id) {
                    log::warn!("could not forget the Mac: {e}");
                }
                self.send_saved();
            }
            Effect::SavePrefs(p) => {
                if let Err(e) = p.save(&self.store) {
                    log::warn!("could not save the preferences: {e}");
                }
                if let Some(core) = self.stream.as_mut() {
                    core.set_input_options(
                        p.keymap_mode(),
                        p.allow_local_pointer,
                        p.allow_local_keyboard,
                    );
                }
            }
            Effect::StartStream { route, .. } => {
                self.close(Duration::ZERO);
                let opts = StreamOpts {
                    source: SourceChoice::Auto,
                    keymap: prefs.keymap_mode(),
                    fps: prefs.fps_to_ask(),
                    allow_local_pointer: prefs.allow_local_pointer,
                    allow_local_keyboard: prefs.allow_local_keyboard,
                    hello_version: prefs.hello_version(),
                    name: prefs.device_name(),
                };
                self.stream = Some(StreamCore::start(
                    route,
                    opts,
                    self.identity.clone(),
                    Some(self.store.clone()),
                    &self.rt,
                    ChordRules::GUI,
                    self.wake_net.clone(),
                    self.wake_frame.clone(),
                ));
            }
            Effect::StopStream => self.close(Duration::ZERO),
            Effect::Session(c) => {
                if let Some(core) = &self.stream {
                    core.send(c);
                }
            }
            Effect::SetFullscreen(on) => platform.set_fullscreen(on),
            Effect::ReleaseAll => {
                if let Some(core) = self.stream.as_mut() {
                    core.release_all();
                }
            }
            Effect::CopyText(s) => platform.copy_text(&s),
            Effect::OpenUrl(u) => {
                if u.starts_with("https://") {
                    platform.open_url(&u);
                }
            }
        }
    }
}
