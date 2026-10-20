//! The graphical app's views: egui code that reads `&Model`, edits only `Drafts`, and pushes a
//! `Msg` for every user action (docs/ui-spec.md sections 3 and 8.2). WP-B4 implements `draw`
//! and adds the submodules; the items below are frozen (wave A).

use crate::gui::model::{Model, Msg};
use std::time::Instant;

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

/// Draws the whole window for one frame: `apply_seed` first, then the screen, cards, dialogs,
/// banner and toasts. WP-B4.
pub fn draw(
    ui: &mut egui::Ui,
    model: &Model,
    drafts: &mut Drafts,
    images: &dyn ImageLookup,
    now: Instant,
    out: &mut Vec<Msg>,
) {
    apply_seed(drafts, model);
    let _ = (ui, images, now, out);
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
