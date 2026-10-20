//! The Preferences dialog, Phase 1 (docs/ui-spec.md section 3.7): General, Input, Streaming and
//! Advanced.

use super::{theme, widgets, Cx};
use crate::gui::model::{Dialog, Msg, Prefs, PrefsTab};
use crate::gui::texts;
use egui::{Context, RichText, Ui};
use transom_proto::compat::{default_device_label, COMPAT_APP_VERSION};

/// Draws the dialog on `tab`.
pub fn draw(ctx: &Context, cx: &mut Cx, tab: PrefsTab) {
    let mut msgs = Vec::new();
    let model = cx.model;
    let drafts = &mut *cx.drafts;
    let p = &mut *cx.prefs;
    let close = widgets::modal(ctx, "dialog", 560.0, |ui| {
        ui.heading(texts::PREFERENCES);
        ui.horizontal(|ui| {
            for (t, text) in [
                (PrefsTab::General, texts::TAB_GENERAL),
                (PrefsTab::Input, texts::TAB_INPUT),
                (PrefsTab::Streaming, texts::TAB_STREAMING),
                (PrefsTab::Advanced, texts::TAB_ADVANCED),
            ] {
                if ui.add(egui::Button::new(text).selected(tab == t)).clicked() {
                    msgs.push(Msg::OpenDialog(Dialog::Preferences { tab: t }));
                }
            }
        });
        ui.separator();
        match tab {
            PrefsTab::General => general(ui, model, drafts, p, &mut msgs),
            PrefsTab::Input => input(ui, p),
            PrefsTab::Streaming => streaming(ui, p),
            PrefsTab::Advanced => advanced(ui, model, drafts, p),
        }
        ui.separator();
        if ui.button(texts::DONE).clicked() {
            msgs.push(Msg::CloseDialog);
        }
    });
    cx.out.extend(msgs);
    if close {
        cx.out.push(Msg::CloseDialog);
    }
}

fn mono(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).monospace().size(13.0)).wrap());
}

fn general(
    ui: &mut Ui,
    model: &crate::gui::model::Model,
    drafts: &mut super::Drafts,
    p: &mut Prefs,
    msgs: &mut Vec<Msg>,
) {
    let r = widgets::text_field(
        ui,
        texts::DEVICE_NAME,
        &mut drafts.device_name,
        &default_device_label(),
        true,
    );
    if let Some(t) = widgets::commit_on_blur(ui, &r, &drafts.device_name) {
        p.device_name = if t.is_empty() || t == default_device_label() {
            None
        } else {
            Some(t)
        };
    }
    widgets::muted(ui, texts::DEVICE_NAME_NOTE);
    ui.add_space(8.0);
    ui.label(RichText::new(texts::THIS_COMPUTERS_KEY).color(theme::MUTED));
    mono(ui, &model.identity.fingerprint_b64);
    ui.horizontal(|ui| {
        if ui.button(texts::COPY).clicked() {
            msgs.push(Msg::Copy(model.identity.fingerprint_b64.clone()));
        }
    });
    ui.collapsing(texts::SHOW_HEX, |ui| {
        mono(ui, &model.identity.fingerprint_hex)
    });
    ui.add_space(8.0);
    ui.label(RichText::new(texts::CONFIG_FOLDER).color(theme::MUTED));
    mono(ui, &model.identity.config_dir);
    if ui.button(texts::COPY).clicked() {
        msgs.push(Msg::Copy(model.identity.config_dir.clone()));
    }
}

fn input(ui: &mut Ui, p: &mut Prefs) {
    ui.label(RichText::new(texts::MODIFIER_LAYOUT).strong());
    widgets::keymap_radios(ui, p);
    egui::Grid::new("keymap-table")
        .striped(true)
        .show(ui, |ui| {
            for h in texts::KEYMAP_TABLE_HEADER {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for row in texts::KEYMAP_TABLE {
                for cell in row {
                    ui.label(cell);
                }
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    ui.label(RichText::new(texts::WHEN_LOCAL).strong());
    ui.checkbox(&mut p.allow_local_pointer, texts::SEND_THE_POINTER);
    ui.checkbox(&mut p.allow_local_keyboard, texts::SEND_THE_KEYBOARD);
    ui.label(
        RichText::new(texts::LOCAL_KEYBOARD_WARNING)
            .small()
            .color(theme::ERROR),
    );
}

fn streaming(ui: &mut Ui, p: &mut Prefs) {
    ui.label(RichText::new(texts::FPS_TO_ASK).strong());
    let fps = p.fps_to_ask();
    if ui.radio(fps == 60, texts::FPS_60).clicked() {
        p.fps = 60;
    }
    if ui.radio(fps == 120, texts::FPS_120).clicked() {
        p.fps = 120;
    }
    ui.checkbox(&mut p.show_stats, texts::SHOW_STATS);
}

fn advanced(
    ui: &mut Ui,
    model: &crate::gui::model::Model,
    drafts: &mut super::Drafts,
    p: &mut Prefs,
) {
    ui.checkbox(&mut p.allow_plain, texts::ALLOW_PLAIN);
    widgets::muted(ui, texts::PLAIN_WARNING);
    ui.add_space(8.0);
    let r = widgets::text_field(
        ui,
        texts::HELLO_VERSION,
        &mut drafts.hello_version,
        COMPAT_APP_VERSION,
        true,
    );
    if let Some(t) = widgets::commit_on_blur(ui, &r, &drafts.hello_version) {
        p.hello_version = if t.is_empty() || t == COMPAT_APP_VERSION {
            None
        } else {
            Some(t)
        };
    }
    if ui.button(texts::RESET).clicked() {
        p.hello_version = None;
        drafts.hello_version = COMPAT_APP_VERSION.to_string();
    }
    ui.add_space(8.0);
    ui.checkbox(&mut p.log_file, texts::WRITE_LOG);
    ui.add_enabled_ui(p.log_file, |ui| {
        if ui.radio(!p.log_debug, texts::LOG_INFO).clicked() {
            p.log_debug = false;
        }
        if ui.radio(p.log_debug, texts::LOG_DEBUG).clicked() {
            p.log_debug = true;
        }
    });
    widgets::muted(
        ui,
        &texts::log_path(&format!("{}/transom.log", model.identity.config_dir)),
    );
    widgets::muted(ui, texts::NEXT_LAUNCH);
}
