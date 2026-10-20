//! The pairing card at home (docs/ui-spec.md section 3.4) and the open-door card. The code
//! block and the state lines are shared with the Add card.

use super::{theme, widgets, Cx};
use crate::gui::model::{Msg, PairCard, PairState};
use crate::gui::texts;
use transom_net::discovery::FoundMac;
use transom_proto::pairing::{check_code, CodeProblem};

/// What the code block found out this frame.
pub struct CodeStatus {
    /// The typed code passes `check_code`.
    pub valid: bool,
    /// Enter was pressed in the field while the code is valid.
    pub submit: bool,
}

/// The labelled code field with its live problem. `extra` is the model's problem (PAIR9 and the
/// like), shown when the live check has none.
pub fn code_block(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    enabled: bool,
    extra: Option<&str>,
) -> CodeStatus {
    let response = widgets::code_field(ui, texts::CODE, &mut cx.drafts.code, enabled);
    let check = check_code(&cx.drafts.code);
    let valid = check.is_ok();
    if matches!(check, Err(CodeProblem::Typo)) {
        widgets::problem(ui, texts::CODE_TYPO);
    } else if let Some(p) = extra {
        widgets::problem(ui, p);
    }
    CodeStatus {
        valid,
        submit: valid && enabled && widgets::entered(ui, &response),
    }
}

/// The spinner with its line.
pub fn working_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(text);
    });
}

/// The pairing card for a found Mac.
pub fn draw(ui: &mut egui::Ui, cx: &mut Cx, card: &PairCard) {
    let mac = card.mac_name.as_str();
    widgets::card_frame(ui).show(ui, |ui| {
        ui.set_width(super::macs::COLUMN_WIDTH - 36.0);
        ui.heading(texts::pair_title(mac));
        match &card.state {
            PairState::Asking => {
                working_line(ui, &texts::asking(mac));
                if ui.button(texts::CANCEL).clicked() {
                    cx.out.push(Msg::CancelCard);
                }
            }
            PairState::Code { problem } => {
                let intro = if card.shown {
                    texts::code_shown(mac)
                } else {
                    texts::code_not_shown(mac)
                };
                ui.label(intro);
                let status = code_block(ui, cx, true, problem.as_deref());
                let mut pair = status.submit;
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, status.valid, texts::PAIR).clicked() {
                        pair = true;
                    }
                    if ui.button(texts::CANCEL).clicked() {
                        cx.out.push(Msg::CancelCard);
                    }
                });
                if pair && status.valid {
                    cx.out.push(Msg::SubmitCode(cx.drafts.code.clone()));
                }
            }
            PairState::Working => {
                code_block(ui, cx, false, None);
                working_line(ui, &texts::pairing_with(mac));
                ui.horizontal(|ui| {
                    ui.add_enabled(false, egui::Button::new(texts::PAIR));
                    ui.add_enabled(false, egui::Button::new(texts::CANCEL));
                });
            }
            PairState::Paired { .. } => {
                ui.label(texts::paired_with(mac));
            }
            PairState::Failed { text } => failed(ui, cx, text),
        }
    });
}

/// A failed card: the text, [Try Again] and [Close].
pub fn failed(ui: &mut egui::Ui, cx: &mut Cx, text: &str) {
    widgets::problem(ui, text);
    ui.horizontal(|ui| {
        if ui.button(texts::TRY_AGAIN).clicked() {
            cx.out.push(Msg::RetryCard);
        }
        if ui.button(texts::CLOSE).clicked() {
            cx.out.push(Msg::CancelCard);
        }
    });
}

/// The card for an unsaved Mac with an open door (spec 3.3, DOOR1).
pub fn draw_open_door(ui: &mut egui::Ui, cx: &mut Cx, found: &FoundMac) {
    widgets::card_frame(ui).show(ui, |ui| {
        ui.set_width(super::macs::COLUMN_WIDTH - 36.0);
        ui.heading(texts::pair_title(&found.name));
        ui.label(texts::open_door_warning(&found.name));
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, true, texts::PAIR).clicked() {
                cx.out.push(Msg::PairOpenDoor);
            }
            if ui.button(texts::CANCEL).clicked() {
                cx.out.push(Msg::CancelCard);
            }
        });
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(texts::CONNECT_WITHOUT_PAIRING).color(theme::ACCENT),
                )
                .frame(false),
            )
            .clicked()
        {
            cx.out
                .push(Msg::Connect(crate::gui::model::ConnectPlan::Unpinned(
                    found.clone(),
                )));
        }
    });
}
