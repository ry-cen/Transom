//! The graphical app's views: egui code that reads `&Model`, edits only `Drafts`, and pushes a
//! `Msg` for every user action (docs/ui-spec.md sections 3 and 8.2). WP-B4 implements `draw`
//! and adds the submodules; the items below are frozen (wave A).

use crate::gui::model::{
    Dialog, Model, Msg, Prefs, PrefsTab, Screen, StreamPanel, ZOOM_MAX, ZOOM_MIN,
};
use std::time::Instant;

pub mod add;
pub mod cards;
pub mod details;
pub mod dialogs;
pub mod icons;
pub mod mac_settings;
pub mod macs;
pub mod pair;
pub mod prefs;
pub mod stream_bar;
pub mod theme;
pub mod toasts;
pub mod widgets;

/// What every view reads and writes during one frame. Views read `model`, edit `drafts` and
/// `prefs` (a working copy; `draw` sends one `Msg::SetPrefs` at the end when it changed) and push
/// `Msg`s to `out`. They keep no other state and do no I/O.
pub struct Cx<'a> {
    pub model: &'a Model,
    pub images: &'a dyn ImageLookup,
    pub now: Instant,
    pub drafts: &'a mut Drafts,
    pub prefs: &'a mut Prefs,
    pub out: &'a mut Vec<Msg>,
}

/// Text being typed, owned by the view between frames. The model resets or fills fields through
/// `Model::seed_drafts`; `apply_seed` copies them in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drafts {
    pub code: String,
    pub address: String,
    pub link: String,
    pub search: String,
    pub remote_host: String,
    pub device_name: String,
    pub hello_version: String,
    /// The `Model::draft_epoch` last applied.
    pub epoch: u64,
}

/// A texture the image cache made, and its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub id: egui::TextureId,
    pub size: egui::Vec2,
}

/// Decoded pictures for the bar. None: not there (yet); draw the placeholder.
pub trait ImageLookup {
    fn thumbnail(&self, window_id: u32) -> Option<Image>;
    fn icon(&self, bundle_id: &str) -> Option<Image>;
}

/// Copies the model's draft seed into `drafts` once per `Model::draft_epoch`.
pub fn apply_seed(drafts: &mut Drafts, model: &Model) {
    if drafts.epoch == model.draft_epoch {
        return;
    }
    drafts.epoch = model.draft_epoch;
    let s = &model.draft_seed;
    let put = |field: &mut String, v: &Option<String>| {
        if let Some(v) = v {
            field.clone_from(v);
        }
    };
    put(&mut drafts.code, &s.code);
    put(&mut drafts.address, &s.address);
    put(&mut drafts.link, &s.link);
    put(&mut drafts.remote_host, &s.remote_host);
    put(&mut drafts.device_name, &s.device_name);
    put(&mut drafts.hello_version, &s.hello_version);
}

/// Draws the whole window for one frame: `apply_seed` first, then the shortcuts, the screen,
/// the dialog and the toasts (the banner belongs to the Macs screen).
pub fn draw(
    ui: &mut egui::Ui,
    model: &Model,
    drafts: &mut Drafts,
    images: &dyn ImageLookup,
    now: Instant,
    out: &mut Vec<Msg>,
) {
    apply_seed(drafts, model);
    let mut prefs = model.prefs.clone();
    {
        let mut cx = Cx {
            model,
            images,
            now,
            drafts,
            prefs: &mut prefs,
            out,
        };
        shortcuts(ui, &mut cx);
        match &model.screen {
            Screen::Macs { card } => macs::draw(ui, &mut cx, card.as_ref()),
            Screen::Stream(s) => {
                stream_bar::draw(ui, &mut cx, s);
                cards::draw(ui, &mut cx, s);
                if s.panel == Some(StreamPanel::MacSettings) {
                    mac_settings::draw(ui, &mut cx, s);
                }
            }
        }
        dialogs::draw(ui, &mut cx);
        toasts::draw(ui, &mut cx);
    }
    if prefs != model.prefs {
        out.push(Msg::SetPrefs(prefs));
    }
}

/// The keyboard shortcuts of Transom's own UI (spec 6.1). The shell gives keys to egui only in
/// Transom mode, so these never fire while keys go to the Mac.
fn shortcuts(ui: &mut egui::Ui, cx: &mut Cx) {
    use egui::{Key, Modifiers};
    let ctx = ui.ctx().clone();
    // Escape closes the top layer: a menu first (egui's own), then dialog, panel, card.
    if !egui::Popup::is_any_open(&ctx)
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        let panel_open = cx.model.stream().is_some_and(|s| s.panel.is_some());
        let card_open = matches!(cx.model.screen, Screen::Macs { card: Some(_) });
        if cx.model.dialog.is_some() {
            cx.out.push(Msg::CloseDialog);
        } else if panel_open {
            cx.out.push(Msg::ClosePanel);
        } else if card_open {
            cx.out.push(Msg::CancelCard);
        }
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Comma)) {
        cx.out.push(Msg::OpenDialog(Dialog::Preferences {
            tab: PrefsTab::General,
        }));
    }
    if matches!(cx.model.screen, Screen::Macs { .. })
        && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N))
    {
        cx.out.push(Msg::OpenAdd);
    }
    let zoom_in = ui.input_mut(|i| {
        let a = i.consume_key(Modifiers::COMMAND, Key::Equals);
        let b = i.consume_key(Modifiers::COMMAND, Key::Plus);
        a || b
    });
    let zoom_out = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Minus));
    if zoom_in || zoom_out {
        let step = if zoom_in { 0.1 } else { -0.1 };
        let z = ((cx.prefs.ui_zoom + step) * 100.0).round() / 100.0;
        cx.prefs.ui_zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::model::{DraftSeed, IdentityInfo, Prefs};

    #[test]
    fn seed_applies_once_per_epoch() {
        let now = Instant::now();
        let mut m = Model::new(IdentityInfo::default(), Prefs::default(), Vec::new(), now);
        let mut d = Drafts {
            code: "1234".into(),
            address: "kept".into(),
            ..Default::default()
        };
        apply_seed(&mut d, &m);
        assert_eq!(d.code, "1234");
        m.seed_drafts(DraftSeed {
            code: Some(String::new()),
            remote_host: Some("100.1.2.3".into()),
            ..Default::default()
        });
        apply_seed(&mut d, &m);
        assert_eq!(d.code, "");
        assert_eq!(d.address, "kept");
        assert_eq!(d.remote_host, "100.1.2.3");
        d.code = "9".into();
        apply_seed(&mut d, &m);
        assert_eq!(d.code, "9");
    }
}
