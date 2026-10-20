//! The Macs screen (docs/ui-spec.md sections 3.2 to 3.4): title, status, the two lists, the
//! pairing or Add card in place of the lists, and the footer.

use super::icons::{self, Icon};
use super::{add, pair, theme, widgets, Cx};
use crate::gui::model::{
    BannerAction, Card, Dialog, MacRow, Msg, PrefsTab, RowKey, RowMenuItem, RowWord,
};
use crate::gui::texts;
use egui::{pos2, vec2, Align2, FontId, Rect, RichText, Role, Sense, Ui, WidgetInfo};

/// The width of the column (spec 3.3).
pub const COLUMN_WIDTH: f32 = 440.0;
/// The height of one row.
pub const ROW_HEIGHT: f32 = 52.0;

/// Draws the whole Macs screen in a central panel.
pub fn draw(ui: &mut Ui, cx: &mut Cx, card: Option<&Card>) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if ui.button(texts::PREFERENCES).clicked() {
                cx.out.push(Msg::OpenDialog(Dialog::Preferences {
                    tab: PrefsTab::General,
                }));
            }
            if cx.model.fullscreen && ui.button(texts::LEAVE_FULL_SCREEN).clicked() {
                cx.out.push(Msg::ToggleFullscreen);
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let side = ((ui.available_width() - COLUMN_WIDTH) / 2.0).max(12.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(COLUMN_WIDTH);
                        column(ui, cx, card);
                    });
                });
            });
    });
}

fn column(ui: &mut Ui, cx: &mut Cx, card: Option<&Card>) {
    banner(ui, cx);
    ui.add_space(8.0);
    ui.heading(texts::MACS_TITLE);
    if let Some(card) = card {
        match card {
            Card::Pair(c) => pair::draw(ui, cx, c),
            Card::Add(c) => add::draw(ui, cx, c),
            Card::OpenDoor { found } => pair::draw_open_door(ui, cx, found),
        }
        return;
    }
    let model = cx.model;
    if model.saved.is_empty() {
        ui.label(RichText::new(texts::WELCOME).color(theme::MUTED));
    }
    if !model.status.is_empty() {
        ui.label(&model.status);
    } else if !model.ever_found {
        ui.label(texts::LOOKING);
    }
    if !model.ever_found && cx.now >= model.started_at + crate::gui::model::HINT_AFTER {
        ui.label(RichText::new(texts::NO_MAC_HINT).color(theme::MUTED));
    }
    section(ui, cx, texts::MY_MACS, &model.rows_mine);
    section(ui, cx, texts::OTHER_MACS, &model.rows_other);
    ui.add_space(4.0);
    let add = egui::Button::new(RichText::new(texts::ADD_A_MAC).color(theme::ACCENT)).frame(false);
    if ui.add(add).clicked() {
        cx.out.push(Msg::OpenAdd);
    }
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(texts::NEEDS_SILL).color(theme::MUTED));
        if ui.link(texts::GET_SILL).clicked() {
            cx.out.push(Msg::OpenUrl(texts::GET_SILL_URL.into()));
        }
        if ui.link(texts::ABOUT_TRANSOM).clicked() {
            cx.out.push(Msg::OpenDialog(Dialog::About));
        }
    });
}

fn banner(ui: &mut Ui, cx: &mut Cx) {
    let Some(b) = &cx.model.banner else { return };
    widgets::card_frame(ui)
        .fill(theme::OPEN)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(COLUMN_WIDTH - 24.0);
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(&b.text).wrap());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if icons::icon_button(ui, Icon::Close, texts::DISMISS, false).clicked() {
                        cx.out.push(Msg::DismissBanner);
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                for a in &b.actions {
                    let text = match a {
                        BannerAction::ConnectAtHome => texts::CONNECT_AT_HOME,
                        BannerAction::PairAgain => texts::PAIR_AGAIN,
                        BannerAction::Pair => texts::PAIR_BUTTON,
                        BannerAction::ForgetAndPair => texts::FORGET_AND_PAIR,
                        BannerAction::GetSill => texts::GET_SILL,
                    };
                    if ui.button(text).clicked() {
                        cx.out.push(Msg::BannerAction(*a));
                    }
                }
            });
        });
}

fn section(ui: &mut Ui, cx: &mut Cx, title: &str, rows: &[MacRow]) {
    if rows.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(RichText::new(title).small().color(theme::MUTED));
    for row in rows {
        row_ui(ui, cx, row);
    }
}

fn word_color(w: RowWord) -> egui::Color32 {
    match w {
        RowWord::Home | RowWord::Remote => theme::ACCENT,
        RowWord::NotFound => theme::MUTED,
        RowWord::NotPaired => theme::TEXT,
        RowWord::UpdateSill | RowWord::Unencrypted => theme::ERROR,
    }
}

/// One row: painted by hand so it is one accessible button, "‹name›, ‹word›".
fn row_ui(ui: &mut Ui, cx: &mut Cx, row: &MacRow) {
    let word = texts::row_word(&row.word);
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(COLUMN_WIDTH, ROW_HEIGHT), Sense::click());
    let accessible = format!("{}, {word}", row.name);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &accessible));
    if row.word == RowWord::UpdateSill {
        response = response.on_hover_text(texts::sill_too_old(&row.name));
    }
    let saved = matches!(row.key, RowKey::Saved(_));
    let painter = ui.painter();
    let fill = if response.hovered() || response.has_focus() {
        theme::CONTROL_HOVER
    } else {
        theme::CONTROL
    };
    painter.rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill);
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(theme::RADIUS),
            egui::Stroke::new(1.5, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let name_y = if row.detail.is_empty() {
        rect.center().y
    } else {
        rect.top() + 18.0
    };
    painter.text(
        pos2(rect.left() + 14.0, name_y),
        Align2::LEFT_CENTER,
        &row.name,
        FontId::proportional(15.0),
        theme::TEXT,
    );
    if !row.detail.is_empty() {
        painter.text(
            pos2(rect.left() + 14.0, rect.top() + 36.0),
            Align2::LEFT_CENTER,
            &row.detail,
            FontId::proportional(12.0),
            theme::MUTED,
        );
    }
    let right = rect.right() - if saved { 48.0 } else { 14.0 };
    painter.text(
        pos2(right, rect.center().y),
        Align2::RIGHT_CENTER,
        word,
        FontId::proportional(14.0),
        word_color(row.word),
    );
    if response.clicked() {
        cx.out.push(Msg::ClickRow {
            key: row.key.clone(),
        });
    }
    if saved {
        let more_rect = Rect::from_center_size(
            pos2(rect.right() - 24.0, rect.center().y),
            vec2(icons::BUTTON_SIZE, icons::BUTTON_SIZE),
        );
        let more = ui.put(more_rect, |ui: &mut Ui| {
            icons::icon_button(ui, Icon::More, &texts::row_menu_label(&row.name), false)
        });
        egui::Popup::menu(&more).show(|ui| menu_items(ui, cx, row));
        egui::Popup::context_menu(&response).show(|ui| menu_items(ui, cx, row));
    }
}

fn menu_items(ui: &mut Ui, cx: &mut Cx, row: &MacRow) {
    let mut pick = |ui: &mut Ui, text: &str, item: RowMenuItem| {
        if ui.button(text).clicked() {
            cx.out.push(Msg::RowMenu {
                key: row.key.clone(),
                item,
            });
        }
    };
    pick(ui, texts::MENU_CONNECT, RowMenuItem::Connect);
    if row.found.is_some() {
        pick(ui, texts::MENU_CONNECT_HOME, RowMenuItem::ConnectHome);
    }
    if row.has_remote {
        pick(ui, texts::MENU_CONNECT_REMOTE, RowMenuItem::ConnectRemote);
    }
    pick(ui, texts::MENU_DETAILS, RowMenuItem::Details);
    pick(ui, &texts::menu_forget(&row.name), RowMenuItem::Forget);
}
