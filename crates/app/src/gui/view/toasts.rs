//! Toasts: a short line at the bottom centre (docs/ui-spec.md section 3.6); the link notice
//! (BAR20) carries [Lower Quality] (BAR19).

use super::{theme, widgets, Cx};
use crate::gui::model::{Msg, ToastAction};
use crate::gui::texts;
use egui::{Align2, Area, Id, Order, RichText, Ui};

/// Draws every toast that has not expired.
pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    let ctx = ui.ctx().clone();
    let model = cx.model;
    let live = model.toasts.iter().filter(|t| t.until > cx.now);
    for (i, toast) in live.enumerate() {
        Area::new(Id::unique(("toast", i)))
            .order(Order::Foreground)
            .anchor(
                Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -24.0 - 52.0 * i as f32),
            )
            .interactable(toast.action.is_some())
            .show(&ctx, |ui| {
                widgets::card_frame(ui)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&toast.text).color(theme::TEXT));
                            if toast.action == Some(ToastAction::LowerQuality)
                                && ui.button(texts::LOWER_QUALITY).clicked()
                            {
                                cx.out.push(Msg::ToastAction(ToastAction::LowerQuality));
                            }
                        });
                    });
            });
    }
}
