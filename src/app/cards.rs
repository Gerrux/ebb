//! The cards on the layer: drawn each frame, and what their actions do to the
//! cards and the database; the editor's commit, tags, a Prompt's variables.

use std::time::{Duration, Instant};

use egui::{
    CornerRadius, Id, Key, Modifiers, Pos2, Rect, RichText, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use crate::card::{self, Kind, Placement};
use crate::resurface;
use crate::rich_text;
use crate::theme;
use crate::win;

use super::EbbApp;
use super::card_text::edit_text;
use super::card_ui::{Action, HEADER_H, card_ui, mark_copied};
use super::toast::{Toast, Undo};

const REVEAL_FOR: Duration = Duration::from_secs(5);
const HIGHLIGHT_FOR: Duration = Duration::from_millis(2500);
const CARD_APPEAR: Duration = Duration::from_millis(220);
const CARD_LEAVE: Duration = Duration::from_millis(160);

/// The field that adds a tag to a card, under its "+" chip.
pub(super) struct TagInput {
    card: i64,
    /// Where it hangs from (the "+" chip's bottom left), in screen points.
    at: Pos2,
    text: String,
    focused: bool,
    opened_pass: u64,
}

pub(super) struct PromptFill {
    card: i64,
    /// The Prompt's text as it will be copied, placeholders in place.
    text: String,
    values: Vec<(String, String)>,
    /// The first field has had the focus once.
    focused: bool,
    /// The pass it opened in: the click that opened it isn't a click elsewhere.
    opened_pass: u64,
}

/// The grid a note made by double-click lands on, so notes made by hand line up.
const NOTE_GRID: f32 = 8.0;

/// Where a note made by a double-click at `at` goes (both in layer coordinates):
/// its top left there, on the grid, and the whole card on a layer of size `area`.
pub(super) fn note_pos_at(at: Pos2, area: Vec2) -> Pos2 {
    let axis = |v: f32, room: f32, size: f32| {
        // The last grid line that still leaves room for the card; 0 on a layer too small.
        let max = ((room - size) / NOTE_GRID).floor().max(0.0) * NOTE_GRID;
        ((v / NOTE_GRID).round() * NOTE_GRID).clamp(0.0, max)
    };
    Pos2::new(
        axis(at.x, area.x, card::DEFAULT_SIZE.x),
        axis(at.y, area.y, card::DEFAULT_SIZE.y),
    )
}

impl EbbApp {
    pub(super) fn commit_edit(&mut self) {
        let Some((id, buf)) = self.editing.take() else {
            return;
        };
        let Some(idx) = self.cards.iter().position(|c| c.id == id) else {
            return;
        };
        if self.cards[idx].kind == Kind::Private {
            // Two fields, no markup: the first line is the label, shown in the open
            // as typed; the rest is the secret, stored verbatim and encrypted by save.
            let (label, secret) = card::private_edit_parts(&buf);
            let c = &mut self.cards[idx];
            c.title = label;
            // Tags from the label only: the tags column is plaintext.
            c.tags = card::tags_of(&c.title);
            c.body = secret;
            self.tag_counts = None;
            let empty = self.cards[idx].body.is_empty();
            self.save(idx);
            if empty {
                // Emptied in the editor: the old value must not stay behind to show or copy.
                let cleared = self.store.clear_secret(id);
                self.report(cleared, "стереть секрет");
            }
            // On the layer only the label stays in memory.
            self.cards[idx].body.clear();
            return;
        }
        let buf = rich_text::trim(&buf);
        let c = &self.cards[idx];
        // A note closed with nothing ever written in it (a new empty note) is
        // no card at all: gone for good, not to the trash, without a toast.
        if buf.is_empty() && c.title.is_empty() && c.body.is_empty() {
            let discarded = self.store.discard_empty(id);
            // Not deleted: the database has text this copy doesn't; left as it is there.
            if self.report(discarded, "убрать пустую заметку") == Some(true) {
                let card = self.cards.remove(idx);
                self.leaving.push((card, Instant::now()));
                self.appearing.retain(|(aid, _)| *aid != id);
                if self.active == Some(id) {
                    self.active = None;
                }
                self.library.lock().unwrap().invalidate();
                self.bar.lock().unwrap().invalidate();
            }
            return;
        }
        let c = &mut self.cards[idx];
        // Saved as written; an old separate title becomes the first line of the text.
        c.title.clear();
        // Tags come from the visible text: "**#idea**" is the tag "idea".
        c.tags = card::tags_of(&rich_text::strip_markup(&buf));
        self.tag_counts = None;
        c.body = buf;
        self.save(idx);
        if self.cards[idx].kind == Kind::Private && self.cards[idx].body.is_empty() {
            // Emptied in the editor: the old value must not stay behind to show or copy.
            let cleared = self.store.clear_secret(id);
            self.report(cleared, "стереть секрет");
        }
        if self.cards[idx].kind == Kind::Private {
            // Encrypted by save; on the layer only the label stays in the open.
            let c = &mut self.cards[idx];
            c.title = card::private_parts(&c.title, &c.body).0;
            c.body.clear();
        }
    }

    /// Closes the editor if it's this card's, saving what was typed.
    fn commit_edit_of(&mut self, id: i64) {
        if self.editing.as_ref().is_some_and(|(eid, _)| *eid == id) {
            self.commit_edit();
        }
    }

    /// `commit_edit_of` for the card at `idx`: where that card is afterwards, or
    /// None if it was an empty note and is gone. Indices past it shift then.
    fn commit_edit_of_at(&mut self, idx: usize) -> Option<usize> {
        let id = self.cards[idx].id;
        self.commit_edit_of(id);
        self.index_of(id)
    }

    fn index_of(&self, id: i64) -> Option<usize> {
        self.cards.iter().position(|c| c.id == id)
    }

    /// An empty note at `pos` (layer coordinates), its editor open: the "+"
    /// button, the menu's "Новая заметка" and a double-click on the layer.
    /// Closed with nothing typed, it's discarded (see `commit_edit`).
    pub(super) fn new_note(&mut self, pos: Pos2) {
        // Not onto a layer that's rolled away or rolling: nothing would show it.
        if self.collapsed || self.curtain > 0.0 {
            return;
        }
        self.commit_edit();
        let parsed = card::Parsed {
            kind: Kind::Note,
            title: String::new(),
            body: String::new(),
            tags: vec![],
        };
        let inserted = self.store.insert(&parsed, pos);
        let Some(c) = self.report(inserted, "создать заметку") else {
            return;
        };
        // Inserted on top of the others (z), so it goes last here too.
        let id = c.id;
        self.appearing.push((id, Instant::now()));
        self.cards.push(c);
        self.active = Some(id);
        self.editing = Some((id, String::new()));
        self.revealed = None;
        self.revealed_text = None;
        self.library.lock().unwrap().invalidate();
        self.bar.lock().unwrap().invalidate();
    }

    /// Rewrites a card's text for a tag change; its tags follow from the text, as
    /// after editing. The title and text it had, if the change was saved.
    fn retag(
        &mut self,
        idx: usize,
        change: impl FnOnce(&str) -> String,
    ) -> Option<(String, String)> {
        let idx = self.commit_edit_of_at(idx)?;
        let c = &mut self.cards[idx];
        let old = (c.title.clone(), c.body.clone());
        let text = if c.title.is_empty() {
            c.body.clone()
        } else {
            format!("{}\n{}", c.title, c.body)
        };
        c.title.clear();
        c.body = change(&text);
        c.tags = card::tags_of(&rich_text::strip_markup(&c.body));
        if !self.save(idx) {
            let c = &mut self.cards[idx];
            (c.title, c.body) = old;
            c.tags = card::tags_of(&rich_text::strip_markup(&text));
            return None;
        }
        self.tag_counts = None;
        self.library.lock().unwrap().invalidate();
        self.bar.lock().unwrap().invalidate();
        Some(old)
    }

    /// The field under a card's "+" chip: type a tag and Enter, or pick one of
    /// the most used; Esc or a click elsewhere closes it.
    fn tag_input_ui(&mut self, ui: &mut Ui) {
        let Some(input) = &mut self.tag_input else {
            return;
        };
        let Some(idx) = self.cards.iter().position(|c| c.id == input.card) else {
            self.tag_input = None;
            return;
        };
        if self.tag_counts.is_none() {
            self.tag_counts = Some(self.store.tag_counts().unwrap_or_default());
        }
        let suggestions = card::suggest_tags(
            self.tag_counts.as_deref().unwrap_or_default(),
            &input.text,
            &self.cards[idx].tags,
            6,
        );
        let (mut chosen, mut cancel) = (None, false);
        let area = egui::Area::new(Id::new("tag-input"))
            .order(egui::Order::Foreground)
            .fixed_pos(input.at + vec2(0.0, 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(200.0);
                        let field = egui::TextEdit::singleline(&mut input.text)
                            .id(Id::new("tag-input-field"))
                            .hint_text("новый тег")
                            .desired_width(f32::INFINITY);
                        let resp = ui.add(field);
                        if !input.focused {
                            resp.request_focus();
                            input.focused = true;
                        }
                        if resp.lost_focus()
                            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
                        {
                            chosen = card::normalize_tag(&input.text);
                            cancel = chosen.is_none();
                        }
                        if !suggestions.is_empty() {
                            ui.add_space(4.0);
                        }
                        for tag in &suggestions {
                            let button =
                                egui::Button::new(RichText::new(format!("#{tag}")).size(13.5))
                                    .min_size(vec2(ui.available_width(), 0.0));
                            if ui.add(button).clicked() {
                                chosen = Some(tag.clone());
                            }
                        }
                    });
            });
        let cancelled = cancel
            || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            || (ui.ctx().cumulative_pass_nr() > input.opened_pass
                && ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer
                            .interact_pos()
                            .is_some_and(|p| !area.response.rect.contains(p))
                }));
        if let Some(tag) = chosen {
            self.tag_input = None;
            self.retag(idx, |text| card::with_tag(text, &tag));
        } else if cancelled {
            self.tag_input = None;
        }
    }

    /// The values for a Prompt's `{{variables}}`, over its card. Enter moves to the
    /// next field and copies from the last; Esc or a click elsewhere closes it.
    fn prompt_fill_ui(&mut self, ui: &mut Ui, origin: Pos2) {
        let Some(fill) = &mut self.prompt_fill else {
            return;
        };
        let Some(card) = self.cards.iter().find(|c| c.id == fill.card) else {
            self.prompt_fill = None;
            return;
        };
        let width = card.size.x.max(260.0);
        let at = origin + card.pos.to_vec2() + vec2(0.0, HEADER_H);
        let count = fill.values.len();
        let (mut copy, mut cancel) = (false, false);
        let area = egui::Area::new(Id::new("prompt-fill"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new("Подставить в prompt")
                                    .size(12.0)
                                    .color(theme::muted()),
                            )
                            .selectable(false),
                        );
                        ui.add_space(4.0);
                        for (i, (name, value)) in fill.values.iter_mut().enumerate() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(name.as_str()).size(13.0).color(theme::text()),
                                )
                                .selectable(false),
                            );
                            let field = egui::TextEdit::singleline(value)
                                .id(Id::new(("prompt-var", i)))
                                .hint_text(format!("{{{{{name}}}}}"))
                                .desired_width(f32::INFINITY);
                            let resp = ui.add(field);
                            if i == 0 && !fill.focused {
                                resp.request_focus();
                                fill.focused = true;
                            }
                            // Taken from the input, or the next field, focused in this same
                            // pass, would see the Enter too and give the focus up again.
                            if resp.lost_focus()
                                && ui.input_mut(|input| {
                                    input.consume_key(Modifiers::NONE, Key::Enter)
                                })
                            {
                                if i + 1 == count {
                                    copy = true;
                                } else {
                                    ui.memory_mut(|m| {
                                        m.request_focus(Id::new(("prompt-var", i + 1)))
                                    });
                                }
                            }
                            ui.add_space(2.0);
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            copy |= ui.button(RichText::new("Копировать").size(14.0)).clicked();
                            cancel |= ui.button(RichText::new("Отмена").size(14.0)).clicked();
                        });
                    });
            });
        let cancelled = cancel
            || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            || (ui.ctx().cumulative_pass_nr() > fill.opened_pass
                && ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer
                            .interact_pos()
                            .is_some_and(|p| !area.response.rect.contains(p))
                }));
        if copy {
            let (id, text) = (fill.card, card::fill_prompt(&fill.text, &fill.values));
            ui.ctx().copy_text(text);
            mark_copied(ui, id);
            let _ = self.store.touch(id);
            self.prompt_fill = None;
        } else if cancelled {
            self.prompt_fill = None;
        }
    }

    /// Changes a card's kind, keeping its text; the toast can undo it.
    fn set_kind(&mut self, idx: usize, kind: Kind) {
        let (id, old) = (self.cards[idx].id, self.cards[idx].kind);
        if old == kind {
            return;
        }
        // The editor's text is saved first, under the old kind; a card going
        // Private hides its text right away.
        let Some(idx) = self.commit_edit_of_at(idx) else {
            return;
        };
        if self.revealed.is_some_and(|(rid, _)| rid == id) {
            self.revealed = None;
            self.revealed_text = None;
        }
        // The store splits the text into label and secret, or joins them back.
        let changed = self.store.set_kind(id, kind);
        if self.report(changed, "сменить тип").is_none() {
            return;
        }
        if let Ok(Some(stored)) = self.store.card(id) {
            let c = &mut self.cards[idx];
            (c.kind, c.title, c.body) = (stored.kind, stored.title, stored.body);
        }
        let text = format!("Тип: {} → {}", old.label(), kind.label());
        self.toast = Some(Toast::new(text, Some(Undo::Kind(id, old))));
        self.library.lock().unwrap().invalidate();
        self.bar.lock().unwrap().invalidate();
    }

    pub(super) fn cards_ui(&mut self, ui: &mut Ui) {
        let origin = ui.max_rect().min;
        let desktop_origin = ui.input(|i| i.viewport().outer_rect.map_or(Pos2::ZERO, |r| r.min));
        // Not over a card while it's over the menu or another popup above the layer.
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|&p| {
            ui.ctx()
                .layer_id_at(p)
                .is_none_or(|l| l.order == egui::Order::Background)
        });
        let hovered_id = pointer.and_then(|p| {
            self.cards
                .iter()
                .rev()
                .find(|c| Rect::from_min_size(origin + c.pos.to_vec2(), c.shown_size()).contains(p))
                .map(|c| c.id)
        });
        let area = self.area(ui);

        // Only a change counts: a layer shown without ever being activated has
        // no focus to lose, and its reveal ends on the timer.
        let focused = ui.input(|i| i.viewport().focused);
        let focus_lost = self.was_focused == Some(true) && focused == Some(false);
        self.was_focused = focused;
        if let Some((_, t)) = self.revealed {
            if focus_lost || t.elapsed() >= REVEAL_FOR {
                self.revealed = None;
                self.revealed_text = None;
            } else {
                ui.ctx().request_repaint_after(REVEAL_FOR - t.elapsed());
            }
        }

        // Appear: fade in while rising a few points. Leave: fade out, not interactive.
        let ease = |t: f32| 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
        self.appearing.retain(|(_, at)| at.elapsed() < CARD_APPEAR);
        self.leaving.retain(|(_, at)| at.elapsed() < CARD_LEAVE);
        if !self.appearing.is_empty() || !self.leaving.is_empty() {
            ui.ctx().request_repaint();
        }

        // Where cards are (layer coordinates), for the magnet; None with it off.
        let rects: Vec<(i64, Rect)> = self
            .cards
            .iter()
            .map(|c| (c.id, Rect::from_min_size(c.pos, c.shown_size())))
            .collect();
        let magnet = self.snap.then_some(rects.as_slice());
        let style = self.card_style;
        // By card id, not index: closing an empty note's editor removes that
        // card, and the indices past it shift.
        let mut actions: Vec<(i64, Action)> = Vec::new();
        for idx in 0..self.cards.len() {
            let id = self.cards[idx].id;
            let editing = self
                .editing
                .as_mut()
                .filter(|(eid, _)| *eid == id)
                .map(|(_, b)| b);
            let revealed = self.revealed.is_some_and(|(rid, _)| rid == id);
            let revealed_text = self
                .revealed_text
                .as_ref()
                .filter(|(rid, _)| *rid == id)
                .map(|(_, text)| text.as_str());
            let appear = self
                .appearing
                .iter()
                .find(|(aid, _)| *aid == id)
                .map_or(1.0, |(_, at)| {
                    ease(at.elapsed().as_secs_f32() / CARD_APPEAR.as_secs_f32())
                });
            let active =
                self.active == Some(id) || self.highlighted.is_some_and(|(hid, _)| hid == id);
            let card = &mut self.cards[idx];
            let hovered = hovered_id == Some(id);
            let produced = ui
                .scope(|ui| {
                    ui.multiply_opacity(appear);
                    card_ui(
                        ui,
                        origin + vec2(0.0, (1.0 - appear) * 10.0),
                        area,
                        card,
                        hovered,
                        editing,
                        revealed,
                        revealed_text,
                        active,
                        magnet,
                        style,
                        false,
                    )
                })
                .inner;
            for a in produced {
                actions.push((id, a));
            }
        }
        for (card, at) in &mut self.leaving {
            let t = ease(at.elapsed().as_secs_f32() / CARD_LEAVE.as_secs_f32());
            ui.scope(|ui| {
                ui.disable();
                ui.multiply_opacity(1.0 - t);
                let _ = card_ui(
                    ui,
                    origin + vec2(0.0, t * 6.0),
                    area,
                    card,
                    false,
                    None,
                    false,
                    None,
                    false,
                    None,
                    style,
                    false,
                );
            });
        }

        if let Some((hid, t)) = self.highlighted {
            match self.cards.iter().find(|c| c.id == hid) {
                Some(c) if t.elapsed() < HIGHLIGHT_FOR => {
                    let fade = 1.0 - t.elapsed().as_secs_f32() / HIGHLIGHT_FOR.as_secs_f32();
                    let rect =
                        Rect::from_min_size(origin + c.pos.to_vec2(), c.shown_size()).expand(3.0);
                    let stroke = Stroke::new(2.0, c.accent().gamma_multiply(fade));
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(self.card_style.radius + 2),
                        stroke,
                        StrokeKind::Outside,
                    );
                    ui.ctx().request_repaint();
                }
                _ => self.highlighted = None,
            }
        }

        // 1–8 change the selected card's kind (not while typing anywhere).
        if let Some(idx) = self
            .active
            .and_then(|a| self.cards.iter().position(|c| c.id == a))
            && self.editing.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
        {
            const DIGITS: [Key; 8] = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
            ];
            let pressed = ui.input_mut(|i| {
                DIGITS
                    .iter()
                    .position(|k| i.consume_key(Modifiers::NONE, *k))
            });
            if let Some(n) = pressed {
                self.set_kind(idx, Kind::ALL[n]);
            }
        }

        // Clicking empty space ends editing and deselects.
        if hovered_id.is_none() && ui.input(|i| i.pointer.any_pressed()) {
            self.commit_edit();
            self.active = None;
        }

        let mut to_front = None;
        let mut remove = None;
        let mut detach = None;
        for (id, action) in actions {
            let Some(idx) = self.index_of(id) else {
                continue;
            };
            let done = matches!(action, Action::Done);
            match action {
                Action::Front => {
                    to_front = Some(id);
                    self.active = Some(self.cards[idx].id);
                    let now = resurface::unix_now();
                    if self.cards[idx].placement == Placement::Rediscover {
                        self.cards[idx].placement = Placement::Manual;
                        self.save(idx);
                    } else if self.cards[idx].due_reminder(now).is_some() {
                        // Seen: a reminder that came due has done its job.
                        self.cards[idx].review_at = None;
                        self.save(idx);
                    }
                    let _ = self.store.touch(self.cards[idx].id);
                }
                Action::WindowDragStopped => {}
                Action::Moved => {
                    self.cards[idx].placement = Placement::Manual;
                    self.save(idx);
                }
                Action::TogglePin => {
                    self.cards[idx].pinned ^= true;
                    self.cards[idx].placement = if self.cards[idx].pinned {
                        Placement::Pinned
                    } else {
                        Placement::Manual
                    };
                    self.save(idx);
                }
                Action::Copy => {
                    let c = &self.cards[idx];
                    let _ = self.store.touch(c.id);
                    if c.kind == Kind::Private {
                        if let Ok(Some(secret)) = self.store.secret(c.id) {
                            let _ = win::copy_private(&secret);
                            mark_copied(ui, c.id);
                        }
                    } else if c.kind == Kind::Prompt {
                        let text = card::prompt_text(&c.title, &c.body);
                        let variables = card::prompt_variables(&text);
                        if variables.is_empty() {
                            ui.ctx().copy_text(text);
                            mark_copied(ui, c.id);
                        } else {
                            // Its form still open (press and release in one pass): what's typed stays.
                            let old = self
                                .prompt_fill
                                .take()
                                .filter(|f| f.card == c.id)
                                .map(|f| f.values)
                                .unwrap_or_default();
                            let values = variables
                                .into_iter()
                                .map(|name| {
                                    let value = old
                                        .iter()
                                        .find(|(n, _)| *n == name)
                                        .map(|(_, v)| v.clone())
                                        .unwrap_or_default();
                                    (name, value)
                                })
                                .collect();
                            self.prompt_fill = Some(PromptFill {
                                card: c.id,
                                text,
                                values,
                                focused: false,
                                opened_pass: ui.ctx().cumulative_pass_nr(),
                            });
                        }
                    } else {
                        let text = if c.title.is_empty() {
                            c.body.clone()
                        } else {
                            format!("{}\n{}", c.title, c.body)
                        };
                        ui.ctx().copy_text(rich_text::strip_markup(&text));
                        mark_copied(ui, c.id);
                    }
                }
                Action::Detach(pos) => {
                    let Some(idx) = self.commit_edit_of_at(idx) else {
                        continue;
                    };
                    self.cards[idx].pos = desktop_origin + pos.to_vec2();
                    self.cards[idx].placement = Placement::Desktop;
                    if self.save(idx) {
                        detach = Some(id);
                    }
                }
                Action::Duplicate => {
                    let src = self.cards[idx].clone();
                    let pos = card::beside(&self.cards, &src, area);
                    let body = if src.kind == Kind::Private {
                        match self.store.secret(src.id) {
                            Ok(Some(secret)) => secret,
                            _ => continue,
                        }
                    } else {
                        src.body
                    };
                    let parsed = card::Parsed {
                        kind: src.kind,
                        title: src.title,
                        body,
                        tags: src.tags,
                    };
                    let inserted = self.store.insert(&parsed, pos);
                    if let Some(mut copy) = self.report(inserted, "создать копию") {
                        copy.size = src.size;
                        copy.tint = src.tint;
                        self.appearing.push((copy.id, Instant::now()));
                        self.cards.push(copy);
                        self.save(self.cards.len() - 1);
                        self.library.lock().unwrap().invalidate();
                        self.bar.lock().unwrap().invalidate();
                    }
                }
                Action::Archive | Action::Done => {
                    // What's typed is saved before the card goes.
                    let Some(idx) = self.commit_edit_of_at(idx) else {
                        continue;
                    };
                    let (placement, review_at) =
                        (self.cards[idx].placement, self.cards[idx].review_at);
                    // A reminder already due is dealt with; left set, Rediscover
                    // would bring the card straight back tomorrow.
                    if self.cards[idx]
                        .due_reminder(resurface::unix_now())
                        .is_some()
                    {
                        self.cards[idx].review_at = None;
                    }
                    self.cards[idx].archived = true;
                    self.cards[idx].placement = Placement::Archive;
                    if !self.save(idx) {
                        // Still in the database as it was: stays on the layer.
                        (
                            self.cards[idx].archived,
                            self.cards[idx].placement,
                            self.cards[idx].review_at,
                        ) = (false, placement, review_at);
                        continue;
                    }
                    let text = if done {
                        "Сделано, карточка в архиве"
                    } else {
                        "Карточка в архиве"
                    };
                    self.toast = Some(Toast::new(text, Some(Undo::Unarchive(id))));
                    remove = Some(id);
                }
                Action::Delete => {
                    let Some(idx) = self.commit_edit_of_at(idx) else {
                        continue;
                    };
                    let deleted = self.store.delete(self.cards[idx].id);
                    if self.report(deleted, "убрать в корзину").is_none() {
                        continue;
                    }
                    let text = format!(
                        "Карточка в корзине, {} дней можно вернуть",
                        crate::store::TRASH_DAYS
                    );
                    self.toast = Some(Toast::new(text, Some(Undo::Restore(id))));
                    remove = Some(id);
                }
                Action::StartEdit => {
                    self.commit_edit();
                    let Some(idx) = self.index_of(id) else {
                        continue;
                    };
                    let c = &self.cards[idx];
                    let _ = self.store.touch(c.id);
                    let text = if c.kind == Kind::Private {
                        // Without its value the editor would save the label as the secret.
                        match self.store.secret(c.id) {
                            Ok(Some(secret)) => card::private_edit_text(&c.title, &secret),
                            Ok(None) => card::private_edit_text(&c.title, ""),
                            Err(_) => continue,
                        }
                    } else {
                        edit_text(c)
                    };
                    self.editing = Some((c.id, text));
                    self.revealed = None;
                    self.revealed_text = None;
                }
                Action::Reveal => {
                    let id = self.cards[idx].id;
                    let _ = self.store.touch(id);
                    if let Ok(Some(secret)) = self.store.secret(id) {
                        self.revealed = Some((id, Instant::now()));
                        self.revealed_text = Some((id, secret));
                    }
                }
                Action::OpenLink => {
                    let c = &self.cards[idx];
                    let _ = self.store.touch(c.id);
                    if let Some(url) = card::first_url(&c.body) {
                        win::open_url(url);
                    }
                }
                Action::Keep => {
                    self.cards[idx].placement = Placement::Manual;
                    let _ = self.store.touch(self.cards[idx].id);
                    self.save(idx);
                    self.toast = Some(Toast::new("Останется на слое", None));
                }
                Action::ToggleCheck(line) => {
                    let _ = self.retag(idx, |text| card::toggle_check(text, line));
                }
                Action::SetKind(kind) => self.set_kind(idx, kind),
                Action::RemoveTag(tag) => {
                    let id = self.cards[idx].id;
                    if let Some((title, body)) =
                        self.retag(idx, |text| card::without_tag(text, &tag))
                    {
                        self.toast = Some(Toast::new(
                            format!("Тег #{tag} убран"),
                            Some(Undo::Text(id, title, body)),
                        ));
                    }
                }
                Action::AddTag(at) => {
                    let card = self.cards[idx].id;
                    self.tag_input = Some(TagInput {
                        card,
                        at,
                        text: String::new(),
                        focused: false,
                        opened_pass: ui.ctx().cumulative_pass_nr(),
                    });
                }
                Action::SetIdeaStatus(status) => {
                    let set = self.store.set_idea_status(self.cards[idx].id, status);
                    if self.report(set, "поменять статус идеи").is_some() {
                        self.cards[idx].idea_status = status;
                    }
                }
                Action::ToggleCollapse => {
                    let Some(idx) = self.commit_edit_of_at(idx) else {
                        continue;
                    };
                    let collapsed = !self.cards[idx].collapsed;
                    let set = self.store.set_collapsed(id, collapsed);
                    if self
                        .report(
                            set,
                            if collapsed {
                                "свернуть карточку"
                            } else {
                                "развернуть карточку"
                            },
                        )
                        .is_some()
                    {
                        self.cards[idx].collapsed = collapsed;
                    }
                }
                Action::SetTint(tint) => {
                    self.cards[idx].tint = tint;
                    self.save(idx);
                }
                Action::Snooze(days) => {
                    if self.commit_edit_of_at(idx).is_none() {
                        continue;
                    }
                    let until = resurface::snooze_until(
                        resurface::unix_now(),
                        crate::search::local_offset_secs(),
                        days,
                    );
                    let snoozed = self.store.snooze(id, until);
                    let Some(snapshot) = self.report(snoozed, "отложить карточку")
                    else {
                        continue;
                    };
                    let text = format!("Вернётся {}", resurface::snooze_label(days));
                    self.toast = Some(Toast::new(text, Some(Undo::Snooze(snapshot))));
                    remove = Some(id);
                }
            }
        }
        if let Some(idx) = detach.and_then(|id| self.index_of(id)) {
            self.floating.push(self.cards.remove(idx));
        }
        // While a value is shown, screenshots and recordings get no layer at all
        // (unless turned off in settings, e.g. to show a value in a screen share).
        let wanted = self.revealed.is_some() && self.hide_from_capture;
        if wanted != self.capture_excluded
            && let Some(h) = self.hwnd
        {
            if win::exclude_from_capture(h, wanted) {
                self.capture_excluded = wanted;
            } else {
                // Keep trying while the secret is visible, or until the old
                // affinity is actually removed after the reveal ends.
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        }
        self.prompt_fill_ui(ui, origin);
        self.tag_input_ui(ui);
        if let Some(id) = remove {
            if let Some(idx) = self.index_of(id) {
                let card = self.cards.remove(idx);
                self.leaving.push((card, Instant::now()));
            }
            self.library.lock().unwrap().invalidate();
            self.bar.lock().unwrap().invalidate();
        } else if let Some(idx) = to_front.and_then(|id| self.index_of(id))
            && idx + 1 != self.cards.len()
        {
            let c = self.cards.remove(idx);
            if let Err(e) = self.store.raise(c.id) {
                eprintln!("raise failed: {e}");
            }
            self.cards.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn note_pos_snaps_to_the_grid() {
        let area = vec2(1920.0, 1080.0);
        assert_eq!(note_pos_at(pos2(203.0, 77.0), area), pos2(200.0, 80.0));
        assert_eq!(note_pos_at(pos2(204.0, 76.0), area), pos2(208.0, 80.0));
    }

    #[test]
    fn note_pos_keeps_the_card_on_the_layer() {
        let area = vec2(1920.0, 1080.0);
        let p = note_pos_at(pos2(1900.0, 1070.0), area);
        let far = p + card::DEFAULT_SIZE;
        assert!(far.x <= area.x && far.y <= area.y, "{p:?}");
        assert_eq!((p.x % NOTE_GRID, p.y % NOTE_GRID), (0.0, 0.0));
        assert_eq!(note_pos_at(pos2(-5.0, -30.0), area), pos2(0.0, 0.0));
        // A layer smaller than a card: at its top left.
        assert_eq!(note_pos_at(pos2(100.0, 100.0), vec2(200.0, 100.0)), pos2(0.0, 0.0));
    }
}
