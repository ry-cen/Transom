//! The Mac Settings panel (docs/ui-spec.md 3.8): A (the connection), B (streaming, through the
//! pick ledger), C (Direct Wireless), D (this computer) and E (Disconnect).

use super::stream_bar::BAR_HEIGHT;
use super::{theme, widgets, Cx};
use crate::gui::model::{Msg, RouteDesc, StreamView};
use crate::gui::texts;
use egui::{Align2, Area, Frame, Id, Margin, Order, RichText, Ui};
use std::time::Instant;
use transom_proto::host_settings::{HostSettingsChange, SettingsField, QUALITY_PRESETS};

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
                            streaming(ui, cx, s, &mac);
                            mac_connection(ui, cx, s, &mac);
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

/// A spinner beside a row whose pick has been pending for a while.
fn spinner(ui: &mut Ui, s: &StreamView, f: SettingsField, now: Instant) {
    if s.pick_spinner(f, now) {
        ui.add(egui::Spinner::new().size(12.0));
    }
}

/// Section B: the Mac's streaming settings, shown through the pick ledger.
fn streaming(ui: &mut Ui, cx: &mut Cx, s: &StreamView, mac: &str) {
    ui.label(RichText::new(texts::STREAM_HEADING).heading());
    if !s.session.is_connected() {
        widgets::muted(ui, texts::NOT_STREAMING);
        ui.add_space(12.0);
        return;
    }
    let (Some(h), Some(shown)) = (s.session.settings.host(), s.session.settings.displayed()) else {
        if s.settings_overdue(cx.now) {
            widgets::muted(ui, &texts::update_sill_for_settings(mac));
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                widgets::muted(ui, texts::LOADING_SETTINGS);
            });
        }
        ui.add_space(12.0);
        return;
    };
    let now = cx.now;
    let pick = |cx: &mut Cx, change: HostSettingsChange| cx.out.push(Msg::ChangeSettings(change));

    ui.horizontal(|ui| {
        egui::ComboBox::from_label(texts::QUALITY)
            .selected_text(texts::quality_title(shown.bitrate))
            .show_ui(ui, |ui| {
                for &(bps, _) in QUALITY_PRESETS.iter() {
                    if ui
                        .selectable_label(shown.bitrate == bps, texts::quality_title(bps))
                        .clicked()
                    {
                        pick(
                            cx,
                            HostSettingsChange {
                                bitrate: Some(bps),
                                ..Default::default()
                            },
                        );
                    }
                }
            });
        spinner(ui, s, SettingsField::Bitrate, now);
    });

    ui.horizontal(|ui| {
        ui.label(texts::RESOLUTION);
        if ui
            .radio(shown.capture_scale == 2.0, texts::RETINA)
            .clicked()
        {
            pick(
                cx,
                HostSettingsChange {
                    capture_scale: Some(2.0),
                    ..Default::default()
                },
            );
        }
        if ui
            .radio(shown.capture_scale == 1.0, texts::STANDARD)
            .clicked()
        {
            pick(
                cx,
                HostSettingsChange {
                    capture_scale: Some(1.0),
                    ..Default::default()
                },
            );
        }
        spinner(ui, s, SettingsField::CaptureScale, now);
    });

    ui.horizontal(|ui| {
        ui.label(texts::FRAME_RATE);
        for (fps, text) in [(60, texts::FPS_60), (120, texts::FPS_120)] {
            if ui.radio(shown.max_fps == fps, text).clicked() {
                pick(
                    cx,
                    HostSettingsChange {
                        max_fps: Some(fps),
                        ..Default::default()
                    },
                );
            }
        }
        spinner(ui, s, SettingsField::MaxFps, now);
    });

    ui.horizontal(|ui| {
        let mut v = shown.prioritize_speed;
        if ui.checkbox(&mut v, texts::PRIORITIZE_SPEED).changed() {
            pick(
                cx,
                HostSettingsChange {
                    prioritize_speed: Some(v),
                    ..Default::default()
                },
            );
        }
        spinner(ui, s, SettingsField::PrioritizeSpeed, now);
    });
    widgets::muted(ui, &texts::prioritize_speed_note(mac));

    ui.horizontal(|ui| {
        let enabled = h.virtual_display_available || shown.virtual_display;
        let mut v = shown.virtual_display;
        if ui
            .add_enabled(enabled, egui::Checkbox::new(&mut v, texts::VIRTUAL_DISPLAY))
            .changed()
        {
            pick(
                cx,
                HostSettingsChange {
                    virtual_display: Some(v),
                    ..Default::default()
                },
            );
        }
        spinner(ui, s, SettingsField::VirtualDisplay, now);
    });
    match &h.virtual_display_note {
        Some(n) => {
            ui.label(
                RichText::new(texts::mac_note(n))
                    .small()
                    .color(theme::ERROR),
            );
        }
        None => widgets::muted(ui, &texts::virtual_display_note(mac)),
    }

    widgets::muted(ui, &texts::applies_to_every_device(mac));
    widgets::muted(
        ui,
        &match &h.stream {
            Some(r) => texts::now_streaming(r.width, r.height, r.fps, r.mbps),
            None => texts::NOT_STREAMING.to_string(),
        },
    );
    if h.software_encoder {
        widgets::problem(ui, &texts::software_encoder(mac));
    }
    if !h.persistent {
        widgets::muted(ui, texts::NOT_PERSISTENT);
    }
    if h.away.as_ref().is_some_and(|a| a.this_connection_away) {
        widgets::muted(ui, &texts::counts_as_away(mac));
    }
    ui.add_space(12.0);
}

/// Section C: Direct Wireless, only when the Mac reports it.
fn mac_connection(ui: &mut Ui, cx: &mut Cx, s: &StreamView, mac: &str) {
    let Some(mut v) = s
        .session
        .settings
        .displayed()
        .and_then(|d| d.direct_wireless)
    else {
        return;
    };
    ui.separator();
    ui.horizontal(|ui| {
        if ui.checkbox(&mut v, texts::DIRECT_WIRELESS).changed() {
            cx.out.push(Msg::ChangeSettings(HostSettingsChange {
                direct_wireless: Some(v),
                ..Default::default()
            }));
        }
        spinner(ui, s, SettingsField::DirectWireless, cx.now);
    });
    widgets::muted(ui, &texts::direct_wireless_note(mac));
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
