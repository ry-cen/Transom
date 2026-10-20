//! The Phase 1 Mac Settings panel (docs/ui-spec.md section 3.8): A (the connection), D (this
//! computer) and E (Disconnect). Sections B and C are Phase 2.

use super::stream_bar::BAR_HEIGHT;
use super::{theme, widgets, Cx};
use crate::gui::model::{Msg, RouteDesc, StreamView};
use crate::gui::texts;
use egui::{Align2, Area, Frame, Id, Margin, Order, Ui};

const WIDTH: f32 = 360.0;
const FOOTER_HEIGHT: f32 = 64.0;

/// Draws the panel at the right, under the bar.
pub fn draw(ui: &mut Ui, cx: &mut Cx, s: &StreamView) {
    let ctx = ui.ctx().clone();
    let height = (ctx.content_rect().height() - BAR_HEIGHT).max(FOOTER_HEIGHT);
    let mac = texts::mac_name(&s.session.mac_name);
    Area::new(Id::unique("panel"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_TOP, egui::vec2(0.0, BAR_HEIGHT))
        .show(&ctx, |ui| {
            Frame::new()
                .fill(theme::PANEL)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(WIDTH - 32.0);
                    ui.set_height(height - 32.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(height - 32.0 - FOOTER_HEIGHT)
                        .show(ui, |ui| {
                            connection(ui, s, &mac);
                            this_computer(ui, cx, s);
                        });
                    ui.separator();
                    if ui.button(texts::disconnect_from(&mac)).clicked() {
                        cx.out.push(Msg::Disconnect);
                    }
                });
        });
}

/// Section A: the Mac's name and how it is reached.
fn connection(ui: &mut Ui, s: &StreamView, mac: &str) {
    ui.heading(mac);
    let peer = s.session.peer;
    let line = match &s.route_desc {
        RouteDesc::Home => {
            texts::connected_at_home(&peer.map(|p| p.ip().to_string()).unwrap_or_default())
        }
        RouteDesc::Remote { via } => texts::connected_remotely(
            &peer.map(|p| p.to_string()).unwrap_or_default(),
            via.as_deref(),
        ),
        RouteDesc::Unpinned | RouteDesc::Plain => texts::CONNECTED_UNPINNED.to_string(),
    };
    widgets::muted(ui, &line);
    ui.add_space(12.0);
}

/// Section D: the keymap, and the local input switches when the Mac is this computer.
fn this_computer(ui: &mut Ui, cx: &mut Cx, s: &StreamView) {
    ui.label(egui::RichText::new(texts::THIS_COMPUTER).heading());
    ui.label(texts::KEYMAP);
    widgets::keymap_radios(ui, cx.prefs);
    if s.session.local_peer {
        ui.add_space(6.0);
        ui.checkbox(&mut cx.prefs.allow_local_pointer, texts::SEND_POINTER);
        ui.checkbox(&mut cx.prefs.allow_local_keyboard, texts::SEND_KEYBOARD);
        ui.label(
            egui::RichText::new(texts::LOCAL_KEYBOARD_WARNING)
                .small()
                .color(theme::ERROR),
        );
    }
}
