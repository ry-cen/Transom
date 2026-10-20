//! The Add a Mac card (docs/ui-spec.md section 3.5): address and code, or a pairing link.

use super::pair::{code_block, failed, working_line};
use super::{widgets, Cx};
use crate::gui::model::{AddCard, AddField, Msg, PairState};
use crate::gui::texts;

fn field_problem(card: &AddCard, field: AddField) -> Option<&str> {
    card.problem
        .as_ref()
        .filter(|(f, _)| *f == field)
        .map(|(_, t)| t.as_str())
}

/// The name shown in the working and paired lines.
fn target_name(card: &AddCard) -> String {
    if let Some(a) = &card.typed_address {
        return a.clone();
    }
    card.pending_link
        .as_ref()
        .map(|l| l.name.clone())
        .unwrap_or_default()
}

/// The Add card.
pub fn draw(ui: &mut egui::Ui, cx: &mut Cx, card: &AddCard) {
    widgets::card_frame(ui).show(ui, |ui| {
        ui.set_width(super::macs::COLUMN_WIDTH - 36.0);
        ui.heading(texts::ADD_TITLE);
        match &card.state {
            PairState::Code { .. } | PairState::Asking => typing(ui, cx, card),
            PairState::Working => {
                let name = target_name(card);
                if card.link_mode {
                    ui.add_enabled(
                        false,
                        egui::TextEdit::singleline(&mut cx.drafts.link.clone())
                            .desired_width(f32::INFINITY),
                    );
                } else {
                    code_block(ui, cx, false, None);
                }
                working_line(ui, &texts::pairing_with(&name));
                ui.horizontal(|ui| {
                    ui.add_enabled(false, egui::Button::new(texts::PAIR));
                    ui.add_enabled(false, egui::Button::new(texts::CANCEL));
                });
            }
            PairState::Paired { mac, .. } => {
                let name = target_name(card);
                let name = if name.is_empty() {
                    mac.name.clone()
                } else {
                    name
                };
                ui.label(texts::paired_with(&name));
            }
            PairState::Failed { text } => failed(ui, cx, text),
        }
    });
}

fn typing(ui: &mut egui::Ui, cx: &mut Cx, card: &AddCard) {
    if let Some(link) = &card.pending_link {
        let first = link
            .addresses
            .first()
            .map(|a| {
                let mut a = a.clone();
                a.port.get_or_insert(link.port);
                a.text()
            })
            .unwrap_or_default();
        ui.label(texts::confirm_link(&link.name, &first));
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, true, texts::PAIR).clicked() {
                cx.out.push(Msg::ConfirmLink);
            }
            if ui.button(texts::CANCEL).clicked() {
                cx.out.push(Msg::CancelCard);
            }
        });
        return;
    }
    ui.label(texts::ADD_INTRO);
    ui.label(texts::ADD_HOW);
    if card.link_mode {
        let r = widgets::text_field(
            ui,
            texts::PAIRING_LINK,
            &mut cx.drafts.link,
            texts::LINK_PLACEHOLDER,
            true,
        );
        if let Some(p) = field_problem(card, AddField::Link) {
            widgets::problem(ui, p);
        }
        let enter = widgets::entered(ui, &r) && !cx.drafts.link.trim().is_empty();
        let mut submit = enter;
        ui.horizontal(|ui| {
            let ok = !cx.drafts.link.trim().is_empty();
            if widgets::primary_button(ui, ok, texts::PAIR).clicked() {
                submit = true;
            }
            if ui.button(texts::USE_ADDRESS).clicked() {
                cx.out.push(Msg::ToggleLinkMode);
            }
            if ui.button(texts::CANCEL).clicked() {
                cx.out.push(Msg::CancelCard);
            }
        });
        if let Some(p) = field_problem(card, AddField::Card) {
            widgets::problem(ui, p);
        }
        if submit {
            cx.out.push(Msg::SubmitLink(cx.drafts.link.trim().into()));
        }
    } else {
        widgets::text_field(
            ui,
            texts::ADDRESS,
            &mut cx.drafts.address,
            texts::ADDRESS_PLACEHOLDER,
            true,
        );
        if let Some(p) = field_problem(card, AddField::Address) {
            widgets::problem(ui, p);
        }
        let status = code_block(ui, cx, true, field_problem(card, AddField::Code));
        let ok = status.valid && !cx.drafts.address.trim().is_empty();
        let mut submit = status.submit && ok;
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, ok, texts::PAIR).clicked() {
                submit = true;
            }
            if ui.button(texts::USE_LINK).clicked() {
                cx.out.push(Msg::ToggleLinkMode);
            }
            if ui.button(texts::CANCEL).clicked() {
                cx.out.push(Msg::CancelCard);
            }
        });
        if let Some(p) = field_problem(card, AddField::Card) {
            widgets::problem(ui, p);
        }
        if submit {
            cx.out.push(Msg::SubmitAddress {
                address: cx.drafts.address.trim().into(),
                code: cx.drafts.code.clone(),
            });
        }
    }
}
