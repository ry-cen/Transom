//! The Stream screen's auto-hiding bar and its handle (docs/ui-spec.md section 3.6). Only
//! `egui::Area`s: the video behind must stay "not over egui" so pointer forwarding works.

use super::icons::{self, Icon};
use super::{theme, Cx, Image, ImageLookup};
use crate::gui::model::{Dialog, Msg, PrefsTab, StreamPanel, StreamView};
use crate::gui::texts;
use egui::{
    pos2, vec2, Align2, Area, Color32, FontId, Frame, Id, Margin, Order, Rect, RichText, Role,
    Sense, Ui, WidgetInfo,
};
use transom_proto::messages::{StreamSource, WindowInfo};
use transom_proto::safetext::{label, LABEL_LIMIT};

/// The bar's height.
pub const BAR_HEIGHT: f32 = 76.0;
/// A window tile's size.
pub const TILE_W: f32 = 96.0;
pub const TILE_H: f32 = 60.0;
const HANDLE_W: f32 = 160.0;
const HANDLE_H: f32 = 24.0;

/// Draws the handle, and the bar when it is shown, and reports the pointer's hover.
pub fn draw(ui: &mut Ui, cx: &mut Cx, s: &StreamView) {
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let visible = s.bar_visible(cx.now);
    let mut hover_rects = Vec::new();

    let handle_top = if visible { BAR_HEIGHT } else { 0.0 };
    let handle_rect = Rect::from_min_size(
        pos2(
            screen.center().x - HANDLE_W / 2.0,
            screen.top() + handle_top,
        ),
        vec2(HANDLE_W, HANDLE_H),
    );
    hover_rects.push(handle_rect);

    if visible {
        let bar_rect = Rect::from_min_size(screen.min, vec2(screen.width(), BAR_HEIGHT));
        hover_rects.push(bar_rect);
        let mut msgs = Vec::new();
        Area::new(Id::unique("bar"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(&ctx, |ui| {
                Frame::new()
                    .fill(theme::BAR_BG)
                    .inner_margin(Margin::symmetric(12, 2))
                    .show(ui, |ui| {
                        ui.set_width((screen.width() - 24.0).max(0.0));
                        ui.set_height(BAR_HEIGHT - 4.0);
                        contents(ui, cx, s, &mut msgs);
                    });
            });
        cx.out.extend(msgs);
    }

    Area::new(Id::unique("handle"))
        .order(Order::Foreground)
        .fixed_pos(handle_rect.min)
        .show(&ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(handle_rect.size(), Sense::click());
            let text = if s.bar.pinned {
                texts::HIDE_BAR
            } else {
                texts::SHOW_BAR
            };
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, text));
            let pill = Rect::from_center_size(rect.center_top() + vec2(0.0, 8.0), vec2(120.0, 5.0));
            let alpha = if response.hovered() { 0.7 } else { 0.4 };
            ui.painter().rect_filled(
                pill,
                egui::CornerRadius::same(3),
                Color32::WHITE.gamma_multiply(alpha),
            );
            if response.clicked() {
                cx.out.push(Msg::ToggleBar);
            }
        });

    let over = ctx
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| hover_rects.iter().any(|r| r.contains(p)));
    if over && s.bar.hover_since.is_none() {
        cx.out.push(Msg::BarHover(true));
    } else if !over && s.bar.hover_since.is_some() {
        cx.out.push(Msg::BarHover(false));
    }
}

fn contents(ui: &mut Ui, cx: &Cx, s: &StreamView, out: &mut Vec<Msg>) {
    let mac = s.session.mac_name.as_str();
    let mut left_out = Vec::new();
    let mut right_out = Vec::new();
    egui::containers::Sides::new()
        .height(BAR_HEIGHT - 4.0)
        .shrink_left()
        .show(
            ui,
            |ui| {
                tiles(ui, s, cx.images, &mut left_out);
            },
            |ui| {
                if icons::icon_button(ui, Icon::Disconnect, &texts::disconnect_from(mac), false)
                    .clicked()
                {
                    right_out.push(Msg::Disconnect);
                }
                let (icon, text) = if cx.model.fullscreen {
                    (Icon::LeaveFullScreen, texts::LEAVE_FULL_SCREEN)
                } else {
                    (Icon::FullScreen, texts::FULL_SCREEN)
                };
                if icons::icon_button(ui, icon, text, false).clicked() {
                    right_out.push(Msg::ToggleFullscreen);
                }
                let open = s.panel == Some(StreamPanel::MacSettings);
                if icons::icon_button(ui, Icon::Gear, texts::MAC_SETTINGS, open).clicked() {
                    right_out.push(if open {
                        Msg::ClosePanel
                    } else {
                        Msg::OpenPanel(StreamPanel::MacSettings)
                    });
                }
                let prefs = &cx.model.prefs;
                if s.session.local_peer
                    && (!prefs.allow_local_keyboard || !prefs.allow_local_pointer)
                {
                    view_only_chip(ui, &mut right_out);
                }
                if prefs.show_stats {
                    let pill = texts::stats_pill(s.session.rtt_ms, s.session.fps);
                    let tip = texts::stats_tooltip(s.session.rtt_ms, s.session.fps, &s.route_desc);
                    ui.add(
                        egui::Label::new(RichText::new(pill).color(theme::MUTED))
                            .sense(Sense::hover()),
                    )
                    .on_hover_text(tip);
                }
                let sel = matches!(s.session.active, StreamSource::Desktop {});
                if desktop_button(ui, sel).clicked() {
                    right_out.push(Msg::SelectDesktop);
                }
            },
        );
    out.extend(left_out);
    out.extend(right_out);
}

/// The "View only" chip, with its explanation and the link to Preferences > Input.
fn view_only_chip(ui: &mut Ui, out: &mut Vec<Msg>) {
    let response =
        ui.add(egui::Button::new(RichText::new(texts::VIEW_ONLY).small()).fill(theme::OPEN));
    let response = response.on_hover_text(texts::VIEW_ONLY_WHY);
    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_max_width(280.0);
            ui.label(texts::VIEW_ONLY_WHY);
            if ui.link(texts::INPUT_PREFERENCES).clicked() {
                out.push(Msg::OpenDialog(Dialog::Preferences {
                    tab: PrefsTab::Input,
                }));
            }
        });
}

/// The Desktop button: the painted desktop icon and BAR2, accessibly named BAR3.
fn desktop_button(ui: &mut Ui, selected: bool) -> egui::Response {
    let color = if selected { theme::ACCENT } else { theme::TEXT };
    let galley = ui.painter().layout_no_wrap(
        texts::DESKTOP.to_string(),
        FontId::proportional(14.0),
        color,
    );
    let size = vec2(30.0 + galley.size().x + 10.0, icons::BUTTON_SIZE);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, texts::SHOW_DESKTOP));
    let fill = if selected {
        theme::OPEN
    } else if response.hovered() || response.has_focus() {
        theme::CONTROL_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(8), fill);
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            egui::Stroke::new(1.5, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    icons::paint(
        painter,
        Rect::from_min_size(rect.min, vec2(28.0, 28.0)),
        Icon::Desktop,
        color,
    );
    painter.galley(
        pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}

/// The scrolling row of window tiles, with painted arrows when they overflow.
fn tiles(ui: &mut Ui, s: &StreamView, images: &dyn ImageLookup, out: &mut Vec<Msg>) {
    let ctx = ui.ctx().clone();
    let state_id = Id::unique("bar-scroll");
    let target_id = Id::unique("bar-scroll-to");
    let (offset, overflow): (f32, bool) = ctx.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let target: Option<f32> = ctx.data_mut(|d| d.remove_temp(target_id));
    let step = TILE_W + 8.0;
    if overflow && icons::plain_button(ui, Icon::ArrowLeft, false).clicked() {
        ctx.data_mut(|d| d.insert_temp(target_id, (offset - step).max(0.0)));
        ctx.request_repaint();
    }
    let mut area = egui::ScrollArea::horizontal()
        .id_salt("bar-tiles")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([true, false]);
    if let Some(t) = target {
        area = area.horizontal_scroll_offset(t);
    }
    let active = match s.session.active {
        StreamSource::Window { id } => Some(id),
        _ => None,
    };
    let result = area.show(ui, |ui| {
        ui.add_space(6.0);
        ui.set_min_height(BAR_HEIGHT - 4.0);
        for w in &s.session.windows {
            tile(ui, w, active == Some(w.id), images, out);
        }
    });
    let overflows = result.content_size.x > result.inner_rect.width() + 0.5;
    ctx.data_mut(|d| d.insert_temp(state_id, (result.state.clamped_offset().x, overflows)));
    if overflow && icons::plain_button(ui, Icon::ArrowRight, false).clicked() {
        ctx.data_mut(|d| d.insert_temp(target_id, offset + step));
        ctx.request_repaint();
    }
}

fn fit(into: Rect, size: egui::Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return into;
    }
    let scale = (into.width() / size.x).min(into.height() / size.y);
    Rect::from_center_size(into.center(), size * scale)
}

fn paint_image(painter: &egui::Painter, image: Image, rect: Rect) {
    painter.image(
        image.id,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn tile(ui: &mut Ui, w: &WindowInfo, active: bool, images: &dyn ImageLookup, out: &mut Vec<Msg>) {
    let (rect, response) = ui.allocate_exact_size(vec2(TILE_W, TILE_H), Sense::click());
    let text = texts::tile_label(&w.app_name, &w.title, active);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &text));
    let response = response.on_hover_text(&text);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let r = egui::CornerRadius::same(8);
        painter.rect_filled(rect, r, theme::TILE_BG);
        let app = label(&w.app_name, LABEL_LIMIT);
        match images.thumbnail(w.id) {
            Some(img) => paint_image(painter, img, fit(rect.shrink(2.0), img.size)),
            None => match images.icon(&w.bundle_id) {
                Some(img) => paint_image(
                    painter,
                    img,
                    Rect::from_center_size(rect.center(), vec2(32.0, 32.0)),
                ),
                None => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        initials(&app),
                        FontId::proportional(16.0),
                        theme::MUTED,
                    );
                }
            },
        }
        if response.hovered() || response.has_focus() {
            painter.rect_stroke(
                rect,
                r,
                egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }
        if active {
            painter.rect_stroke(
                rect,
                r,
                egui::Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let badge = Rect::from_min_size(
            pos2(rect.left() - 4.0, rect.bottom() + 4.0 - 20.0),
            vec2(20.0, 20.0),
        );
        match images.icon(&w.bundle_id) {
            Some(img) => paint_image(painter, img, badge),
            None => {
                painter.rect_filled(badge, egui::CornerRadius::same(5), theme::BADGE_BG);
                painter.text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    initials(&app),
                    FontId::proportional(9.0),
                    theme::TEXT,
                );
            }
        }
    }
    if response.clicked() {
        out.push(Msg::SelectWindow(w.id));
    }
}

/// The first two letters of an app name.
fn initials(app: &str) -> String {
    app.chars().take(2).collect()
}
