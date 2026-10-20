//! Cards pulled off the layer, each in its own borderless desktop window (a
//! deferred viewport), drawn with the same `card_ui` as on the layer.
//!
//! The layer owns the cards and the database; a window only reports what was
//! done to its card (`Event`), and the layer answers through `Shared`: the
//! editor's text, a Private value on show, the tag field, and the drag a card's
//! window takes over from the layer when the card is pulled off it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Id, Key, Pos2, Rect, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};

use crate::card::{self, Card, MIN_SIZE, Placement};
use crate::resurface;
use crate::win;

use super::EbbApp;
use super::card_ui::{Action, card_ui};
use super::cards::{REVEAL_FOR, TagInput, TagOutcome, tag_field};

/// Between the layer and the cards' windows.
#[derive(Default)]
pub(super) struct Shared {
    events: Vec<Event>,
    /// The card whose editor is open in its window, and the editor's text.
    editing: Option<(i64, String)>,
    /// A Private card's value on show in its window, since then.
    revealed: Option<(i64, Instant, String)>,
    /// The tag field open in a card's window, and the tags by use it suggests from.
    tag_input: Option<(TagInput, Vec<(String, i64)>)>,
    /// A card just pulled off the layer, the button still held.
    pub(super) handoff: Option<Handoff>,
}

impl Shared {
    /// The card was pulled off the layer and its window hasn't taken the drag yet.
    pub(super) fn handoff_waiting(&self, card: i64) -> bool {
        self.handoff
            .as_ref()
            .is_some_and(|h| h.card == card && !h.taken)
    }

    /// A card pulled off the layer, the button still held; its editor, if it was
    /// open on the layer, stays open in the card's window.
    pub(super) fn pull(&mut self, card: i64, grab: Vec2, editor: Option<(i64, String)>) {
        self.handoff = Some(Handoff::new(card, grab));
        if let Some(editor) = editor.filter(|(id, _)| *id == card) {
            self.editing = Some(editor);
        }
    }

    /// Drops a Private value on show whose card no longer has a window: only that
    /// window's frames time it out, and the value mustn't outlive it in memory or
    /// show again if the card is pulled off the layer once more.
    fn forget_revealed_unless(&mut self, floating: impl Fn(i64) -> bool) {
        self.revealed.take_if(|(id, _, _)| !floating(*id));
    }

    /// Drops a hand-off whose card has left `floating` before its window took
    /// the drag; nothing else would ever end it. Whether one was dropped.
    fn forget_handoff_unless(&mut self, floating: impl Fn(i64) -> bool) -> bool {
        self.handoff.take_if(|h| !floating(h.card)).is_some()
    }
}

pub(super) struct Handoff {
    card: i64,
    /// Where the pointer holds the card, from its top left (points).
    grab: Vec2,
    /// The card's window has the drag (or the button was up by then).
    taken: bool,
}

impl Handoff {
    pub(super) fn new(card: i64, grab: Vec2) -> Self {
        Self {
            card,
            grab,
            taken: false,
        }
    }
}

enum Event {
    /// The window was let go of here; over the layer, which takes it back.
    Dropped(i64, Pos2, bool),
    Resized(i64, Pos2, Vec2),
    Action(i64, Action),
    /// Its editor closed with this text.
    Edited(i64, String),
    AddTag(i64, String),
}

fn viewport_id(card: i64) -> ViewportId {
    ViewportId::from_hash_of(("floating-card", card))
}

fn window_title(card: i64) -> String {
    format!("Ebb Note {card}")
}

/// Where the window is let go of, and whether that's over the layer. `offset`:
/// where the card is in the window (see `menu_room`).
fn dropped(ui: &Ui, card: &Card, layer: Option<Rect>, offset: Vec2) -> Option<Event> {
    let (rect, scale) = ui.input(|i| {
        i.viewport()
            .outer_rect
            .zip(i.viewport().native_pixels_per_point)
    })?;
    let safe = win::clamp_to_work_area(
        Rect::from_min_size(rect.min + offset, card.shown_size()),
        scale,
    );
    let over_layer = layer.is_some_and(|target| {
        target.contains(Rect::from_min_size(safe, card.shown_size()).center())
    });
    Some(Event::Dropped(card.id, safe, over_layer))
}

/// The window while one of the card's menus is open, in card coordinates; None
/// while none is. A menu can't leave its window, and a small card's window is
/// too small for one: the window grows by the menu's size (measured the frame it
/// opens, before it's shown), below the card and to both sides, since a menu
/// hangs from a button inside the card and may reach past either edge. The card
/// keeps its place on screen.
fn menu_room(ctx: &egui::Context, card: Vec2) -> Option<Rect> {
    // A button's inset in the card, and the menu's shadow.
    const MARGIN: f32 = 16.0;
    if !egui::Popup::is_any_open(ctx) {
        return None;
    }
    let menu = ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .filter_map(|layer| m.area_rect(layer.id))
            .fold(Vec2::ZERO, |size, rect| size.max(rect.size()))
    });
    let side = (menu.x - card.x).max(0.0) + MARGIN;
    (menu.y > 0.0).then(|| {
        Rect::from_min_size(
            pos2(-side, 0.0),
            vec2(card.x + 2.0 * side, card.y + menu.y + MARGIN),
        )
    })
}

impl EbbApp {
    pub(super) fn floating_viewports(&self, ui: &Ui) {
        // Where a card can be dropped back onto the layer; None while it can't.
        let layer = (self.layer_visible && !self.collapsed)
            .then(|| ui.input(|i| i.viewport().outer_rect))
            .flatten()
            .map(|r| Rect::from_min_size(r.min, self.full_area));
        let (style, hide_from_capture) = (self.card_style, self.hide_from_capture);
        for card in &self.floating {
            let (card, shared) = (card.clone(), self.floating_shared.clone());
            let title = window_title(card.id);
            let created = win::find_own_window_title(&title).is_none();
            let mut viewport = ViewportBuilder::default()
                .with_title(title.clone())
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(!card.pinned && !card.collapsed)
                .with_min_inner_size(MIN_SIZE)
                .with_taskbar(false);
            if created {
                viewport = viewport
                    .with_inner_size(card.shown_size())
                    .with_position(card.pos);
            }
            ui.ctx().show_viewport_deferred(
                viewport_id(card.id),
                viewport,
                move |ui, _class| {
                    card_window(ui, &title, &card, &shared, layer, style, hide_from_capture)
                },
            );
        }
    }

    /// What the cards' windows reported since the last frame.
    pub(super) fn floating_events(&mut self, ctx: &egui::Context) {
        let events = std::mem::take(&mut self.floating_shared.lock().unwrap().events);
        for event in events {
            match event {
                Event::Resized(id, pos, size) => {
                    if let Some(card) = self.floating.iter_mut().find(|c| c.id == id) {
                        card.pos = pos;
                        card.size = size;
                        let _ = self.store.save(card);
                    }
                }
                Event::Dropped(id, pos, over_layer) => {
                    let Some(idx) = self.floating.iter().position(|c| c.id == id) else {
                        continue;
                    };
                    if over_layer {
                        // The editor comes back onto the layer with the card, as it
                        // went out with it: closed, an empty new note would be gone.
                        let editor = self.floating_shared.lock().unwrap().editing.take_if(|(eid, _)| *eid == id);
                        let mut card = self.floating.remove(idx);
                        let origin =
                            ctx.input(|i| i.viewport().outer_rect.map_or(Pos2::ZERO, |r| r.min));
                        let max = self.full_area - card.size;
                        card.pos = pos2(
                            (pos.x - origin.x).clamp(0.0, max.x.max(0.0)),
                            (pos.y - origin.y).clamp(0.0, max.y.max(0.0)),
                        );
                        card.placement = Placement::Manual;
                        let _ = self.store.save(&card);
                        self.cards.push(card);
                        if editor.is_some() {
                            // One editor at a time: the layer's, if any, closes first.
                            self.commit_edit();
                            self.editing = editor;
                            self.active = Some(id);
                        }
                    } else {
                        self.floating[idx].pos = pos;
                        let _ = self.store.save(&self.floating[idx]);
                    }
                }
                Event::Edited(id, text) => self.commit_floating(id, &text),
                Event::AddTag(id, tag) => {
                    self.retext_floating(id, |text| card::with_tag(text, &tag));
                }
                Event::Action(id, action) => self.floating_action(ctx, id, action),
            }
        }
        let is_floating = |id| self.floating.iter().any(|c| c.id == id);
        {
            let mut shared = self.floating_shared.lock().unwrap();
            shared.forget_revealed_unless(is_floating);
            shared.forget_handoff_unless(is_floating);
        }
        // The stand-in on the layer goes with its hand-off (and the button's
        // release it was owed, as when the window takes the drag).
        if self.pulled.take_if(|c| !is_floating(c.id)).is_some() {
            self.release_pointer = true;
            ctx.request_repaint();
        }
    }

    /// After a display change: each card's window goes to where the card was
    /// left, brought inside the work area of the nearest monitor. The card's own
    /// position (the user's intent) is not touched, so a monitor that comes back
    /// gets the card back where it was. Only a drop or a resize by the user
    /// saves a position (see `Event`), and this move is neither.
    pub(super) fn reclamp_floating(&self, ctx: &egui::Context) {
        let handoff = self.floating_shared.lock().unwrap().handoff.as_ref().map(|h| h.card);
        for card in &self.floating {
            // The pointer places a window being pulled off the layer.
            if handoff == Some(card.id) {
                continue;
            }
            // Not up yet: it is created at the card's position and clamps itself.
            let Some(h) = win::find_own_window_title(&window_title(card.id)) else {
                continue;
            };
            let at = win::clamp_to_work_area(Rect::from_min_size(card.pos, card.shown_size()), win::dpi_scale(h));
            let id = viewport_id(card.id);
            ctx.send_viewport_cmd_to(id, ViewportCommand::OuterPosition(at));
            ctx.request_repaint_of(id);
        }
    }

    fn floating_action(&mut self, ctx: &egui::Context, id: i64, action: Action) {
        // What's typed is saved before anything else changes the card.
        if !matches!(
            action,
            Action::Front
                | Action::Moved
                | Action::WindowDragStopped
                | Action::Copy
                | Action::OpenLink
                | Action::StartEdit
                | Action::Reveal
        ) {
            self.commit_floating_edit_of(id);
        }
        let Some(idx) = self.floating.iter().position(|c| c.id == id) else {
            return;
        };
        let repaint = || ctx.request_repaint_of(viewport_id(id));
        match action {
            Action::Front | Action::Moved | Action::WindowDragStopped => {
                let _ = self.store.touch(id);
            }
            Action::StartEdit => {
                // One editor at a time, on the layer or in any window.
                self.commit_edit();
                let Some(idx) = self.floating.iter().position(|c| c.id == id) else {
                    return;
                };
                let _ = self.store.touch(id);
                let Some(text) = self.edit_text_of(&self.floating[idx]) else {
                    return;
                };
                let mut shared = self.floating_shared.lock().unwrap();
                shared.editing = Some((id, text));
                shared.revealed = None;
                repaint();
            }
            Action::Reveal => {
                let _ = self.store.touch(id);
                if let Ok(Some(secret)) = self.store.secret(id) {
                    self.floating_shared.lock().unwrap().revealed =
                        Some((id, Instant::now(), secret));
                    repaint();
                }
            }
            Action::AddTag(at) => {
                self.commit_edit();
                if self.tag_counts.is_none() {
                    self.tag_counts = Some(self.store.tag_counts().unwrap_or_default());
                }
                let counts = self.tag_counts.clone().unwrap_or_default();
                let pass = ctx.cumulative_pass_nr_for(viewport_id(id));
                self.floating_shared.lock().unwrap().tag_input =
                    Some((TagInput::new(id, at, pass), counts));
                repaint();
            }
            Action::Copy => {
                let c = &self.floating[idx];
                if c.kind == card::Kind::Private {
                    if let Ok(Some(secret)) = self.store.secret(id) {
                        let _ = win::copy_private(&secret);
                    }
                } else {
                    let text = if c.title.is_empty() {
                        c.body.clone()
                    } else {
                        format!("{}\n{}", c.title, c.body)
                    };
                    ctx.copy_text(crate::rich_text::strip_markup(&text));
                }
                let _ = self.store.touch(id);
            }
            Action::Duplicate => {
                let src = self.floating[idx].clone();
                let body = if src.kind == card::Kind::Private {
                    match self.store.secret(src.id) {
                        Ok(Some(secret)) => secret,
                        _ => return,
                    }
                } else {
                    src.body.clone()
                };
                let parsed = card::Parsed {
                    kind: src.kind,
                    title: src.title.clone(),
                    body,
                    tags: src.tags.clone(),
                };
                if let Ok(mut copy) = self.store.insert(&parsed, Pos2::ZERO) {
                    copy.size = src.size;
                    copy.tint = src.tint;
                    copy.pos = src.pos + vec2(24.0, 24.0);
                    copy.placement = Placement::Desktop;
                    if self.store.save(&copy).is_ok() {
                        self.floating.push(copy);
                    }
                }
            }
            Action::TogglePin => {
                let c = &mut self.floating[idx];
                c.pinned = !c.pinned;
                let _ = self.store.save(c);
            }
            Action::SetTint(tint) => {
                self.floating[idx].tint = tint;
                let _ = self.store.save(&self.floating[idx]);
            }
            Action::SetKind(kind) => {
                if self.store.set_kind(id, kind).is_ok()
                    && let Ok(Some(stored)) = self.store.card(id)
                {
                    let c = &mut self.floating[idx];
                    (c.kind, c.title, c.body) = (stored.kind, stored.title, stored.body);
                }
            }
            Action::ToggleCollapse => {
                let collapsed = !self.floating[idx].collapsed;
                if self.store.set_collapsed(id, collapsed).is_ok() {
                    self.floating[idx].collapsed = collapsed;
                }
            }
            Action::SetIdeaStatus(status) => {
                if self.store.set_idea_status(id, status).is_ok() {
                    self.floating[idx].idea_status = status;
                }
            }
            Action::ToggleCheck(line) => {
                self.retext_floating(id, |text| card::toggle_check(text, line));
            }
            Action::RemoveTag(tag) => {
                self.retext_floating(id, |text| card::without_tag(text, &tag));
            }
            Action::OpenLink => {
                if let Some(url) = card::first_url(&self.floating[idx].body) {
                    win::open_url(url);
                }
            }
            Action::Archive | Action::Done => {
                let c = &mut self.floating[idx];
                c.archived = true;
                c.placement = Placement::Archive;
                if self.store.save(c).is_ok() {
                    self.floating.remove(idx);
                }
            }
            Action::Delete => {
                if self.store.delete(id).is_ok() {
                    self.floating.remove(idx);
                }
            }
            Action::Snooze(days) => {
                let until = resurface::snooze_until(
                    resurface::unix_now(),
                    crate::search::local_offset_secs(),
                    days,
                );
                if self.store.snooze(id, until).is_ok() {
                    self.floating.remove(idx);
                }
            }
            Action::Keep => {
                let mut card = self.floating.remove(idx);
                card.placement = Placement::Manual;
                card.pos = card::free_slot_for(&self.cards, card.size, self.full_area);
                let _ = self.store.save(&card);
                self.cards.push(card);
            }
            Action::Detach { .. } => {}
        }
    }

    /// Closes the editor open in a card's window, if any, saving what was typed.
    pub(super) fn commit_floating_edit(&mut self) {
        let taken = self.floating_shared.lock().unwrap().editing.take();
        if let Some((id, text)) = taken {
            self.commit_floating(id, &text);
        }
    }

    fn commit_floating_edit_of(&mut self, id: i64) {
        let taken = {
            let mut shared = self.floating_shared.lock().unwrap();
            shared
                .editing
                .take_if(|(eid, _)| *eid == id)
        };
        if let Some((id, text)) = taken {
            self.commit_floating(id, &text);
        }
    }

    fn commit_floating(&mut self, id: i64, text: &str) {
        let Some(idx) = self.floating.iter().position(|c| c.id == id) else {
            return;
        };
        let mut c = self.floating[idx].clone();
        if self.write_edit(&mut c, text) {
            self.floating[idx] = c;
        } else {
            // A new empty note closed with nothing in it: gone, and its window with it.
            self.floating.remove(idx);
            self.library.lock().unwrap().invalidate();
            self.bar.lock().unwrap().invalidate();
        }
    }

    fn retext_floating(&mut self, id: i64, change: impl FnOnce(&str) -> String) {
        self.commit_floating_edit_of(id);
        let Some(idx) = self.floating.iter().position(|c| c.id == id) else {
            return;
        };
        let mut c = self.floating[idx].clone();
        self.retext(&mut c, change);
        self.floating[idx] = c;
    }
}

/// One card's window, each frame.
fn card_window(
    ui: &mut Ui,
    title: &str,
    card: &Card,
    shared: &Arc<Mutex<Shared>>,
    layer: Option<Rect>,
    style: card::CardStyle,
    hide_from_capture: bool,
) {
    let hwnd = win::find_own_window_title(title);
    if let Some(h) = hwnd {
        win::set_window_rounded(h, false);
        win::install_window_rules(h, win::BORDERLESS);
    }
    let full = ui.max_rect();
    let original_size = card.size;
    let mut card = card.clone();
    card.pos = Pos2::ZERO;
    let id = card.id;
    let to_layer = |ui: &Ui, event: Event| {
        shared.lock().unwrap().events.push(event);
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    };

    // Only a change counts, as on the layer.
    let focused = ui.input(|i| i.viewport().focused);
    let focus_id = Id::new(("floating-focus", id));
    let was_focused = ui.data(|d| d.get_temp::<bool>(focus_id));
    if let Some(f) = focused {
        ui.data_mut(|d| d.insert_temp(focus_id, f));
    }
    let focus_lost = was_focused == Some(true) && focused == Some(false);

    // What the layer opened in this window.
    let (mut editing, revealed_text, tag_input, handoff) = {
        let mut s = shared.lock().unwrap();
        // A value on show goes after a while, or when the window loses the focus.
        if s.revealed
            .as_ref()
            .is_some_and(|(rid, at, _)| *rid == id && (focus_lost || at.elapsed() >= REVEAL_FOR))
        {
            s.revealed = None;
        }
        (
            s.editing
                .as_ref()
                .filter(|(eid, _)| *eid == id)
                .map(|(_, text)| text.clone()),
            s.revealed
                .as_ref()
                .filter(|(rid, _, _)| *rid == id)
                .map(|(_, at, text)| (*at, text.clone())),
            s.tag_input.take_if(|(input, _)| input.card() == id),
            s.handoff
                .as_ref()
                .filter(|h| h.card == id)
                .map(|h| (h.grab, h.taken)),
        )
    };
    if let Some((at, _)) = &revealed_text {
        ui.ctx()
            .request_repaint_after(REVEAL_FOR.saturating_sub(at.elapsed()));
    }
    // While a value is shown, screenshots and recordings don't get this window.
    let wanted = revealed_text.is_some() && hide_from_capture;
    let excluded_id = Id::new(("floating-excluded", id));
    let excluded = ui.data(|d| d.get_temp::<bool>(excluded_id)).unwrap_or(false);
    if wanted != excluded
        && let Some(h) = hwnd
    {
        if win::exclude_from_capture(h, wanted) {
            ui.data_mut(|d| d.insert_temp(excluded_id, wanted));
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    // The editor closes on Esc, or when the window loses the focus (a click
    // anywhere else); the tag field takes its own Esc.
    let edit_done = editing.is_some()
        && tag_input.is_none()
        && (focus_lost || ui.input(|i| i.key_pressed(Key::Escape)));

    let viewport_rect = ui.input(|i| i.viewport().outer_rect);
    let scale = ui.input(|i| i.viewport().native_pixels_per_point);
    let native_resize_id = Id::new(("card", id)).with("window-resize");
    let native_resizing = ui
        .data(|data| data.get_temp::<bool>(native_resize_id))
        .unwrap_or(false);
    // While a menu is open the window is bigger than the card (see `menu_room`):
    // the window's rect in card coordinates.
    let room_id = Id::new(("floating-room", id));
    let room = ui.data(|d| d.get_temp::<Rect>(room_id));
    let offset = room.map_or(Vec2::ZERO, |r| -r.min.to_vec2());
    if native_resizing && !card.collapsed && room.is_none() {
        card.size = full.size();
    }
    let card_rect = room.map_or(full, |_| {
        Rect::from_min_size(full.min + offset, card.shown_size())
    });
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| card_rect.contains(p));
    let active = editing.is_some();
    let actions = card_ui(
        ui,
        card_rect.min,
        card_rect.size(),
        &mut card,
        hovered,
        editing.as_mut(),
        revealed_text.is_some(),
        revealed_text.as_ref().map(|(_, text)| text.as_str()),
        active,
        None,
        style,
        true,
    );
    if let Some(text) = editing {
        let mut s = shared.lock().unwrap();
        if edit_done {
            s.editing.take_if(|(eid, _)| *eid == id);
            s.events.push(Event::Edited(id, text));
            ui.ctx().request_repaint_of(ViewportId::ROOT);
        } else if let Some((_, buf)) = s.editing.as_mut().filter(|(eid, _)| *eid == id) {
            *buf = text;
        }
    }
    if let Some((mut input, counts)) = tag_input {
        let suggestions = card::suggest_tags(&counts, input.text(), &card.tags, 6);
        match tag_field(ui, &mut input, &suggestions) {
            TagOutcome::Open => shared.lock().unwrap().tag_input = Some((input, counts)),
            TagOutcome::Chosen(tag) => to_layer(ui, Event::AddTag(id, tag)),
            TagOutcome::Cancelled => {}
        }
    }

    // Just pulled off the layer: the window takes the held button over and
    // follows the pointer; when it's let go, it's dropped like after any drag.
    let mut handoff_done = false;
    match (handoff, hwnd) {
        (Some((grab, false)), Some(h)) => {
            let ppp = scale.unwrap_or(1.0);
            let grab = ((grab.x * ppp).round() as i32, (grab.y * ppp).round() as i32);
            if win::primary_button_down() {
                win::begin_move(h, grab);
            } else {
                // Let go before the window was up: it goes where the pointer is,
                // and is dropped there next frame, once egui knows where it is.
                win::place_under_cursor(h, grab);
                ui.ctx().request_repaint();
            }
            if let Some(handoff) = shared.lock().unwrap().handoff.as_mut() {
                handoff.taken = true;
            }
            // The layer drops the card it kept drawing until now.
            ui.ctx().request_repaint_of(ViewportId::ROOT);
        }
        // The loop's end wakes the window (see `win::begin_move`); a button let
        // go before the loop started is released over this window, which wakes it too.
        (Some((_, true)), Some(h)) => handoff_done = win::take_move_done(h),
        (Some(_), None) => ui.ctx().request_repaint(),
        (None, _) => {}
    }
    if handoff_done {
        shared.lock().unwrap().handoff.take_if(|h| h.card == id);
        if let Some(event) = dropped(ui, &card, layer, offset) {
            to_layer(ui, event);
        }
    }

    let desired_size = card.shown_size();
    let native_size = ui
        .input(|i| i.viewport().inner_rect)
        .map(|rect| rect.size());
    let clamp = |desired: Rect, scale: f32| {
        hwnd.map_or_else(
            || win::clamp_to_work_area(desired, scale),
            |h| win::clamp_to_work_area_on(h, desired, scale),
        )
    };
    // Where the card is on screen, kept in the work area.
    let safe = viewport_rect
        .zip(scale)
        .map(|(rect, scale)| clamp(Rect::from_min_size(rect.min + offset, desired_size), scale));
    // The room for a menu goes into the work area too; it's the card that keeps
    // its place, so near the screen's bottom the window grows upwards instead.
    let new_room = menu_room(ui.ctx(), desired_size)
        .filter(|_| !native_resizing && handoff.is_none())
        .zip(safe.zip(scale))
        .map(|(room, (safe, scale))| {
            let window = room.translate(safe.to_vec2());
            Rect::from_min_size(clamp(window, scale), room.size()).translate(-safe.to_vec2())
        });
    if new_room != room {
        if let Some(r) = new_room {
            ui.data_mut(|d| d.insert_temp(room_id, r));
        } else {
            ui.data_mut(|d| d.remove::<Rect>(room_id));
        }
        let size = new_room.map_or(desired_size, |r| r.size());
        if native_size != Some(size) {
            ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        ui.ctx().request_repaint();
    }
    // Not while the system moves it: that's the pointer's to place.
    if let (Some(rect), Some(safe), None) = (viewport_rect, safe, handoff) {
        let window = safe + new_room.map_or(Vec2::ZERO, |r| r.min.to_vec2());
        if window != rect.min {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::OuterPosition(window));
        }
    }
    if card.size != original_size {
        if !native_resizing && new_room.is_none() && native_size != Some(desired_size) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::InnerSize(desired_size));
        }
        if let Some(safe) = safe {
            to_layer(ui, Event::Resized(id, safe, card.size));
        }
    }
    for action in actions {
        match action {
            Action::WindowDragStopped => {
                if let Some(event) = dropped(ui, &card, layer, offset) {
                    to_layer(ui, event);
                }
            }
            action => to_layer(ui, Event::Action(id, action)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revealed_value_goes_with_its_window() {
        let mut shared = Shared {
            revealed: Some((7, Instant::now(), "secret".to_owned())),
            ..Shared::default()
        };
        shared.forget_revealed_unless(|id| id == 7);
        assert!(shared.revealed.is_some());
        // Its card was archived, or dropped back onto the layer.
        shared.forget_revealed_unless(|id| id == 8);
        assert!(shared.revealed.is_none());
    }

    #[test]
    fn handoff_ends_with_its_card() {
        let mut shared = Shared {
            handoff: Some(Handoff::new(7, vec2(10.0, 5.0))),
            ..Shared::default()
        };
        // Its card is still in a window: the window takes the drag in its own time.
        assert!(!shared.forget_handoff_unless(|id| id == 7));
        assert!(shared.handoff_waiting(7));
        // The card left before its window's first frame.
        assert!(shared.forget_handoff_unless(|id| id == 8));
        assert!(shared.handoff.is_none());
        assert!(!shared.forget_handoff_unless(|_| false));
    }

    #[test]
    fn pulled_card_keeps_its_editor() {
        // A new note pulled off the layer before anything was typed in it: its
        // editor opens in its window, so it isn't discarded on the way.
        let mut shared = Shared::default();
        shared.pull(7, vec2(10.0, 5.0), Some((7, String::new())));
        assert!(shared.handoff_waiting(7));
        assert_eq!(shared.editing, Some((7, String::new())));
        // Without an editor open on the layer there is none in the window either.
        let mut shared = Shared::default();
        shared.pull(7, vec2(10.0, 5.0), None);
        assert!(shared.editing.is_none());
    }
}
