//! Dialog dispatch, and the small dialogs: About and the two confirmations (docs/ui-spec.md
//! section 3.10).

use super::{details, prefs, widgets, Cx};
use crate::gui::model::{Dialog, Msg};
use crate::gui::texts;
use egui::Ui;

/// Draws the open dialog, if any.
pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    let ctx = ui.ctx().clone();
    let Some(dialog) = &cx.model.dialog else {
        widgets::first_time(&ctx, "dialog", None);
        return;
    };
    match dialog {
        Dialog::Preferences { tab } => prefs::draw(&ctx, cx, *tab),
        Dialog::MacDetails { mac_id, problem } => {
            details::draw(&ctx, cx, mac_id, problem.as_deref())
        }
        Dialog::About => about(&ctx, cx),
        Dialog::ConfirmForget { mac_id, then_pair } => {
            let Some(mac) = cx.model.saved.iter().find(|m| &m.mac_id == mac_id) else {
                cx.out.push(Msg::CloseDialog);
                return;
            };
            let text = if *then_pair {
                texts::confirm_forget_and_pair(&mac.name)
            } else {
                texts::confirm_forget(&mac.name)
            };
            let first = widgets::first_time(&ctx, "dialog", Some(mac_id));
            confirm(&ctx, cx, &text, texts::FORGET, true, first);
        }
        Dialog::ConfirmPlain { found } => {
            let first = widgets::first_time(&ctx, "dialog", Some(&found.fullname));
            let text = texts::confirm_plain(&found.name);
            confirm(&ctx, cx, &text, texts::CONNECT, false, first);
        }
    }
}

/// A question with a confirming button and [Cancel], which has the focus.
fn confirm(ctx: &egui::Context, cx: &mut Cx, text: &str, button: &str, danger: bool, first: bool) {
    let mut msgs = Vec::new();
    let close = widgets::modal(ctx, "dialog", 440.0, |ui| {
        ui.label(text);
        ui.horizontal(|ui| {
            let cancel = ui.button(texts::CANCEL);
            if first {
                cancel.request_focus();
            }
            if cancel.clicked() {
                msgs.push(Msg::CloseDialog);
            }
            let ok = if danger {
                widgets::danger_button(ui, button)
            } else {
                ui.button(button)
            };
            if ok.clicked() {
                msgs.push(Msg::ConfirmDialog);
            }
        });
    });
    cx.out.extend(msgs);
    if close {
        cx.out.push(Msg::CloseDialog);
    }
}

fn about(ctx: &egui::Context, cx: &mut Cx) {
    let mut msgs = Vec::new();
    let close = widgets::modal(ctx, "dialog", 440.0, |ui| {
        ui.heading(texts::about_version());
        ui.label(texts::ABOUT_INDEPENDENT);
        ui.label(texts::ABOUT_LICENSE);
        ui.horizontal(|ui| {
            if ui.link(texts::ABOUT_LINK_SOURCE).clicked() {
                msgs.push(Msg::OpenUrl(texts::SOURCE_URL.into()));
            }
            if ui.link(texts::ABOUT_LINK_SILL).clicked() {
                msgs.push(Msg::OpenUrl(texts::SILL_URL.into()));
            }
        });
        if ui.button(texts::CLOSE).clicked() {
            msgs.push(Msg::CloseDialog);
        }
    });
    cx.out.extend(msgs);
    if close {
        cx.out.push(Msg::CloseDialog);
    }
}
