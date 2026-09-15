//! The notice at the bottom of the layer, with an optional undo.

use std::time::{Duration, Instant};

use egui::{Align2, CornerRadius, CursorIcon, FontId, Id, Rect, Sense, Ui, pos2, vec2};

use crate::card::{self, Kind};
use crate::rich_text;
use crate::store::ReviewSnapshot;
use crate::theme;

use super::EbbApp;
use super::paint::{PANEL_APPEAR, PANEL_FADE_OUT, glass_panel, panel_ease};

const TOAST_FOR: Duration = Duration::from_secs(6);

#[derive(Clone)]
pub(super) enum Undo {
    Unarchive(i64),
    Restore(i64),
    /// Back to the kind a card had.
    Kind(i64, Kind),
    /// Back on the layer as it was before "Позже".
    Snooze(ReviewSnapshot),
    /// A card's title and text as they were before a tag change.
    Text(i64, String, String),
}

pub(super) struct Toast {
    text: String,
    undo: Option<Undo>,
    at: Instant,
}

impl Toast {
    pub(super) fn new(text: impl Into<String>, undo: Option<Undo>) -> Self {
        Self { text: text.into(), undo, at: Instant::now() }
    }
}

impl EbbApp {
    /// Bottom-center notice with an optional undo, dismissed after TOAST_FOR.
    pub(super) fn toast_ui(&mut self, ui: &mut Ui) {
        let Some(toast) = &self.toast else { return };
        let left = TOAST_FOR.saturating_sub(toast.at.elapsed());
        if left.is_zero() {
            self.toast = None;
            return;
        }
        // Rises in over PANEL_APPEAR, fades out over its last PANEL_FADE_OUT.
        let appear = panel_ease(toast.at.elapsed().as_secs_f32() / PANEL_APPEAR.as_secs_f32());
        let fade_out = (left.as_secs_f32() / PANEL_FADE_OUT.as_secs_f32()).min(1.0);
        if appear < 1.0 || fade_out < 1.0 {
            ui.ctx().request_repaint();
        } else {
            ui.ctx().request_repaint_after(left.saturating_sub(PANEL_FADE_OUT));
        }

        let mut undo = None;
        ui.scope(|ui| {
            ui.multiply_opacity(appear * fade_out);
            let font = FontId::proportional(14.0);
            let text = ui.painter().layout_no_wrap(toast.text.clone(), font, theme::text());
            let undo_w = if toast.undo.is_some() { 96.0 } else { 0.0 };
            let size = vec2(text.size().x + undo_w + 40.0, 44.0);
            let full = ui.max_rect();
            let center = pos2(full.center().x, full.bottom() - 48.0 - size.y / 2.0 + (1.0 - appear) * 12.0);
            let rect = Rect::from_center_size(center, size);
            glass_panel(ui, rect, false);
            ui.painter().galley(pos2(rect.left() + 20.0, rect.center().y - text.size().y / 2.0), text, theme::text());

            if let Some(action) = toast.undo.clone() {
                let button = Rect::from_min_size(pos2(rect.right() - undo_w - 8.0, rect.top() + 8.0), vec2(undo_w, 28.0));
                let resp = ui.interact(button, Id::new("toast-undo"), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Отменить"));
                let fill = if resp.hovered() { theme::highlight(90) } else { theme::chip_fill() };
                ui.painter().rect_filled(button, CornerRadius::same(8), fill);
                ui.painter().text(button.center(), Align2::CENTER_CENTER, "Отменить", FontId::proportional(13.5), theme::text());
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    undo = Some(action);
                }
            }
        });
        if let Some(action) = undo {
            self.toast = None;
            let undone = match action {
                Undo::Unarchive(id) => self.store.set_archived(id, false),
                Undo::Restore(id) => self.store.restore(id),
                Undo::Kind(id, kind) => self.store.set_kind(id, kind),
                Undo::Snooze(snapshot) => self.store.undo_review(&snapshot),
                Undo::Text(id, title, body) => match self.cards.iter().position(|c| c.id == id) {
                    Some(idx) => {
                        let c = &mut self.cards[idx];
                        c.tags = card::tags_of(&rich_text::strip_markup(&format!("{title}\n{body}")));
                        (c.title, c.body) = (title, body);
                        self.tag_counts = None;
                        self.store.save(&self.cards[idx])
                    }
                    None => Ok(()),
                },
            };
            self.report(undone, "отменить");
            self.reload_cards();
            self.bar.lock().unwrap().invalidate();
            self.library.lock().unwrap().invalidate();
        }
    }
}
