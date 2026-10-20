//! The Mac details dialog (docs/ui-spec.md section 3.9): name, ID, key, reported addresses and
//! the manual remote address.

use super::{theme, widgets, Cx};
use crate::gui::model::{Dialog, Msg};
use crate::gui::texts;
use egui::{Context, RichText, Ui};
use transom_proto::safetext::label;

fn copyable(ui: &mut Ui, out: &mut Vec<Msg>, caption: &str, value: &str) {
    ui.label(RichText::new(caption).color(theme::MUTED));
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(value).monospace().size(13.0)).wrap());
        if ui.button(texts::COPY).clicked() {
            out.push(Msg::Copy(value.to_string()));
        }
    });
}

/// Draws the dialog for `mac_id`; closes it when the Mac is gone.
pub fn draw(ctx: &Context, cx: &mut Cx, mac_id: &str, problem: Option<&str>) {
    let Some(mac) = cx.model.saved.iter().find(|m| m.mac_id == mac_id) else {
        cx.out.push(Msg::CloseDialog);
        return;
    };
    let mut msgs = Vec::new();
    let drafts = &mut *cx.drafts;
    let close = widgets::modal(ctx, "dialog", 520.0, |ui| {
        ui.heading(texts::mac_name(&mac.name));
        ui.label(RichText::new(texts::NAME).color(theme::MUTED));
        ui.label(texts::mac_name(&mac.name));
        ui.label(RichText::new(texts::MAC_ID).color(theme::MUTED));
        ui.label(
            RichText::new(label(&mac.mac_id, 255))
                .monospace()
                .size(13.0),
        );
        copyable(ui, &mut msgs, texts::KEY_FINGERPRINT, &mac.fingerprint);
        ui.separator();
        ui.label(RichText::new(texts::REPORTED_ADDRESSES).color(theme::MUTED));
        if mac.addresses.is_empty() {
            widgets::muted(ui, texts::NO_REPORTED_ADDRESSES);
        }
        for a in &mac.addresses {
            ui.label(texts::reported_address(&a.host, a.port, &a.via, &a.kind));
        }
        ui.separator();
        widgets::text_field(
            ui,
            texts::REMOTE_ADDRESS,
            &mut drafts.remote_host,
            texts::ADDRESS_PLACEHOLDER,
            true,
        );
        widgets::muted(ui, &texts::remote_address_hint(&mac.name));
        if let Some(p) = problem {
            widgets::problem(ui, p);
        }
        let host = drafts.remote_host.trim().to_string();
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, !host.is_empty(), texts::SAVE).clicked() {
                msgs.push(Msg::SetRemoteHost {
                    mac_id: mac.mac_id.clone(),
                    host: Some(host.clone()),
                });
            }
            if ui.button(texts::CLEAR).clicked() {
                msgs.push(Msg::SetRemoteHost {
                    mac_id: mac.mac_id.clone(),
                    host: None,
                });
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if widgets::danger_button(ui, &texts::menu_forget(&mac.name)).clicked() {
                msgs.push(Msg::OpenDialog(Dialog::ConfirmForget {
                    mac_id: mac.mac_id.clone(),
                    then_pair: false,
                }));
            }
            if ui.button(texts::CLOSE).clicked() {
                msgs.push(Msg::CloseDialog);
            }
        });
    });
    cx.out.extend(msgs);
    if close {
        cx.out.push(Msg::CloseDialog);
    }
}
