//! The library window: archive, trash and settings.
//!
//! Unlike the bar it is created only while open (it's not latency-critical), so
//! closing it destroys the window, its GL surface and its database connection.
//! Changes that affect the layer go to the root viewport as [`Request`]s.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Layout, Modifiers, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, ViewportCommand, ViewportId, pos2, vec2,
};

use crate::autostart;
use crate::bar::{age, highlighted};
use crate::card::Kind;
use crate::search;
use crate::store::{Hit, Scope, Store, TRASH_DAYS};
use crate::theme;
use crate::win::{self, Backdrop};

pub const SIZE: Vec2 = vec2(800.0, 600.0);
pub const SETTINGS_SIZE: Vec2 = vec2(600.0, 680.0);
const ROW_H: f32 = 58.0;
const HEADER_H: f32 = 52.0;
const RESULTS: usize = 200;

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("library")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Archive,
    Trash,
    Settings,
}

/// Layer settings as the root viewport has them; shown and edited here.
#[derive(Clone, Debug)]
pub struct LayerSettings {
    pub monitor_device: Option<String>,
    pub backdrop: Backdrop,
    pub tint: u8,
    pub pin_bottom: bool,
    pub capture_hotkey: Option<&'static str>,
    pub search_hotkey: Option<&'static str>,
    /// Esc / tray click on a summoned layer hides it instead of sending it back.
    pub dismiss_hides: bool,
    /// The layer collapses to a notch at the top edge; else to a tab at the bottom.
    pub curtain_top: bool,
    pub settings_on_launch: bool,
    /// Cards stick to each other's edges while dragged.
    pub snap: bool,
    /// How cards show their kind on the layer.
    pub card_style: crate::card::CardStyle,
    pub theme_mode: theme::ThemeMode,
}

impl Default for LayerSettings {
    fn default() -> Self {
        Self {
            monitor_device: None,
            backdrop: Backdrop::AccentAcrylic,
            tint: 70,
            pin_bottom: true,
            capture_hotkey: None,
            search_hotkey: None,
            dismiss_hides: false,
            curtain_top: true,
            settings_on_launch: true,
            snap: true,
            card_style: crate::card::CardStyle::default(),
            theme_mode: theme::ThemeMode::System,
        }
    }
}

/// Requests for the root viewport.
pub enum Request {
    /// Put the card on the layer (restoring from the archive) and show it.
    Open(i64),
    /// Cards changed in the database: reload the layer.
    Changed,
    SetMonitor(String),
    SetBackdrop(Backdrop),
    SetTint(u8),
    SetPinBottom(bool),
    SetDismissHides(bool),
    SetCurtainTop(bool),
    SetSettingsOnLaunch(bool),
    SetSnap(bool),
    SetCardStyle(crate::card::CardStyle),
    SetTheme(theme::ThemeMode),
    ImportSticky,
}

#[derive(Default)]
enum AutostartUi {
    #[default]
    Unknown,
    Busy,
    Known(Result<autostart::Status, String>),
}

#[derive(Clone, Copy, PartialEq)]
enum Confirm {
    Purge(i64),
    EmptyTrash,
}

#[derive(Default)]
pub struct LibraryState {
    pub open: bool,
    /// Moved to the cursor's monitor and given its backdrop: safe to show.
    pub placed: bool,
    pub hwnd: Option<isize>,
    pub tab: Tab,
    /// Opened as the settings window: no archive and trash tabs.
    pub settings_only: bool,
    /// First run: the settings window greets the user.
    pub welcome: bool,
    pub settings: LayerSettings,
    pub outbox: Vec<Request>,

    store: Option<Store>,
    query: String,
    searched: Option<(Tab, String)>,
    parsed: search::Query,
    hits: Vec<Hit>,
    selected: usize,
    first_row: usize,
    counts: (i64, i64, i64),
    confirm: Option<Confirm>,
    notice: Option<(String, Instant)>,
    request_focus: bool,
    autostart: Arc<Mutex<AutostartUi>>,
    monitors: Vec<win::Monitor>,
}

impl LibraryState {
    pub fn open(&mut self, tab: Tab) {
        if !self.open {
            self.placed = false;
            self.hwnd = None;
            *self.autostart.lock().unwrap() = AutostartUi::Unknown;
            self.monitors = win::monitors();
        }
        self.open = true;
        self.tab = tab;
        self.settings_only = tab == Tab::Settings;
        self.searched = None;
        self.request_focus = true;
    }

    pub fn size(&self) -> Vec2 {
        if self.settings_only { SETTINGS_SIZE } else { SIZE }
    }

    fn close(&mut self) {
        self.open = false;
        self.welcome = false;
        // Drop the connection and results with the window.
        self.store = None;
        self.hits = Vec::new();
        self.query.clear();
        self.confirm = None;
    }

    /// Data changed elsewhere (layer, search bar): refresh on the next frame.
    pub fn invalidate(&mut self) {
        self.searched = None;
    }
}

fn autostart_job(state: &Arc<Mutex<AutostartUi>>, ctx: &egui::Context, change: Option<bool>) {
    *state.lock().unwrap() = AutostartUi::Busy;
    let (state, ctx) = (state.clone(), ctx.clone());
    std::thread::spawn(move || {
        let changed = match change {
            Some(true) => autostart::enable(),
            Some(false) => autostart::disable(),
            None => Ok(()),
        };
        let result = changed.and_then(|_| autostart::status()).map_err(|e| e.message());
        *state.lock().unwrap() = AutostartUi::Known(result);
        ctx.request_repaint();
    });
}

pub fn ui(ui: &mut Ui, state: &Mutex<LibraryState>) {
    let mut st = state.lock().unwrap();
    if !st.open {
        return;
    }
    let (close_requested, esc) = ui.input(|i| (i.viewport().close_requested(), i.key_pressed(Key::Escape)));
    let next_tab = !st.settings_only && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab));
    if next_tab {
        st.tab = match st.tab {
            Tab::Archive => Tab::Trash,
            Tab::Trash => Tab::Settings,
            Tab::Settings => Tab::Archive,
        };
        st.query.clear();
        st.searched = None;
        st.selected = 0;
        st.first_row = 0;
        st.confirm = None;
        st.request_focus = true;
    }

    let full = ui.max_rect();
    ui.painter().rect(
        full,
        CornerRadius::same(12),
        theme::window_fill(170),
        Stroke::new(1.0, theme::glass_stroke()),
        StrokeKind::Inside,
    );

    let mut close = header(ui, &mut st);
    let body = Rect::from_min_max(pos2(full.left() + 20.0, full.top() + HEADER_H + 8.0), full.max - vec2(20.0, 16.0));
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| match st.tab {
        Tab::Archive | Tab::Trash => list_tab(ui, &mut st),
        Tab::Settings => settings_tab(ui, &mut st),
    });

    if esc {
        if st.confirm.is_some() {
            st.confirm = None;
        } else {
            close = true;
        }
    }
    if close || close_requested {
        st.close();
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    }
    if !st.outbox.is_empty() {
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    }
}

/// Title, tabs, close button; the empty part drags the window. Returns "close".
fn header(ui: &mut Ui, st: &mut LibraryState) -> bool {
    let full = ui.max_rect();
    let bar = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
    let drag = ui.interact(bar, Id::new("library-drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    let mut close = false;
    ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(20.0, 10.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        let title = match (st.settings_only, st.welcome) {
            (false, _) => "Библиотека",
            (true, false) => "Настройки Ebb",
            (true, true) => "Добро пожаловать в Ebb",
        };
        ui.label(RichText::new(title).font(theme::semibold(17.0)).color(theme::text()));
        ui.add_space(18.0);
        let (_, archived, trashed) = st.counts;
        let tabs = if st.settings_only {
            Vec::new()
        } else {
            vec![
                (Tab::Archive, format!("Архив {archived}")),
                (Tab::Trash, format!("Корзина {trashed}")),
                (Tab::Settings, "Настройки".to_owned()),
            ]
        };
        for (tab, label) in tabs {
            let on = st.tab == tab;
            let galley = ui.painter().layout_no_wrap(label.clone(), FontId::proportional(14.0), theme::text());
            let (r, resp) = ui.allocate_exact_size(galley.size() + vec2(20.0, 12.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, &label));
            let fill = if on {
                theme::highlight(70)
            } else if resp.hovered() {
                theme::wash(14)
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(r, CornerRadius::same(8), fill);
            ui.painter().galley(r.min + vec2(10.0, 6.0), galley, if on { theme::text() } else { theme::dim() });
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !on {
                st.tab = tab;
                st.query.clear();
                st.searched = None;
                st.selected = 0;
                st.first_row = 0;
                st.confirm = None;
                st.request_focus = true;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(32.0, 28.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Закрыть"));
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), Color32::from_rgba_unmultiplied(232, 17, 35, 160));
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E8BB}", theme::icons(11.0), theme::text());
            close = resp.on_hover_text("Закрыть (Esc)").clicked();
        });
    });
    close
}

// ---------------------------------------------------------------------------
// Archive and trash
// ---------------------------------------------------------------------------

fn refresh(st: &mut LibraryState) {
    let key = (st.tab, st.query.clone());
    if st.searched.as_ref() == Some(&key) {
        return;
    }
    if st.store.is_none() {
        st.store = Store::open().ok();
    }
    let Some(store) = st.store.as_ref() else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    st.parsed = search::parse(&st.query, now, search::local_offset_secs());
    let scope = if st.tab == Tab::Trash { Scope::Trash } else { Scope::Archive };
    let selected_id = st.hits.get(st.selected).map(|h| h.id);
    st.hits = store.search(&st.parsed, scope, RESULTS).unwrap_or_default();
    st.counts = store.counts().unwrap_or_default();
    let same_view = st.searched.as_ref().is_some_and(|(t, q)| *t == key.0 && *q == key.1);
    st.selected = match (same_view, selected_id) {
        (true, Some(id)) => st.hits.iter().position(|h| h.id == id).unwrap_or(st.selected),
        _ => st.selected,
    }
    .min(st.hits.len().saturating_sub(1));
    st.searched = Some(key);
}

fn days_ago(ts: i64) -> i64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    ((now - ts).max(0)) / 86_400
}

/// Primary: back to the layer / restore. Secondary: to the trash / delete forever.
fn act(st: &mut LibraryState, primary: bool) {
    let Some(hit) = st.hits.get(st.selected).cloned() else { return };
    let Some(store) = st.store.as_ref() else { return };
    match (st.tab, primary) {
        (Tab::Archive, true) => {
            st.outbox.push(Request::Open(hit.id));
            st.notice = Some(("Возвращено на слой".into(), Instant::now()));
        }
        (Tab::Archive, false) => {
            let _ = store.delete(hit.id);
            st.outbox.push(Request::Changed);
            st.notice = Some(("Перемещено в корзину".into(), Instant::now()));
        }
        (Tab::Trash, true) => {
            let _ = store.restore(hit.id);
            st.outbox.push(Request::Changed);
            let place = if hit.archived { "в архив" } else { "на слой" };
            st.notice = Some((format!("Восстановлено {place}"), Instant::now()));
        }
        (Tab::Trash, false) => {
            if st.confirm != Some(Confirm::Purge(hit.id)) {
                st.confirm = Some(Confirm::Purge(hit.id));
                return;
            }
            let _ = store.purge(hit.id);
            st.notice = Some(("Удалено навсегда".into(), Instant::now()));
            st.confirm = None;
        }
        (Tab::Settings, _) => return,
    }
    st.searched = None;
}

fn list_tab(ui: &mut Ui, st: &mut LibraryState) {
    let (up, down, enter, del) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Delete),
        )
    });
    let area = ui.max_rect();
    let trash = st.tab == Tab::Trash;

    // Query row + trash controls.
    let top = Rect::from_min_size(area.min, vec2(area.width(), 36.0));
    let mut empty_clicked = false;
    ui.scope_builder(UiBuilder::new().max_rect(top).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.label(RichText::new("\u{E721}").font(theme::icons(14.0)).color(theme::dim()));
        ui.add_space(6.0);
        let hint = if trash { "Найти в корзине" } else { "Найти в архиве: текст, #тег, «ссылки прошлого года»" };
        let edit = ui.add(
            egui::TextEdit::singleline(&mut st.query)
                .hint_text(hint)
                .font(FontId::proportional(15.0))
                .text_color(theme::text())
                .frame(egui::Frame::NONE)
                .desired_width(if trash { area.width() - 230.0 } else { area.width() - 40.0 }),
        );
        if std::mem::take(&mut st.request_focus) {
            edit.request_focus();
        }
        if edit.changed() {
            st.selected = 0;
            st.first_row = 0;
            st.confirm = None;
        }
        if trash {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if st.confirm == Some(Confirm::EmptyTrash) { "Точно очистить? Нажми ещё раз" } else { "Очистить корзину" };
                empty_clicked = ui.add_enabled(st.counts.2 > 0, egui::Button::new(label)).clicked();
            });
        }
    });
    if empty_clicked {
        if st.confirm == Some(Confirm::EmptyTrash) {
            if let Some(store) = st.store.as_ref() {
                let _ = store.empty_trash();
            }
            st.confirm = None;
            st.notice = Some(("Корзина очищена".into(), Instant::now()));
            st.searched = None;
        } else {
            st.confirm = Some(Confirm::EmptyTrash);
        }
    }
    refresh(st);

    let n = st.hits.len();
    if n > 0 {
        if up {
            st.selected = (st.selected + n - 1) % n;
            st.confirm = None;
        }
        if down {
            st.selected = (st.selected + 1) % n;
            st.confirm = None;
        }
        if enter {
            act(st, true);
        }
        if del {
            act(st, false);
        }
    }

    // Info line.
    let info = Rect::from_min_size(pos2(area.left(), top.bottom() + 2.0), vec2(area.width(), 20.0));
    let notice = st.notice.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_millis(2000)).map(|(s, _)| s.clone());
    if notice.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(2000));
    }
    let chips = st.parsed.chips().join(" · ");
    let total = if trash { st.counts.2 } else { st.counts.1 };
    let found = if st.query.trim().is_empty() {
        format!("{total} {}", if trash { "в корзине" } else { "в архиве" })
    } else {
        format!("найдено {n} из {total}{}", if chips.is_empty() { String::new() } else { format!(" · {chips}") })
    };
    let text = match notice {
        Some(n) => n,
        None if trash => format!("{found} · удалённое хранится {TRASH_DAYS} дней"),
        None => found,
    };
    ui.painter().text(info.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(12.0), theme::muted());

    // Rows.
    let footer_h = 20.0;
    let list = Rect::from_min_max(pos2(area.left(), info.bottom() + 6.0), pos2(area.right(), area.bottom() - footer_h - 4.0));
    let visible = ((list.height() / ROW_H).floor() as usize).max(1);
    if st.selected < st.first_row {
        st.first_row = st.selected;
    } else if st.selected >= st.first_row + visible {
        st.first_row = st.selected + 1 - visible;
    }
    let wheel = ui.input(|i| if list.contains(i.pointer.hover_pos().unwrap_or_default()) { i.smooth_scroll_delta.y } else { 0.0 });
    if wheel.abs() > 0.5 {
        let max_first = n.saturating_sub(visible);
        st.first_row = if wheel < 0.0 { (st.first_row + 1).min(max_first) } else { st.first_row.saturating_sub(1) };
    }
    if n == 0 {
        let msg = match (trash, st.query.trim().is_empty()) {
            (true, true) => "Корзина пуста",
            (false, true) => "В архиве пусто. Сюда уходят карточки со слоя и импорт.",
            _ => "Ничего не нашлось",
        };
        ui.painter().text(list.center_top() + vec2(0.0, 60.0), Align2::CENTER_TOP, msg, FontId::proportional(14.0), theme::muted());
    }

    let mut action = None;
    for (row, idx) in (st.first_row..n.min(st.first_row + visible)).enumerate() {
        let r = Rect::from_min_size(pos2(list.left(), list.top() + row as f32 * ROW_H), vec2(list.width(), ROW_H - 4.0));
        let hit = st.hits[idx].clone();
        let resp = ui.interact(r, Id::new(("library-row", hit.id)), Sense::click());
        if resp.clicked() {
            st.selected = idx;
            st.confirm = None;
        }
        if resp.double_clicked() {
            st.selected = idx;
            action = Some(true);
        }
        let selected = idx == st.selected;
        if let Some(primary) = row_ui(ui, r, &hit, &st.parsed, selected, resp.hovered(), trash, st.confirm == Some(Confirm::Purge(hit.id))) {
            st.selected = idx;
            action = Some(primary);
        }
    }
    if let Some(primary) = action {
        act(st, primary);
    }

    let hint = if trash {
        "↑↓ выбор · Enter — восстановить · Delete — удалить навсегда · Ctrl+Tab — вкладка · Esc — закрыть"
    } else {
        "↑↓ выбор · Enter — на слой · Delete — в корзину · Ctrl+Tab — вкладка · Esc — закрыть"
    };
    ui.painter().text(pos2(area.left(), area.bottom() - footer_h / 2.0), Align2::LEFT_CENTER, hint, FontId::proportional(11.5), theme::muted());
}

/// One result row; returns Some(primary?) when one of its buttons was clicked.
#[allow(clippy::too_many_arguments)]
fn row_ui(ui: &mut Ui, r: Rect, hit: &Hit, q: &search::Query, selected: bool, hovered: bool, trash: bool, confirm: bool) -> Option<bool> {
    let painter = ui.painter().clone();
    if selected || hovered {
        painter.rect_filled(r, CornerRadius::same(8), theme::wash(if selected { 20 } else { 9 }));
    }
    painter.text(r.left_top() + vec2(14.0, 12.0), Align2::LEFT_TOP, hit.kind.icon(), theme::icons(14.0), hit.kind.accent());

    // Buttons on the selected or hovered row, age otherwise.
    let mut clicked = None;
    let mut right = r.right() - 10.0;
    if selected || hovered {
        let labels = if trash {
            [(false, if confirm { "Точно? Ещё раз" } else { "Удалить навсегда" }), (true, "Восстановить")]
        } else {
            [(false, "В корзину"), (true, "На слой")]
        };
        for (primary, label) in labels {
            let galley = painter.layout_no_wrap(label.to_owned(), FontId::proportional(12.5), theme::text());
            let b = Rect::from_min_size(pos2(right - galley.size().x - 18.0, r.center().y - 13.0), vec2(galley.size().x + 18.0, 26.0));
            let resp = ui.interact(b, Id::new(("library-btn", hit.id, primary)), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            let fill = match (primary, resp.hovered()) {
                (true, true) => theme::highlight(130),
                (true, false) => theme::highlight(70),
                (false, true) if trash => Color32::from_rgba_unmultiplied(232, 17, 35, 140),
                (false, true) => theme::wash(30),
                (false, false) => theme::wash(14),
            };
            painter.rect_filled(b, CornerRadius::same(7), fill);
            painter.galley(b.min + vec2(9.0, 13.0 - galley.size().y / 2.0), galley, theme::text());
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                clicked = Some(primary);
            }
            right = b.left() - 6.0;
        }
    } else {
        let label = if trash {
            let deleted = hit.deleted_at.map_or(0, days_ago);
            format!("удалено {} · ещё {} дн", if deleted == 0 { "сегодня".into() } else { format!("{deleted} дн назад") }, (TRASH_DAYS - deleted).max(0))
        } else {
            age(hit.updated_at)
        };
        let g = painter.text(pos2(right, r.top() + 12.0), Align2::RIGHT_TOP, label, FontId::proportional(11.5), theme::muted());
        right = g.left() - 8.0;
    }

    let left = r.left() + 40.0;
    let width = (right - left - 8.0).max(60.0);
    let title = if hit.title.is_empty() { String::new() } else { search::snippet(&hit.title, q, 200) };
    let private = hit.kind == Kind::Private;
    let (line1, line2) = match (title.is_empty(), private) {
        (false, true) => (title, "••••••••••".to_owned()),
        (true, true) => ("Private".to_owned(), "••••••••••".to_owned()),
        (false, false) => (title, hit.snippet.clone()),
        (true, false) => (hit.snippet.clone(), String::new()),
    };
    painter.galley(pos2(left, r.top() + 8.0), painter.layout_job(highlighted(&line1, 14.0, theme::text(), width)), theme::text());
    let second = if line2.is_empty() && trash {
        if hit.archived { "был в архиве".to_owned() } else { "был на слое".to_owned() }
    } else {
        line2
    };
    if !second.is_empty() {
        painter.galley(pos2(left, r.top() + 30.0), painter.layout_job(highlighted(&second, 12.5, theme::dim(), width)), theme::dim());
    }
    clicked
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).font(theme::semibold(14.5)).color(theme::text()));
    ui.add_space(2.0);
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::muted()));
}

/// The monitors as Windows arranges them, to scale; returns the clicked one's
/// device id when it isn't the current monitor.
fn monitor_map(ui: &mut Ui, monitors: &[win::Monitor], current: Option<&str>) -> Option<String> {
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
    let first = monitors.first()?;
    let (mut l, mut t, mut r, mut b) = (first.rect.left, first.rect.top, first.rect.right, first.rect.bottom);
    for m in monitors {
        (l, t, r, b) = (l.min(m.rect.left), t.min(m.rect.top), r.max(m.rect.right), b.max(m.rect.bottom));
    }
    let (w, h) = ((r - l).max(1) as f32, (b - t).max(1) as f32);
    let scale = ((area.width() - 8.0) / w).min((area.height() - 8.0) / h);
    let offset = area.center() - vec2(w, h) * scale / 2.0;

    let mut chosen = None;
    for (i, m) in monitors.iter().enumerate() {
        let min = offset + vec2((m.rect.left - l) as f32, (m.rect.top - t) as f32) * scale;
        let max = offset + vec2((m.rect.right - l) as f32, (m.rect.bottom - t) as f32) * scale;
        let rect = Rect::from_min_max(min, max).shrink(3.0);
        let (pw, ph) = (m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
        let name = format!("Монитор {}", i + 1);
        let on = current.is_some_and(|d| m.matches_device(d));
        let resp = ui.interact(rect, Id::new(("monitor", i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, &name));
        let accent = theme::highlight(255);
        let fill = match (on, resp.hovered()) {
            (true, _) => accent.gamma_multiply(0.35),
            (false, true) => theme::wash(26),
            (false, false) => theme::wash(10),
        };
        let stroke = if on { Stroke::new(2.0, accent) } else { Stroke::new(1.0, theme::glass_stroke()) };
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
        let color = if on { theme::text() } else { theme::dim() };
        painter.text(rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, (i + 1).to_string(), theme::semibold(20.0), color);
        let detail = format!("{pw}×{ph}{}", if m.primary { " · основной" } else { "" });
        painter.text(rect.center() + vec2(0.0, 13.0), Align2::CENTER_CENTER, detail, FontId::proportional(11.5), theme::muted());
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !on {
            chosen = m.device_ids.first().cloned();
        }
    }
    chosen
}

/// Theme glass, the taskbar's color, or a color of one's own: paper swatches
/// and a picker.
fn background_picker(ui: &mut Ui, background: &mut theme::CardBackground) {
    use theme::CardBackground as B;
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(background, B::Theme, "По теме");
        ui.radio_value(background, B::Taskbar, "Как панель задач");
        let custom = matches!(background, B::Custom(_));
        if ui.radio(custom, "Свой цвет").clicked() && !custom {
            *background = B::Custom(theme::PAPER[0]);
        }
    });
    match background {
        B::Custom(rgb) => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for paper in theme::PAPER {
                    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                    let on = *rgb == paper;
                    let stroke = if on { Stroke::new(2.0, theme::highlight(255)) } else { Stroke::new(1.0, theme::glass_stroke()) };
                    ui.painter().rect(rect.shrink(2.0), CornerRadius::same(4), Color32::from_rgb(paper[0], paper[1], paper[2]), stroke, StrokeKind::Outside);
                    let name = format!("#{:02X}{:02X}{:02X}", paper[0], paper[1], paper[2]);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, &name));
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        *rgb = paper;
                    }
                }
                ui.add_space(6.0);
                egui::widgets::color_picker::color_edit_button_srgb(ui, rgb);
            });
            note(ui, "Цвет текста подбирается сам: тёмный на светлом фоне, светлый на тёмном.");
        }
        B::Taskbar => note(ui, "Берётся из «Параметры → Персонализация → Цвета» и меняется вместе с Windows."),
        B::Theme => {}
    }
}

/// A preset to pick: three tiny cards (an idea, a link, a private note) drawn in
/// its style, with its name under them.
fn preset_tile(ui: &mut Ui, preset: &crate::card::Preset, on: bool) -> egui::Response {
    use crate::card::Kind;
    let (rect, resp) = ui.allocate_exact_size(vec2(172.0, 112.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, preset.name));
    let fill = match (on, resp.hovered()) {
        (true, _) => theme::highlight(40),
        (false, true) => theme::wash(20),
        (false, false) => theme::wash(8),
    };
    let stroke = if on { Stroke::new(1.5, theme::highlight(255)) } else { Stroke::new(1.0, theme::glass_stroke()) };
    ui.painter().rect(rect, CornerRadius::same(8), fill, stroke, StrokeKind::Inside);

    // Scaled down: corners and strips at half size so the look still reads.
    let mut style = preset.style;
    style.radius /= 2;
    style.strip = (style.strip / 2).max(2);
    let base = match style.background {
        theme::CardBackground::Theme => theme::glass_fill(),
        theme::CardBackground::Taskbar if theme::is_light() => Color32::from_rgb(238, 238, 238),
        theme::CardBackground::Taskbar => Color32::from_rgb(32, 32, 32),
        theme::CardBackground::Custom([r, g, b]) => Color32::from_rgb(r, g, b),
    };
    let base = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), (f32::from(style.opacity) * 2.55) as u8);
    for (i, kind) in [Kind::Idea, Kind::Link, Kind::Private].into_iter().enumerate() {
        let card = Rect::from_min_size(rect.min + vec2(10.0 + i as f32 * 52.0, 10.0), vec2(48.0, 58.0));
        if style.shadow > 0 {
            let shadow = egui::epaint::Shadow { offset: [0, 2], blur: (f32::from(style.shadow) / 8.0) as u8, spread: 0, color: Color32::from_black_alpha(style.shadow) };
            ui.painter().add(shadow.as_shape(card, CornerRadius::same(style.radius)));
        }
        ui.painter().rect(card, CornerRadius::same(style.radius), base, Stroke::new(1.0, theme::glass_stroke()), StrokeKind::Inside);
        let accent = kind.accent();
        crate::app::paint_marker(ui, card, accent, style, false);
        for line in 0..3 {
            let y = card.top() + 12.0 + line as f32 * 8.0;
            let w = [30.0, 22.0, 26.0][line];
            ui.painter().rect_filled(Rect::from_min_size(pos2(card.left() + 6.0, y), vec2(w, 3.0)), CornerRadius::same(1), theme::wash(60));
        }
        let size = if style.bold_icon { 11.0 } else { 9.0 };
        let at = match style.icon {
            crate::card::IconSpot::TopLeft => pos2(card.left() + 8.0, card.top() + 4.0),
            _ => pos2(card.right() - 8.0, card.bottom() - 8.0),
        };
        if style.icon != crate::card::IconSpot::Hover {
            ui.painter().text(at, Align2::CENTER_CENTER, kind.icon(), theme::icons(size), kind.accent());
        }
    }
    ui.painter().text(pos2(rect.left() + 10.0, rect.bottom() - 34.0), Align2::LEFT_TOP, preset.name, theme::semibold(13.0), theme::text());
    ui.painter().text(pos2(rect.left() + 10.0, rect.bottom() - 17.0), Align2::LEFT_TOP, preset.hint, FontId::proportional(10.5), theme::muted());
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(preset.hint)
}

fn settings_tab(ui: &mut Ui, st: &mut LibraryState) {
    if st.store.is_none() {
        st.store = Store::open().ok();
    }
    if let Some(store) = st.store.as_ref() {
        st.counts = store.counts().unwrap_or(st.counts);
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;

        if st.welcome {
            ui.label(
                RichText::new("Ebb кладёт заметки на отдельный экран — под окнами, как обои. Выберите, где им лежать; остальное можно не трогать.")
                    .size(13.5)
                    .color(theme::dim()),
            );
            note(ui, "Настройки всегда открываются из меню значка Ebb в трее.");
        }

        section(ui, "Экран слоя");
        if let Some(device) = monitor_map(ui, &st.monitors, st.settings.monitor_device.as_deref()) {
            st.settings.monitor_device = Some(device.clone());
            st.outbox.push(Request::SetMonitor(device));
        }
        note(ui, "Кликните по монитору — слой переедет туда сразу.");

        section(ui, "Слой");
        let mut pin = st.settings.pin_bottom;
        if ui.checkbox(&mut pin, "Держать слой под окнами").changed() {
            st.settings.pin_bottom = pin;
            st.outbox.push(Request::SetPinBottom(pin));
        }
        note(ui, "Клик по значку в трее поднимает слой поверх всех окон; Esc, повторный клик или переход в другое окно возвращают его.");
        ui.label(RichText::new("Esc и повторный клик в трее").size(13.0).color(theme::dim()));
        let mut hides = st.settings.dismiss_hides;
        let a = ui.radio_value(&mut hides, false, "Убрать слой на фон");
        let b = ui.radio_value(&mut hides, true, "Скрыть слой");
        if (a.changed() || b.changed()) && hides != st.settings.dismiss_hides {
            st.settings.dismiss_hides = hides;
            st.outbox.push(Request::SetDismissHides(hides));
        }
        ui.label(RichText::new("Свернуть слой к краю").size(13.0).color(theme::dim()));
        let mut top = st.settings.curtain_top;
        let a = ui.radio_value(&mut top, true, "Сверху — чёлкой у края экрана");
        let b = ui.radio_value(&mut top, false, "Снизу — плашкой над панелью задач");
        if (a.changed() || b.changed()) && top != st.settings.curtain_top {
            st.settings.curtain_top = top;
            st.outbox.push(Request::SetCurtainTop(top));
        }
        note(ui, "Ручка на краю слоя сворачивает его, и рабочий стол под ним снова доступен; клик по свёрнутому слою возвращает его.");

        section(ui, "Оформление");
        use crate::card::{IconSpot, Marker, PRESETS};
        let mut mode = st.settings.theme_mode;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Тема").size(13.0).color(theme::dim()));
            for m in theme::ThemeMode::ALL {
                ui.radio_value(&mut mode, m, m.label());
            }
        });

        ui.add_space(4.0);
        ui.label(RichText::new("Вид карточек").size(13.0).color(theme::dim()));
        let mut style = st.settings.card_style;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for preset in &PRESETS {
                if preset_tile(ui, preset, preset.style == style).clicked() {
                    style = preset.style;
                }
            }
        });
        if !PRESETS.iter().any(|p| p.style == style) {
            note(ui, "Свой вариант: настроен вручную.");
        }

        egui::CollapsingHeader::new(RichText::new("Настроить вручную").size(13.0).color(theme::dim()))
            .id_salt("card-style-manual")
            .show(ui, |ui| {
                ui.label(RichText::new("Цвет типа на карточке").size(13.0).color(theme::dim()));
                ui.horizontal_wrapped(|ui| {
                    for m in Marker::ALL {
                        ui.radio_value(&mut style.marker, m, m.label());
                    }
                });
                if style.marker != Marker::None {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Интенсивность").size(13.0).color(theme::dim()));
                        ui.add(egui::Slider::new(&mut style.strength, 5..=100).suffix(" %"));
                    });
                }
                if matches!(style.marker, Marker::StripTop | Marker::StripLeft) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Толщина полосы").size(13.0).color(theme::dim()));
                        ui.add(egui::Slider::new(&mut style.strip, 2..=10).suffix(" pt"));
                    });
                }
                let slider = |ui: &mut Ui, label: &str, widget: egui::Slider<'_>| {
                    ui.horizontal(|ui| {
                        ui.add_sized(vec2(150.0, 18.0), egui::Label::new(RichText::new(label).size(13.0).color(theme::dim())));
                        ui.add(widget);
                    });
                };
                slider(ui, "Скругление углов", egui::Slider::new(&mut style.radius, 0..=20).suffix(" pt"));
                slider(ui, "Тень", egui::Slider::new(&mut style.shadow, 0..=100).suffix(" %"));
                slider(ui, "Непрозрачность фона", egui::Slider::new(&mut style.opacity, 30..=100).suffix(" %"));
                ui.label(RichText::new("Шрифт текста").size(13.0).color(theme::dim()));
                ui.horizontal_wrapped(|ui| {
                    for f in theme::CardFont::ALL {
                        ui.radio_value(&mut style.font, f, f.label());
                    }
                });
                slider(
                    ui,
                    "Размер текста",
                    egui::Slider::new(&mut style.text, 22..=40).custom_formatter(|v, _| format!("{:.1} pt", v / 2.0)),
                );
                ui.label(RichText::new("Иконка типа").size(13.0).color(theme::dim()));
                ui.horizontal_wrapped(|ui| {
                    for i in IconSpot::ALL {
                        ui.radio_value(&mut style.icon, i, i.label());
                    }
                });
                ui.checkbox(&mut style.bold_icon, "Жирная иконка");
                ui.label(RichText::new("Фон карточек").size(13.0).color(theme::dim()));
                background_picker(ui, &mut style.background);
            });

        if style != st.settings.card_style {
            st.settings.card_style = style;
            st.outbox.push(Request::SetCardStyle(style));
        }
        if mode != st.settings.theme_mode {
            st.settings.theme_mode = mode;
            st.outbox.push(Request::SetTheme(mode));
        }
        note(ui, "Слой меняется сразу. F4 на слое перебирает варианты цвета. Клик по иконке типа меняет тип и цвет карточки.");

        section(ui, "Карточки");
        let mut snap = st.settings.snap;
        if ui.checkbox(&mut snap, "Магнит: прилипать к краям соседних карточек").changed() {
            st.settings.snap = snap;
            st.outbox.push(Request::SetSnap(snap));
        }
        note(ui, "Удерживайте Alt при перетаскивании, чтобы поставить карточку свободно.");

        section(ui, "Запуск");
        let mut on_launch = st.settings.settings_on_launch;
        if ui.checkbox(&mut on_launch, "Открывать это окно при запуске Ebb из ярлыка").changed() {
            st.settings.settings_on_launch = on_launch;
            st.outbox.push(Request::SetSettingsOnLaunch(on_launch));
        }
        {
            let mut state = st.autostart.lock().unwrap();
            if matches!(*state, AutostartUi::Unknown) {
                drop(state);
                autostart_job(&st.autostart, ui.ctx(), None);
                state = st.autostart.lock().unwrap();
            }
            let (mut on, detail) = match &*state {
                AutostartUi::Known(Ok(autostart::Status::On { current_exe, command })) => {
                    (true, (!current_exe).then(|| format!("Запускается другая копия: {command}")))
                }
                AutostartUi::Known(Err(e)) => (false, Some(format!("Не удалось узнать: {e}"))),
                _ => (false, None),
            };
            let busy = matches!(*state, AutostartUi::Busy | AutostartUi::Unknown);
            drop(state);
            if ui.add_enabled(!busy, egui::Checkbox::new(&mut on, "Запускать при входе в Windows")).changed() {
                autostart_job(&st.autostart, ui.ctx(), Some(on));
            }
            note(ui, "Через Планировщик заданий: стартует сразу после входа, без задержки автозагрузки.");
            if let Some(detail) = detail {
                note(ui, &detail);
            }
        }

        section(ui, "Внешний вид");
        ui.label(RichText::new("Фон").size(13.0).color(theme::dim()));
        for backdrop in Backdrop::ALL {
            if ui.radio(st.settings.backdrop == backdrop, backdrop.title()).clicked() && st.settings.backdrop != backdrop {
                st.settings.backdrop = backdrop;
                st.outbox.push(Request::SetBackdrop(backdrop));
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Затемнение").size(13.0).color(theme::dim()));
            let mut tint = st.settings.tint;
            if ui.add(egui::Slider::new(&mut tint, 0..=220).show_value(false)).changed() {
                st.settings.tint = tint;
                st.outbox.push(Request::SetTint(tint));
            }
        });

        section(ui, "Горячие клавиши");
        let key = |k: Option<&'static str>| k.unwrap_or("занята другой программой");
        note(ui, &format!("Записать мысль: {}", key(st.settings.capture_hotkey)));
        note(ui, &format!("Поиск: {}", key(st.settings.search_hotkey)));
        note(ui, "Если сочетание занято, берётся следующее свободное. Выбор своих сочетаний появится позже.");

        section(ui, "Импорт");
        if ui.button("Импорт из Sticky Notes…").clicked() {
            st.outbox.push(Request::ImportSticky);
        }
        note(ui, "Открытые заметки встанут на слой как на экране, остальные уйдут в архив. Повторный импорт заменит прошлый, изменённые тобой карточки останутся.");

        section(ui, "Данные");
        let (layer, archived, trashed) = st.counts;
        note(ui, &format!("На слое {layer}, в архиве {archived}, в корзине {trashed}."));
        let path = crate::store::db_path();
        ui.horizontal(|ui| {
            note(ui, &path.display().to_string());
            if ui.small_button("Открыть папку").clicked() {
                if let Some(dir) = path.parent() {
                    let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
                }
            }
        });
    });
}
