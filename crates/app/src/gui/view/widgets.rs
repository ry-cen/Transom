//! Small shared widgets: labelled text fields, the code field, buttons and card frames.

use super::theme;
use egui::{Align, Layout, Response, RichText, Ui};
use std::time::Instant;

/// Digits only, at most 12, grouped in fours with a space (the code's display form).
pub fn group_code(text: &str) -> String {
    let digits: Vec<char> = text
        .chars()
        .filter(char::is_ascii_digit)
        .take(transom_proto::pairing::CODE_LEN)
        .collect();
    digits
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn move_cursor_to_end(ui: &Ui, id: egui::Id, text: &str) {
    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(text.chars().count()),
        )));
        st.store(ui.ctx(), id);
    }
}

/// A caption, then a single-line text field labelled by it (for the accessibility tree and for
/// tests). `hint` is the placeholder.
pub fn text_field(
    ui: &mut Ui,
    label: &str,
    text: &mut String,
    hint: &str,
    enabled: bool,
) -> Response {
    let l = ui.label(RichText::new(label).color(theme::MUTED));
    ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(l.id)
}

/// The pairing code field (spec 3.4): monospace, digits only, shown in groups of four. Typing
/// regroups the text and keeps the cursor at the end.
pub fn code_field(ui: &mut Ui, label: &str, text: &mut String, enabled: bool) -> Response {
    let l = ui.label(RichText::new(label).color(theme::MUTED));
    let response = ui
        .add_enabled(
            enabled,
            egui::TextEdit::singleline(text)
                .font(egui::TextStyle::Monospace)
                .hint_text(crate::gui::texts::CODE_PLACEHOLDER)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(l.id);
    if response.changed() {
        let grouped = group_code(text);
        if grouped != *text {
            *text = grouped;
            move_cursor_to_end(ui, response.id, text);
        }
    }
    response
}

/// Enter was pressed in a text field that just lost focus.
pub fn entered(ui: &Ui, response: &Response) -> bool {
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A problem line in the error colour.
pub fn problem(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::ERROR));
}

/// Secondary text.
pub fn muted(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED).small());
}

/// A button for the destructive action.
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(theme::ERROR)))
}

/// A button with the accent colour, for the main action.
pub fn primary_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(theme::PANEL)).fill(theme::ACCENT),
    )
}

/// Lays `add` out as a right-aligned row of buttons.
pub fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add)
        .inner
}

/// The frame of a floating card.
pub fn card_frame(ui: &Ui) -> egui::Frame {
    egui::Frame::popup(ui.style())
        .fill(theme::PANEL)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(18))
}

/// The two modifier-layout radios (PREF9, PREF10). A click always sets the preference, even on
/// the radio that is already selected (the default depends on the OS).
pub fn keymap_radios(ui: &mut Ui, prefs: &mut crate::gui::model::Prefs) {
    use crate::gui::texts;
    use transom_input::KeymapMode;
    let mode = prefs.keymap_mode();
    if ui.radio(mode == KeymapMode::Pc, texts::KEYMAP_PC).clicked() {
        prefs.keymap = Some("pc".into());
    }
    if ui
        .radio(mode == KeymapMode::Mac, texts::KEYMAP_MAC)
        .clicked()
    {
        prefs.keymap = Some("mac".into());
    }
}

/// A modal dialog `width` wide; true when it asks to close (a click outside; Esc is consumed
/// earlier, in `view::draw`).
pub fn modal(ctx: &egui::Context, id: &str, width: f32, content: impl FnOnce(&mut Ui)) -> bool {
    egui::Modal::new(egui::Id::unique(id))
        .show(ctx, |ui| {
            ui.set_width(width);
            content(ui);
        })
        .should_close()
}

/// True once per `key`: the first frame a dialog or card shows. `key` None (nothing shown)
/// forgets the last one.
pub fn first_time(ctx: &egui::Context, slot: &str, key: Option<&str>) -> bool {
    let id = egui::Id::unique(("first-time", slot));
    let last: Option<String> = ctx.data(|d| d.get_temp(id));
    match key {
        Some(k) => {
            let first = last.as_deref() != Some(k);
            if first {
                ctx.data_mut(|d| d.insert_temp(id, k.to_string()));
            }
            first
        }
        None => {
            if last.is_some() {
                ctx.data_mut(|d| d.remove_temp::<String>(id));
            }
            false
        }
    }
}

/// A text field whose edit commits when it loses focus (Enter or Tab): returns the trimmed text
/// once, after it was edited. Without an edit, focus changes commit nothing.
pub fn commit_on_blur(ui: &Ui, response: &Response, text: &str) -> Option<String> {
    let id = response.id.with("edited");
    if response.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(id, true));
    }
    if response.lost_focus() && ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        ui.ctx().data_mut(|d| d.remove_temp::<bool>(id));
        return Some(text.trim().to_string());
    }
    None
}

/// Seconds left until `at`, rounded up (the reconnect countdown).
pub fn secs_until(at: Instant, now: Instant) -> u64 {
    at.saturating_duration_since(now).as_secs_f64().ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_groups_in_fours() {
        assert_eq!(group_code("482913557208"), "4829 1355 7208");
        assert_eq!(group_code("4829-1355 7208 99"), "4829 1355 7208");
        assert_eq!(group_code("48291"), "4829 1");
        assert_eq!(group_code("ab"), "");
    }
}
