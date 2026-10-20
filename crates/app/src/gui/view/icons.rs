//! Painted icons. egui's default fonts have no glyph for gear, full screen, close and the like,
//! so every icon here is drawn from lines, circles and rectangles (never a symbol character).

use super::theme;
use egui::{pos2, vec2, Color32, Painter, Rect, Response, Sense, Stroke, Ui, WidgetInfo};
use egui::{CornerRadius, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Gear,
    FullScreen,
    LeaveFullScreen,
    Disconnect,
    Desktop,
    More,
    Close,
    ArrowLeft,
    ArrowRight,
}

/// The side of an icon button, at least.
pub const BUTTON_SIZE: f32 = 28.0;

/// Paints `icon` inside `rect` (a square, about 28 pt) in `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = Stroke::new(1.6, color);
    let u = rect.width().min(rect.height()) / 28.0;
    let p = |x: f32, y: f32| pos2(c.x + x * u, c.y + y * u);
    match icon {
        Icon::Gear => {
            painter.circle_stroke(c, 6.5 * u, s);
            painter.circle_stroke(c, 2.5 * u, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (sn, cs) = a.sin_cos();
                painter.line_segment([p(cs * 8.0, sn * 8.0), p(cs * 10.5, sn * 10.5)], s);
            }
        }
        Icon::FullScreen | Icon::LeaveFullScreen => {
            let (near, far) = if icon == Icon::FullScreen {
                (9.0, 4.0)
            } else {
                (4.0, 9.0)
            };
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = p(sx * near, sy * near);
                painter.line_segment([corner, p(sx * far, sy * near)], s);
                painter.line_segment([corner, p(sx * near, sy * far)], s);
            }
        }
        Icon::Disconnect => {
            painter.circle_stroke(c, 9.0 * u, s);
            painter.line_segment([p(-4.0, -4.0), p(4.0, 4.0)], s);
            painter.line_segment([p(4.0, -4.0), p(-4.0, 4.0)], s);
        }
        Icon::Desktop => {
            let r = Rect::from_center_size(p(0.0, -2.0), vec2(20.0 * u, 13.0 * u));
            painter.rect_stroke(r, CornerRadius::same(2), s, egui::StrokeKind::Middle);
            painter.line_segment([p(0.0, 5.0), p(0.0, 9.0)], s);
            painter.line_segment([p(-5.0, 9.0), p(5.0, 9.0)], s);
        }
        Icon::More => {
            for x in [-6.0, 0.0, 6.0] {
                painter.circle_filled(p(x, 0.0), 1.9 * u, color);
            }
        }
        Icon::Close => {
            painter.line_segment([p(-5.0, -5.0), p(5.0, 5.0)], s);
            painter.line_segment([p(5.0, -5.0), p(-5.0, 5.0)], s);
        }
        Icon::ArrowLeft => {
            painter.line_segment([p(2.5, -6.0), p(-2.5, 0.0)], s);
            painter.line_segment([p(-2.5, 0.0), p(2.5, 6.0)], s);
        }
        Icon::ArrowRight => {
            painter.line_segment([p(-2.5, -6.0), p(2.5, 0.0)], s);
            painter.line_segment([p(2.5, 0.0), p(-2.5, 6.0)], s);
        }
    }
}

/// Paints the button for `icon` into an already allocated `response`.
fn paint_button(ui: &Ui, response: &Response, icon: Icon, selected: bool) {
    let rect = response.rect;
    let fill = if selected || response.is_pointer_button_down_on() {
        theme::OPEN
    } else if response.hovered() || response.has_focus() {
        theme::CONTROL_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), fill);
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let color = if selected { theme::ACCENT } else { theme::TEXT };
    paint(painter, rect, icon, color);
}

/// An icon-only button, 28x28 pt, with no hover text and no accessible name (for the bar's
/// scroll arrows, which have no copy).
pub fn plain_button(ui: &mut Ui, icon: Icon, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON_SIZE, BUTTON_SIZE), Sense::click());
    if ui.is_rect_visible(rect) {
        paint_button(ui, &response, icon, selected);
    }
    response
}

/// An icon-only button, 28x28 pt, with `label` as its hover text and accessible name.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let response = plain_button(ui, icon, selected).on_hover_text(label);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    response
}
