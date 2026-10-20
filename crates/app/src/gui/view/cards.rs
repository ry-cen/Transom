//! The Stream screen's state cards (docs/ui-spec.md sections 3.6 and 8.5): connecting, waiting
//! for the first picture, and reconnecting. Each is a centred `egui::Area`.

use super::{widgets, Cx};
use crate::gui::model::{Msg, StreamView, WAITING_AFTER};
use crate::gui::texts;
use crate::session_state::Phase;
use egui::{Align2, Area, Id, Order, Ui};

/// Which card is up, as a number, for the once-per-state focus.
fn card_key(s: &StreamView, now: std::time::Instant) -> Option<u8> {
    match &s.session.phase {
        Phase::Connecting => Some(0),
        Phase::Connected => {
            let waiting = s.session.first_frame_at.is_none()
                && s.session
                    .connected_at
                    .is_some_and(|t| now >= t + WAITING_AFTER);
            waiting.then_some(1)
        }
        Phase::Reconnecting { .. } => Some(2),
        Phase::Finished { .. } => None,
    }
}

/// Draws the card for the stream's phase, if there is one.
pub fn draw(ui: &mut Ui, cx: &mut Cx, s: &StreamView) {
    let ctx = ui.ctx().clone();
    let phase_id = Id::unique("card-phase");
    let Some(key) = card_key(s, cx.now) else {
        ctx.data_mut(|d| d.remove_temp::<u8>(phase_id));
        return;
    };
    let last: Option<u8> = ctx.data(|d| d.get_temp(phase_id));
    let first_time = last != Some(key);
    ctx.data_mut(|d| d.insert_temp(phase_id, key));
    let mac = s.session.mac_name.as_str();
    let mut msgs = Vec::new();
    Area::new(Id::unique("card"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(&ctx, |ui| {
            widgets::card_frame(ui).show(ui, |ui| {
                ui.set_width(380.0);
                match &s.session.phase {
                    Phase::Connecting => {
                        let text = if s.session.status.is_empty() {
                            texts::connecting_to(mac)
                        } else {
                            s.session.status.clone()
                        };
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(text);
                        });
                        let cancel = ui.button(texts::CANCEL);
                        if first_time {
                            cancel.request_focus();
                        }
                        if cancel.clicked() {
                            msgs.push(Msg::Disconnect);
                        }
                    }
                    Phase::Connected => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(texts::WAITING_FOR_PICTURE);
                        });
                    }
                    Phase::Reconnecting {
                        kind,
                        text,
                        retry_at,
                    } => {
                        ui.label(texts::end_text(kind, mac, text));
                        ui.label(texts::trying_again_in(widgets::secs_until(
                            *retry_at, cx.now,
                        )));
                        ui.horizontal(|ui| {
                            let now_btn = ui.button(texts::TRY_NOW);
                            if first_time {
                                now_btn.request_focus();
                            }
                            if now_btn.clicked() {
                                msgs.push(Msg::RetryNow);
                            }
                            if ui.button(texts::DISCONNECT).clicked() {
                                msgs.push(Msg::Disconnect);
                            }
                        });
                    }
                    Phase::Finished { .. } => {}
                }
            });
        });
    cx.out.extend(msgs);
}
