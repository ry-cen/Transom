//! The look (docs/ui-spec.md section 3.1): a dark palette with one accent.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// Window and panel background.
pub const PANEL: Color32 = Color32::from_rgb(0x1F, 0x21, 0x26);
/// Buttons and text fields.
pub const CONTROL: Color32 = Color32::from_rgb(0x23, 0x26, 0x2C);
/// A control under the pointer.
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x2B, 0x2F, 0x37);
/// A pressed or open control, and a selected row.
pub const OPEN: Color32 = Color32::from_rgb(0x2C, 0x35, 0x50);
/// Selection, focus ring and links.
pub const ACCENT: Color32 = Color32::from_rgb(0x9B, 0xB7, 0xFF);
/// Body text.
pub const TEXT: Color32 = Color32::from_rgb(0xEE, 0xF0, 0xF3);
/// Secondary text.
pub const MUTED: Color32 = Color32::from_rgb(0xA4, 0xAB, 0xB5);
/// Problems and the destructive button.
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);
/// A window tile's background in the bar.
pub const TILE_BG: Color32 = Color32::from_rgb(0x26, 0x28, 0x2D);
/// The badge's background when the app has no icon.
pub const BADGE_BG: Color32 = Color32::from_rgb(0x3A, 0x3D, 0x44);
/// The bar: #16181C at 94 % alpha.
pub const BAR_BG: Color32 = Color32::from_rgba_premultiplied(0x15, 0x16, 0x1A, 240);
/// The corner radius of controls.
pub const RADIUS: u8 = 10;

/// Installs the visuals and text styles on `ctx` and turns egui's own keyboard zoom off (the
/// app keeps its own clamped, remembered zoom, spec 6.1).
pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(MUTED);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.faint_bg_color = CONTROL;
    v.extreme_bg_color = Color32::from_rgb(0x18, 0x1A, 0x1E);
    v.hyperlink_color = ACCENT;
    v.error_fg_color = ERROR;
    v.selection.bg_fill = OPEN;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let r = CornerRadius::same(RADIUS);
    for (w, fill) in [
        (&mut v.widgets.inactive, CONTROL),
        (&mut v.widgets.hovered, CONTROL_HOVER),
        (&mut v.widgets.active, OPEN),
        (&mut v.widgets.open, OPEN),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = r;
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.5));
    v.widgets.noninteractive.corner_radius = r;
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.text_styles
            .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
        s.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
        s.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        s.text_styles.insert(
            TextStyle::Heading,
            FontId::new(17.0, FontFamily::Proportional),
        );
        s.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(20.0, FontFamily::Monospace),
        );
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}
