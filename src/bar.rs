//! The command bar: one small always-on-top window that is either quick capture
//! or search. One pre-created window for both instead of two saves a GL surface
//! (~4 MiB); switching modes only resizes it.
//!
//! The command palette lives in the search mode: a query that starts with `>`
//! lists commands (`commands`) instead of notes. The note search doesn't pay for
//! it: one check of the query's first character.
//!
//! The bar renders in its own deferred-viewport callback, so it keeps its own
//! database connection for searching and hands anything that changes the layer
//! to the root viewport through [`Outbox`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Key, Layout, Modifiers, Rect, RichText,
    Sense, Ui, UiBuilder, Vec2, ViewportId, pos2, vec2,
};

use crate::card::{Kind, Placement, parse_capture_with};
use crate::commands::{self, Arg, BarTarget, CommandId, Ctx, Match};
use crate::search::{self, MARK_END, MARK_START};
use crate::store::{Hit, STORE_FAILED, Store};
use crate::theme;
use crate::win;

pub const CAPTURE_SIZE: Vec2 = vec2(640.0, 132.0);
pub const SEARCH_SIZE: Vec2 = vec2(680.0, 476.0);
const ROW_H: f32 = 54.0;
/// Appear animation: window fade (see app/mod.rs) and content rise.
pub const APPEAR_SECS: f32 = 0.16;
pub const DISAPPEAR_SECS: f32 = 0.11;
const RESULTS: usize = 40;
const CMD_ROW_H: f32 = 40.0;
/// Settings key of the palette's recently used commands.
const SET_RECENT: &str = "palette.recent";

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("capture")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Capture,
    Search,
}

/// Requests for the layer, applied by the root viewport.
pub enum Outbox {
    /// Captured text; `true` when the Private chip was taken off it.
    Captured(String, bool),
    /// Show this card on the layer (restoring it from the archive if needed).
    Open(i64),
    /// Cards changed in the database (pin, archive, delete): reload.
    Changed,
    /// A palette command to run, with its number if it takes one.
    Command(CommandId, Option<i64>),
}

pub enum Press {
    Show,
    Switch,
    Hide,
}

#[derive(Default)]
pub struct BarState {
    /// Logically open (accepts input).
    pub visible: bool,
    /// The window is on screen: true from show until its fade-out finished.
    pub shown: bool,
    /// The fade-in for the current showing has been started (from the bar's own
    /// first frame, so it isn't spent while eframe hasn't shown the window yet).
    fading_in: bool,
    /// Closed from inside the bar (Enter, Esc, focus lost): the root fades the
    /// window out and then hides it.
    pub hide_requested: bool,
    pub mode: Mode,
    /// When the hotkey was pressed for the current showing.
    pub pressed_at: Option<Instant>,
    pub latency_ms: Option<f64>,
    pub hwnd: Option<isize>,
    pub outbox: Vec<Outbox>,
    request_focus: bool,

    // Capture
    text: String,
    /// The Private chip was taken off: the text only looked like a credential.
    plain: bool,

    // Search
    query: String,
    /// Query the current hits belong to; None forces a refresh.
    searched: Option<String>,
    last_query: String,
    parsed: search::Query,
    hits: Vec<Hit>,
    selected: usize,
    first_row: usize,
    /// Index into the action strip when Tab moved focus there.
    action: Option<usize>,
    search_ms: f64,
    notice: Option<(String, Instant)>,
    store: Option<Store>,

    // Command palette
    /// What the app looks like, filled by the root each time the bar opens.
    pub ctx: Ctx,
    commands: Vec<Match>,
    /// Query the current commands belong to; None forces a refresh.
    commands_for: Option<String>,
    /// The last frame showed commands, not notes.
    was_command: bool,
    /// Recently used commands, read from the settings when first needed.
    recent: Vec<CommandId>,
    recent_loaded: bool,
    /// The query was set by the bar: put the caret after its last character.
    caret_end: bool,
}

/// The search text is a command query: it starts with `>`.
fn command_text(query: &str) -> Option<&str> {
    query.trim_start().strip_prefix('>')
}

fn query_id() -> Id {
    Id::new("bar-search-query")
}

impl BarState {
    pub fn size(&self) -> Vec2 {
        match self.mode {
            Mode::Capture => CAPTURE_SIZE,
            Mode::Search => SEARCH_SIZE,
        }
    }

    /// A hotkey for `mode` was pressed at `t`.
    pub fn press(&mut self, mode: Mode, t: Instant) -> Press {
        self.press_as(mode, t, false)
    }

    /// The palette's hotkey was pressed at `t`: search with `>` typed.
    pub fn press_palette(&mut self, t: Instant) -> Press {
        self.press_as(Mode::Search, t, true)
    }

    fn press_as(&mut self, mode: Mode, t: Instant, palette: bool) -> Press {
        if self.visible && self.mode == mode {
            // The palette's hotkey turns an open note search into the palette,
            // and closes an open palette.
            if !palette || command_text(&self.query).is_some() {
                self.visible = false;
                return Press::Hide;
            }
        }
        let was_visible = self.visible;
        self.visible = true;
        self.shown = true;
        self.mode = mode;
        self.request_focus = true;
        if mode == Mode::Search {
            self.query.clear();
            self.searched = None;
            self.commands_for = None;
            self.action = None;
            if palette {
                self.query.push('>');
                self.caret_end = true;
                self.selected = 0;
                self.first_row = 0;
            }
        }
        if was_visible {
            Press::Switch
        } else {
            self.pressed_at = Some(t);
            self.latency_ms = None;
            self.fading_in = false;
            Press::Show
        }
    }

    /// A capture that couldn't be saved comes back into the field, unless
    /// something new was typed since.
    pub fn restore_capture(&mut self, text: String, plain: bool) {
        if self.text.trim().is_empty() {
            (self.text, self.plain) = (text, plain);
        }
    }

    /// Re-runs the search on the next frame (after the data changed).
    pub fn invalidate(&mut self) {
        self.searched = None;
    }
}

pub fn ui(ui: &mut Ui, state: &Mutex<BarState>) {
    let mut st = state.lock().unwrap();
    if !st.shown {
        return;
    }
    // Fading out: keep drawing, but no input.
    if !st.visible {
        ui.disable();
    }
    if st.visible && !st.fading_in {
        st.fading_in = true;
        if let Some(h) = st.hwnd {
            crate::win::fade(h, 0, 255, Duration::from_secs_f32(APPEAR_SECS), || {});
        }
    }
    if st.latency_ms.is_none() {
        if let Some(t) = st.pressed_at {
            st.latency_ms = Some(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let lost_focus = ui.input(|i| i.viewport().focused == Some(false))
        && st.pressed_at.is_some_and(|t| t.elapsed() > Duration::from_millis(400));

    let rect = ui.max_rect();
    // Square and unstroked: DWM rounds the window and draws its border (apply_backdrop).
    ui.painter().rect_filled(rect, CornerRadius::ZERO, theme::window_fill(150));

    // The window itself fades in (root, win::fade); the content rises a few
    // points while it does.
    let t = st.pressed_at.map_or(1.0, |p| (p.elapsed().as_secs_f32() / APPEAR_SECS).min(1.0));
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    let rise = (1.0 - t).powi(3) * 8.0;
    let content = rect.translate(vec2(0.0, rise));
    let close = ui
        .scope_builder(UiBuilder::new().max_rect(content), |ui| match st.mode {
            Mode::Capture => capture_ui(ui, &mut st),
            Mode::Search => search_ui(ui, &mut st),
        })
        .inner;
    let close = st.visible && (close || lost_focus);
    if close {
        st.visible = false;
        st.hide_requested = true;
    }
    if !st.outbox.is_empty() || close {
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    }
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

fn capture_ui(ui: &mut Ui, st: &mut BarState) -> bool {
    let enabled = ui.is_enabled();
    let (submit, cancel) = ui.input_mut(|i| {
        (
            enabled && i.consume_key(Modifiers::NONE, Key::Enter),
            enabled && i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });

    if st.text.trim().is_empty() {
        st.plain = false;
    }
    let parsed = parse_capture_with(&st.text, !st.plain);
    // Detected rather than asked for with a prefix: the chip can be taken off.
    let detected = parsed.kind == Kind::Private && parse_capture_with(&st.text, false).kind != Kind::Private;
    let inner = ui.max_rect().shrink2(vec2(18.0, 14.0));
    let mut edit_resp = None;
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.horizontal_top(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("\u{E710}").font(theme::icons(16.0)).color(parsed.kind.accent()));
            ui.add_space(6.0);
            edit_resp = Some(
                ui.add(
                    egui::TextEdit::multiline(&mut st.text)
                        .hint_text("Запиши мысль…")
                        .font(FontId::proportional(18.0))
                        .text_color(theme::text())
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2),
                ),
            );
        });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(parsed.kind.icon()).font(theme::icons(11.0)).color(parsed.kind.accent()));
                ui.label(RichText::new(parsed.kind.label()).size(12.0).color(theme::dim()));
                if detected {
                    let off = ui
                        .add(egui::Label::new(RichText::new("\u{E711}").font(theme::icons(10.0)).color(theme::dim())).sense(Sense::click()))
                        .on_hover_text("Похоже на секрет. Сохранить обычной заметкой");
                    if off.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if off.clicked() {
                        st.plain = true;
                        st.request_focus = true;
                    }
                }
                for tag in &parsed.tags {
                    ui.label(RichText::new(format!("#{tag}")).size(12.0).color(theme::muted()));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hint = match st.latency_ms {
                        Some(l) => format!("Enter — сохранить · Shift+Enter — строка · Esc   ·   {l:.0} мс"),
                        None => "Enter — сохранить · Shift+Enter — строка · Esc".into(),
                    };
                    ui.label(RichText::new(hint).size(11.5).color(theme::muted()));
                });
            });
        });
    });

    if std::mem::take(&mut st.request_focus) {
        if let Some(r) = &edit_resp {
            r.request_focus();
        }
    }
    if submit && !st.text.trim().is_empty() {
        let text = std::mem::take(&mut st.text);
        st.outbox.push(Outbox::Captured(text, std::mem::take(&mut st.plain)));
        return true;
    }
    cancel
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ActionKind {
    Open,
    Copy,
    Pin,
    Archive,
    Delete,
}

fn actions(hit: &Hit) -> [(ActionKind, &'static str); 5] {
    [
        (ActionKind::Open, "На слой"),
        (ActionKind::Copy, "Копировать"),
        (ActionKind::Pin, if hit.pinned { "Открепить" } else { "Закрепить" }),
        (ActionKind::Archive, if hit.archived { "Вернуть на слой" } else { "В архив" }),
        (ActionKind::Delete, "В корзину"),
    ]
}

fn open_store(st: &mut BarState) {
    if st.store.is_none() {
        st.store = Store::open().ok();
        if st.store.is_none() {
            st.notice = Some((STORE_FAILED.into(), Instant::now()));
        }
    }
}

fn run_search(st: &mut BarState) {
    if st.searched.as_deref() == Some(st.query.as_str()) {
        return;
    }
    open_store(st);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    st.parsed = search::parse_local(&st.query, now);
    let selected_id = st.hits.get(st.selected).map(|h| h.id);
    let started = Instant::now();
    st.hits = st.store.as_ref().and_then(|s| s.search(&st.parsed, crate::store::Scope::Live, RESULTS).ok()).unwrap_or_default();
    st.search_ms = started.elapsed().as_secs_f64() * 1000.0;
    if st.last_query != st.query {
        // New text: start from the top.
        st.selected = 0;
        st.first_row = 0;
        st.last_query = st.query.clone();
    } else {
        // Refresh after pin/archive re-sorts the list: keep the same card selected.
        st.selected = selected_id
            .and_then(|id| st.hits.iter().position(|h| h.id == id))
            .unwrap_or(st.selected.min(st.hits.len().saturating_sub(1)));
    }
    st.searched = Some(st.query.clone());
}

fn perform(ui: &Ui, st: &mut BarState, kind: ActionKind) -> bool {
    let Some(hit) = st.hits.get(st.selected).cloned() else { return false };
    let Some(store) = st.store.as_ref() else {
        st.notice = Some((STORE_FAILED.into(), Instant::now()));
        return false;
    };
    match kind {
        ActionKind::Open => {
            st.outbox.push(Outbox::Open(hit.id));
            return true;
        }
        ActionKind::Copy => {
            if let Ok(Some(card)) = store.card(hit.id) {
                let notice = if card.kind == Kind::Private {
                    match store.secret(card.id) {
                        Ok(Some(secret)) if win::copy_private(&secret) => "Скопировано, очистится через 30 с",
                        _ => "Не удалось скопировать",
                    }
                } else {
                    let text = if card.title.is_empty() { card.body } else { format!("{}\n{}", card.title, card.body) };
                    ui.ctx().copy_text(text);
                    "Скопировано"
                };
                st.notice = Some((notice.into(), Instant::now()));
            }
        }
        ActionKind::Pin => {
            let saved = store.card(hit.id).and_then(|card| {
                let Some(mut card) = card else { return Ok(()) };
                card.pinned ^= true;
                // As on the layer: a pinned Rediscover card mustn't go back to the archive tomorrow.
                card.placement = if card.pinned { Placement::Pinned } else { Placement::Manual };
                store.save(&card)
            });
            if saved.is_err() {
                st.notice = Some(("Не удалось закрепить".into(), Instant::now()));
            }
            st.outbox.push(Outbox::Changed);
            st.searched = None;
        }
        ActionKind::Archive => {
            if hit.archived {
                st.outbox.push(Outbox::Open(hit.id));
                st.notice = Some(("Возвращено на слой".into(), Instant::now()));
            } else {
                let notice = match store.set_archived(hit.id, true) {
                    Ok(()) => "Убрано в архив",
                    Err(_) => "Не удалось убрать в архив",
                };
                st.outbox.push(Outbox::Changed);
                st.notice = Some((notice.into(), Instant::now()));
            }
            st.searched = None;
        }
        ActionKind::Delete => {
            let notice = match store.delete(hit.id) {
                Ok(()) => "В корзине",
                Err(_) => "Не удалось удалить",
            };
            st.outbox.push(Outbox::Changed);
            st.notice = Some((notice.into(), Instant::now()));
            st.action = None;
            st.searched = None;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

/// Looks the commands up again when the query changed.
fn refresh_commands(st: &mut BarState) {
    if st.commands_for.as_deref() == Some(st.query.as_str()) {
        return;
    }
    if !st.recent_loaded {
        // First use: the recents are read here, not at startup.
        st.recent_loaded = true;
        open_store(st);
        st.recent = st.store.as_ref().and_then(|s| s.setting(SET_RECENT)).map_or_else(Vec::new, |v| commands::decode_recent(&v));
    }
    let started = Instant::now();
    st.commands = commands::search(command_text(&st.query).unwrap_or_default(), &st.ctx, &st.recent);
    st.search_ms = started.elapsed().as_secs_f64() * 1000.0;
    // New text: start from the top.
    (st.selected, st.first_row) = (0, 0);
    st.commands_for = Some(st.query.clone());
}

/// Enter (or a double click) on a command. True when the bar should close.
fn run_match(st: &mut BarState, m: &Match) -> bool {
    let Some(command) = commands::command(m.id) else { return false };
    if m.needs_arg {
        // Not run: the name goes into the query for the number to be typed after it.
        st.query = format!(">{} ", command.title);
        (st.caret_end, st.request_focus, st.commands_for) = (true, true, None);
        return false;
    }
    commands::push_recent(&mut st.recent, m.id);
    if let Some(store) = st.store.as_ref() {
        let _ = store.set_setting(SET_RECENT, &commands::encode_recent(&st.recent));
    }
    match m.id.bar_target() {
        // Another mode of the bar itself: it switches in place and stays open.
        Some(BarTarget::Search(text)) => {
            st.query = text.to_owned();
            (st.caret_end, st.request_focus, st.searched) = (true, true, None);
            false
        }
        // The window changes size: the root does the switch (see `Event::Capture`).
        Some(BarTarget::Capture) => {
            st.outbox.push(Outbox::Command(m.id, None));
            false
        }
        None => {
            st.outbox.push(Outbox::Command(m.id, m.arg));
            true
        }
    }
}

/// The result list in command mode: header, rows, footer. `keys` are Esc, Enter,
/// Up and Down, consumed by the caller. True when the bar should close.
fn command_list(ui: &mut Ui, st: &mut BarState, inner: Rect, query_rect: Rect, keys: [bool; 4]) -> bool {
    let [esc, enter, up, down] = keys;
    if esc {
        return true;
    }
    let n = st.commands.len();
    if n > 0 {
        if up {
            st.selected = (st.selected + n - 1) % n;
        }
        if down {
            st.selected = (st.selected + 1) % n;
        }
        if enter {
            let m = st.commands[st.selected].clone();
            if run_match(st, &m) {
                return true;
            }
        }
    }

    // Header.
    let header = Rect::from_min_size(pos2(inner.left(), query_rect.bottom() + 4.0), vec2(inner.width(), 22.0));
    ui.scope_builder(UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("Команды").size(12.0).color(theme::muted()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let stats = match st.notice.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_millis(1500)) {
                Some((text, _)) => text.clone(),
                None => format!("{} · {:.2} мс", n, st.search_ms),
            };
            ui.label(RichText::new(stats).size(11.5).color(theme::muted()));
        });
    });
    if st.notice.as_ref().is_some_and(|(_, t)| t.elapsed() < Duration::from_millis(1500)) {
        ui.ctx().request_repaint_after(Duration::from_millis(1500));
    }

    // Rows.
    let footer_h = 22.0;
    let list = Rect::from_min_max(pos2(inner.left(), header.bottom() + 6.0), pos2(inner.right(), inner.bottom() - footer_h - 4.0));
    let visible_rows = ((list.height() / CMD_ROW_H).floor() as usize).max(1);
    if st.selected < st.first_row {
        st.first_row = st.selected;
    } else if st.selected >= st.first_row + visible_rows {
        st.first_row = st.selected + 1 - visible_rows;
    }
    let wheel = ui.input(|i| if list.contains(i.pointer.hover_pos().unwrap_or_default()) { i.smooth_scroll_delta.y } else { 0.0 });
    if wheel.abs() > 0.5 {
        let max_first = n.saturating_sub(visible_rows);
        st.first_row = if wheel < 0.0 { (st.first_row + 1).min(max_first) } else { st.first_row.saturating_sub(1) };
    }
    if n == 0 {
        ui.painter().text(list.center_top() + vec2(0.0, 40.0), Align2::CENTER_TOP, "Такой команды нет", FontId::proportional(14.0), theme::muted());
    }
    let (mut clicked, mut double) = (None, None);
    for (row, idx) in (st.first_row..n.min(st.first_row + visible_rows)).enumerate() {
        let m = &st.commands[idx];
        let r = Rect::from_min_size(pos2(list.left(), list.top() + row as f32 * CMD_ROW_H), vec2(list.width(), CMD_ROW_H - 4.0));
        let resp = ui.interact(r, ui.id().with(("command", m.id.key())), Sense::click());
        if resp.clicked() {
            clicked = Some(idx);
        }
        if resp.double_clicked() {
            double = Some(idx);
        }
        let selected = idx == st.selected;
        if selected || resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(8), theme::wash(if selected { 20 } else { 9 }));
        }
        // The format of the number the command takes, on the right.
        let mut right = r.right() - 14.0;
        if let Some(Arg::Number { hint, .. }) = commands::command(m.id).map(|c| c.arg) {
            let hint = match m.arg {
                Some(n) => format!("{hint}: {n}"),
                None => hint.to_owned(),
            };
            let g = ui.painter().text(pos2(right, r.center().y), Align2::RIGHT_CENTER, hint, FontId::proportional(12.5), theme::muted());
            right = g.left() - 10.0;
        }
        let galley = ui.painter().layout_job(highlighted(&m.title, 14.5, theme::text(), (right - r.left() - 28.0).max(40.0)));
        ui.painter().galley(pos2(r.left() + 14.0, r.center().y - galley.size().y / 2.0), galley, theme::text());
    }
    if let Some(idx) = clicked {
        st.selected = idx;
    }
    if let Some(idx) = double {
        st.selected = idx;
        let m = st.commands[idx].clone();
        if run_match(st, &m) {
            return true;
        }
    }

    ui.painter().text(
        pos2(inner.left() + 4.0, inner.bottom() - footer_h / 2.0),
        Align2::LEFT_CENTER,
        "↑↓ выбор · Enter — выполнить · Esc · стереть > — к заметкам",
        FontId::proportional(11.5),
        theme::muted(),
    );
    false
}

fn search_ui(ui: &mut Ui, st: &mut BarState) -> bool {
    let in_actions = st.action.is_some();
    let commands_shown = command_text(&st.query).is_some();
    // Disabled while fading out: draw as before, react to nothing.
    let enabled = ui.is_enabled();
    let (esc, enter, up, down, left, right, tab, copy) = ui.input_mut(|i| {
        if !enabled {
            return Default::default();
        }
        // Ctrl+C copies the selected note; among commands it stays the text field's.
        let copy = !commands_shown && i.events.iter().any(|e| matches!(e, egui::Event::Copy));
        if !commands_shown {
            i.events.retain(|e| !matches!(e, egui::Event::Copy));
        }
        (
            i.consume_key(Modifiers::NONE, Key::Escape),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            in_actions && i.consume_key(Modifiers::NONE, Key::ArrowLeft),
            in_actions && i.consume_key(Modifiers::NONE, Key::ArrowRight),
            i.consume_key(Modifiers::NONE, Key::Tab),
            copy,
        )
    });

    let inner = ui.max_rect().shrink2(vec2(16.0, 12.0));

    // Query row.
    let query_rect = Rect::from_min_size(inner.min, vec2(inner.width(), 34.0));
    let mut edit = None;
    ui.scope_builder(UiBuilder::new().max_rect(query_rect).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.add_space(4.0);
        let icon = if commands_shown { "\u{E756}" } else { "\u{E721}" };
        ui.label(RichText::new(icon).font(theme::icons(16.0)).color(theme::dim()));
        ui.add_space(8.0);
        if std::mem::take(&mut st.caret_end) {
            let at = CCursor::new(st.query.chars().count());
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), query_id()).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::one(at)));
            state.store(ui.ctx(), query_id());
        }
        edit = Some(
            ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .id(query_id())
                    .hint_text("Найти заметку: текст, #тег, «идеи прошлого месяца»")
                    .font(FontId::proportional(18.0))
                    .text_color(theme::text())
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            ),
        );
    });
    if let Some(edit) = edit.as_ref().filter(|_| enabled) {
        if std::mem::take(&mut st.request_focus) || !edit.has_focus() {
            edit.request_focus();
        }
        if edit.changed() {
            st.action = None;
        }
    }
    // The palette (`>` typed, or removed again) takes the list over from the notes.
    let commands_now = command_text(&st.query).is_some();
    if commands_now != st.was_command {
        st.was_command = commands_now;
        (st.selected, st.first_row, st.action) = (0, 0, None);
        // The hits are for a query typed before the palette; look again.
        st.searched = None;
    }
    if commands_now {
        refresh_commands(st);
        return command_list(ui, st, inner, query_rect, [esc, enter, up, down]);
    }
    run_search(st);

    // Keyboard.
    let n = st.hits.len();
    if esc {
        if st.action.is_some() {
            st.action = None;
        } else {
            return true;
        }
    }
    if n > 0 {
        if up {
            st.selected = (st.selected + n - 1) % n;
            st.action = None;
        }
        if down {
            st.selected = (st.selected + 1) % n;
            st.action = None;
        }
        if tab {
            st.action = if st.action.is_some() { None } else { Some(0) };
        }
        if let Some(a) = st.action.as_mut() {
            if left {
                *a = (*a + 4) % 5;
            }
            if right {
                *a = (*a + 1) % 5;
            }
        }
        if copy && perform(ui, st, ActionKind::Copy) {
            return true;
        }
        if enter {
            let kind = match st.action {
                Some(a) => actions(&st.hits[st.selected])[a].0,
                None => ActionKind::Open,
            };
            if perform(ui, st, kind) {
                return true;
            }
        }
    }

    // Filters understood + stats.
    let chips_rect = Rect::from_min_size(pos2(inner.left(), query_rect.bottom() + 4.0), vec2(inner.width(), 22.0));
    ui.scope_builder(UiBuilder::new().max_rect(chips_rect).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.add_space(4.0);
        let chips = st.parsed.chips();
        if st.query.trim().is_empty() {
            ui.label(RichText::new("Недавние").size(12.0).color(theme::muted()));
        }
        for chip in chips {
            let galley = ui.painter().layout_no_wrap(chip, FontId::proportional(12.0), theme::dim());
            let (r, _) = ui.allocate_exact_size(galley.size() + vec2(14.0, 6.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(7), theme::highlight(45));
            ui.painter().galley(r.min + vec2(7.0, 3.0), galley, theme::text());
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let stats = match (st.notice.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_millis(1500)), st.latency_ms) {
                (Some((text, _)), _) => text.clone(),
                (None, Some(l)) => format!("{} · {:.1} мс · открыто за {l:.0} мс", n, st.search_ms),
                (None, None) => format!("{} · {:.1} мс", n, st.search_ms),
            };
            ui.label(RichText::new(stats).size(11.5).color(theme::muted()));
        });
    });
    if st.notice.as_ref().is_some_and(|(_, t)| t.elapsed() < Duration::from_millis(1500)) {
        ui.ctx().request_repaint_after(Duration::from_millis(1500));
    }

    // Results.
    let footer_h = 22.0;
    let list = Rect::from_min_max(pos2(inner.left(), chips_rect.bottom() + 6.0), pos2(inner.right(), inner.bottom() - footer_h - 4.0));
    let visible_rows = ((list.height() / ROW_H).floor() as usize).max(1);
    if st.selected < st.first_row {
        st.first_row = st.selected;
    } else if st.selected >= st.first_row + visible_rows {
        st.first_row = st.selected + 1 - visible_rows;
    }
    let wheel = ui.input(|i| if list.contains(i.pointer.hover_pos().unwrap_or_default()) { i.smooth_scroll_delta.y } else { 0.0 });
    if wheel.abs() > 0.5 {
        let max_first = n.saturating_sub(visible_rows);
        st.first_row = if wheel < 0.0 { (st.first_row + 1).min(max_first) } else { st.first_row.saturating_sub(1) };
    }

    if n == 0 {
        let msg = if st.query.trim().is_empty() { "Заметок пока нет" } else { "Ничего не нашлось. Попробуй часть слова или #тег." };
        ui.painter().text(list.center_top() + vec2(0.0, 40.0), Align2::CENTER_TOP, msg, FontId::proportional(14.0), theme::muted());
    }

    let mut clicked = None;
    let mut double = None;
    for (row, idx) in (st.first_row..n.min(st.first_row + visible_rows)).enumerate() {
        let r = Rect::from_min_size(pos2(list.left(), list.top() + row as f32 * ROW_H), vec2(list.width(), ROW_H - 4.0));
        let resp = ui.interact(r, ui.id().with(("hit", st.hits[idx].id)), Sense::click());
        if resp.clicked() {
            clicked = Some(idx);
        }
        if resp.double_clicked() {
            double = Some(idx);
        }
        let selected = idx == st.selected;
        hit_row(ui, r, &st.hits[idx], &st.parsed, selected, resp.hovered(), selected.then_some(st.action).flatten());
    }
    if let Some(idx) = clicked {
        st.selected = idx;
        st.action = None;
    }
    if let Some(idx) = double {
        st.selected = idx;
        if perform(ui, st, ActionKind::Open) {
            return true;
        }
    }

    // Footer.
    let hint = if st.action.is_some() {
        "←→ действие · Enter — выполнить · Tab или Esc — назад"
    } else {
        "↑↓ выбор · Enter — на слой · Ctrl+C — копировать · Tab — действия · Esc"
    };
    ui.painter().text(
        pos2(inner.left() + 4.0, inner.bottom() - footer_h / 2.0),
        Align2::LEFT_CENTER,
        hint,
        FontId::proportional(11.5),
        theme::muted(),
    );
    false
}

/// Text with search markers as a single-line job; matched words get a tinted background.
pub(crate) fn highlighted(text: &str, size: f32, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob {
        wrap: TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') },
        ..Default::default()
    };
    let mut hi = false;
    for part in text.split([MARK_START, MARK_END]) {
        // split alternates plain / marked because markers always come in pairs.
        job.append(
            part,
            0.0,
            TextFormat {
                font_id: FontId::proportional(size),
                color: if hi { theme::text() } else { color },
                background: if hi { theme::highlight(70) } else { Color32::TRANSPARENT },
                ..Default::default()
            },
        );
        hi = !hi;
    }
    job
}

#[allow(clippy::too_many_arguments)]
fn hit_row(ui: &Ui, r: Rect, hit: &Hit, q: &search::Query, selected: bool, hovered: bool, action: Option<usize>) {
    let painter = ui.painter();
    if selected || hovered {
        let fill = if selected { theme::wash(20) } else { theme::wash(9) };
        painter.rect_filled(r, CornerRadius::same(8), fill);
    }
    let accent = hit.kind.accent();
    painter.text(r.left_top() + vec2(14.0, 12.0), Align2::LEFT_TOP, hit.kind.icon(), theme::icons(14.0), accent);

    // Right side: badges and age.
    let mut right = r.right() - 12.0;
    let age = painter.text(pos2(right, r.top() + 13.0), Align2::RIGHT_TOP, age(hit.updated_at), FontId::proportional(11.5), theme::muted());
    right = age.left() - 8.0;
    if hit.archived {
        let g = painter.layout_no_wrap("архив".into(), FontId::proportional(11.0), theme::dim());
        let chip = Rect::from_min_size(pos2(right - g.size().x - 10.0, r.top() + 11.0), g.size() + vec2(10.0, 4.0));
        painter.rect_filled(chip, CornerRadius::same(6), theme::chip_fill());
        painter.galley(chip.min + vec2(5.0, 2.0), g, theme::dim());
        right = chip.left() - 6.0;
    }
    if hit.pinned {
        let p = painter.text(pos2(right, r.top() + 13.0), Align2::RIGHT_TOP, "\u{E841}", theme::icons(11.5), accent);
        right = p.left() - 6.0;
    }

    let text_left = r.left() + 40.0;
    let width = (right - text_left - 8.0).max(40.0);
    let title = if hit.title.is_empty() { String::new() } else { search::snippet(&hit.title, q, 200) };
    let (line1, line2) = match (title.is_empty(), hit.kind == Kind::Private) {
        (false, true) => (title, "••••••••••".to_owned()),
        (true, true) => ("Private".to_owned(), "••••••••••".to_owned()),
        (false, false) => (title, hit.snippet.clone()),
        (true, false) => (hit.snippet.clone(), String::new()),
    };
    let g1 = painter.layout_job(highlighted(&line1, 14.5, theme::text(), width));
    painter.galley(pos2(text_left, r.top() + 8.0), g1, theme::text());
    if action.is_some() {
        // Action strip replaces the second line.
        let mut x = text_left;
        for (i, (_, label)) in actions(hit).iter().enumerate() {
            let g = painter.layout_no_wrap(label.to_string(), FontId::proportional(12.0), theme::text());
            let chip = Rect::from_min_size(pos2(x, r.top() + 29.0), g.size() + vec2(14.0, 5.0));
            let on = action == Some(i);
            let fill = if on { theme::highlight(110) } else { theme::wash(14) };
            painter.rect_filled(chip, CornerRadius::same(6), fill);
            painter.galley(chip.min + vec2(7.0, 2.5), g, if on { theme::text() } else { theme::dim() });
            x = chip.right() + 6.0;
        }
    } else if !line2.is_empty() {
        let g2 = painter.layout_job(highlighted(&line2, 12.5, theme::dim(), width));
        painter.galley(pos2(text_left, r.top() + 30.0), g2, theme::dim());
    }
}

pub(crate) fn age(ts: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let days = (now - ts).max(0) / 86_400;
    match days {
        0 => "сегодня".into(),
        1 => "вчера".into(),
        d if d < 30 => format!("{d} дн"),
        d if d < 365 => format!("{} мес", d / 30),
        d => format!("{} г", d / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_hotkey_opens_search_with_the_prompt_typed() {
        let mut bar = BarState::default();
        assert!(matches!(bar.press_palette(Instant::now()), Press::Show));
        assert_eq!((bar.mode, bar.query.as_str(), bar.caret_end), (Mode::Search, ">", true));
        // A second press closes it.
        assert!(matches!(bar.press_palette(Instant::now()), Press::Hide));
        assert!(!bar.visible);
    }

    #[test]
    fn palette_hotkey_turns_an_open_search_into_the_palette() {
        let mut bar = BarState::default();
        bar.press(Mode::Search, Instant::now());
        bar.query = "идеи".into();
        assert!(matches!(bar.press_palette(Instant::now()), Press::Switch));
        assert_eq!(bar.query, ">");
        // From capture too.
        let mut bar = BarState::default();
        bar.press(Mode::Capture, Instant::now());
        assert!(matches!(bar.press_palette(Instant::now()), Press::Switch));
        assert_eq!(bar.mode, Mode::Search);
    }

    #[test]
    fn only_a_leading_prompt_makes_a_command_query() {
        assert_eq!(command_text(">архив"), Some("архив"));
        assert_eq!(command_text("  > архив"), Some(" архив"));
        assert_eq!(command_text("архив >"), None);
        assert_eq!(command_text(""), None);
    }

    #[test]
    fn enter_on_a_command_that_needs_a_number_asks_for_it() {
        let mut st = BarState { ctx: Ctx { layer_visible: true, tint: 70, ..Ctx::default() }, ..BarState::default() };
        st.recent_loaded = true;
        st.query = ">затемнение".into();
        refresh_commands(&mut st);
        let m = st.commands[0].clone();
        assert!(!run_match(&mut st, &m));
        assert_eq!(st.query, ">Затемнение слоя ");
        assert!(st.outbox.is_empty());
        // Typing the number makes it runnable and closes the bar.
        st.query.push_str("40");
        refresh_commands(&mut st);
        let m = st.commands[0].clone();
        assert!(run_match(&mut st, &m));
        assert!(matches!(st.outbox[..], [Outbox::Command(CommandId::Dimming, Some(40))]));
    }

    #[test]
    fn commands_that_open_the_bar_do_not_close_it() {
        let mut st = BarState { ctx: Ctx { layer_visible: true, ..Ctx::default() }, ..BarState::default() };
        st.recent_loaded = true;
        st.query = ">идеи".into();
        refresh_commands(&mut st);
        let m = st.commands[0].clone();
        assert_eq!(m.id, CommandId::ShowIdeas);
        assert!(!run_match(&mut st, &m));
        assert_eq!(st.query, "идеи ");
        assert!(st.outbox.is_empty());
        // The capture is switched by the root, which resizes the window.
        st.query = ">новая".into();
        refresh_commands(&mut st);
        let m = st.commands[0].clone();
        assert!(!run_match(&mut st, &m));
        assert!(matches!(st.outbox[..], [Outbox::Command(CommandId::NewNote, None)]));
    }
}
