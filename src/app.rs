//! The ambient layer (root viewport); the capture/search bar lives in `bar`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Layout, Modifiers, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId, pos2, vec2,
};

use std::sync::atomic::Ordering;

use crate::bar::{self, BarState, Mode, Outbox, Press};
use crate::import_ui::StickyImport;
use crate::library::{self, LibraryState, Request, Tab};
use crate::shell::{self, Event};
use crate::card::{self, Card, Kind, MIN_SIZE, Sides, parse_capture};
use crate::store::Store;
use crate::theme::{self, TEXT, TEXT_DIM, TEXT_MUTED};
use crate::win::{self, Backdrop};

const HEADER_H: f32 = 30.0;
const FOOTER_H: f32 = 22.0;
const REVEAL_FOR: Duration = Duration::from_secs(5);
const HIGHLIGHT_FOR: Duration = Duration::from_millis(2500);
const TOAST_FOR: Duration = Duration::from_secs(6);
/// How long the copy button shows its check mark.
const COPIED_FOR: Duration = Duration::from_millis(1500);
const LAYER_FADE_IN: Duration = Duration::from_millis(220);
const CARD_APPEAR: Duration = Duration::from_millis(220);
const CARD_LEAVE: Duration = Duration::from_millis(160);
pub(crate) const PANEL_APPEAR: Duration = Duration::from_millis(200);
const PANEL_FADE_OUT: Duration = Duration::from_millis(250);

/// Ease-out cubic on a 0..1 progress (clamped).
pub(crate) fn panel_ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}
const LAYER_FADE_OUT: Duration = Duration::from_millis(160);

// Settings keys.
const SET_MONITOR: &str = "layer.monitor";
const SET_BACKDROP: &str = "layer.backdrop";
const SET_TINT: &str = "layer.tint";
const SET_PIN_BOTTOM: &str = "layer.pin_bottom";
/// "hide" or "back": what Esc / a second tray click does to a summoned layer.
const SET_DISMISS: &str = "layer.dismiss";
const SET_SETTINGS_ON_LAUNCH: &str = "app.settings_on_launch";
const SET_ONBOARDED: &str = "app.onboarded";
const SET_SNAP: &str = "cards.snap";
/// A tray click this soon after another app took the focus from a summoned layer
/// (the taskbar does, on mouse down) counts as a click on the summoned layer.
const TRAY_CLICK_AFTER_LOWER_MS: u64 = 500;

#[derive(Clone, Copy)]
enum Undo {
    Unarchive(i64),
    Restore(i64),
}

struct Toast {
    text: String,
    undo: Option<Undo>,
    at: Instant,
}

impl Toast {
    fn new(text: impl Into<String>, undo: Option<Undo>) -> Self {
        Self { text: text.into(), undo, at: Instant::now() }
    }
}

#[derive(Clone, Copy)]
enum MenuItem {
    Capture,
    Search,
    Library,
    Import,
    Settings,
    Dismiss,
    Exit,
}

enum Action {
    Front,
    Moved,
    TogglePin,
    Copy,
    Duplicate,
    Archive,
    Delete,
    StartEdit,
    Reveal,
}

pub struct EbbApp {
    store: Store,
    cards: Vec<Card>,
    bar: Arc<Mutex<BarState>>,
    shell: Arc<shell::Shared>,
    /// Set by fade-out threads; logic() then hides the window through eframe.
    bar_faded_out: Arc<std::sync::atomic::AtomicBool>,
    layer_faded_out: Arc<std::sync::atomic::AtomicBool>,

    hwnd: Option<isize>,
    layer_visible: bool,
    backdrop: Backdrop,
    tint: u8,
    dismiss_hides: bool,
    settings_on_launch: bool,
    snap: bool,
    /// Started by the user (not at logon): bring the layer up, maybe open settings.
    manual_start: bool,
    library: Arc<Mutex<LibraryState>>,

    editing: Option<(i64, String)>,
    /// Last card clicked or dragged: shows its details until something else is clicked.
    active: Option<i64>,
    revealed: Option<(i64, Instant)>,
    /// Card just opened from search: outlined for a moment.
    highlighted: Option<(i64, Instant)>,
    toast: Option<Toast>,
    /// Cards that just arrived on the layer / just left it, for their animations.
    appearing: Vec<(i64, Instant)>,
    leaving: Vec<(Card, Instant)>,

    sticky: StickyImport,

    show_debug: bool,
    autostarted: bool,
    main_started: Instant,
    first_frame: Option<(f64, f64)>,
    frames: u64,
}

impl EbbApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Store,
        cards: Vec<Card>,
        main_started: Instant,
        autostarted: bool,
    ) -> Self {
        theme::install(&cc.egui_ctx);

        let hwnd = win::hwnd_of(cc);
        // Saved layer settings. DWM acrylic turns flat grey when the window is
        // inactive; the accent one stays blurred, hence the default.
        let backdrop = store.setting(SET_BACKDROP).as_deref().and_then(Backdrop::from_key).unwrap_or(Backdrop::AccentAcrylic);
        let tint = store.setting(SET_TINT).and_then(|v| v.parse().ok()).unwrap_or(70);
        let pin_bottom = store.setting(SET_PIN_BOTTOM).is_none_or(|v| v != "0");
        win::PIN_BOTTOM.store(pin_bottom, Ordering::Relaxed);
        let dismiss_hides = store.setting(SET_DISMISS).is_some_and(|v| v == "hide");
        let settings_on_launch = store.setting(SET_SETTINGS_ON_LAUNCH).is_none_or(|v| v != "0");
        let snap = store.setting(SET_SNAP).is_none_or(|v| v != "0");
        let manual_start = !autostarted && crate::bench::path().is_none();
        // Launched from a shortcut: shown over the windows, not under them.
        win::RAISED.store(manual_start, Ordering::Relaxed);
        if let Some(h) = hwnd {
            let monitors = win::monitors();
            let saved = store.setting(SET_MONITOR);
            // The saved monitor if it's still connected, else the first secondary one.
            let monitor = saved.as_deref().and_then(|d| monitors.iter().find(|m| m.matches_device(d))).or(monitors.first());
            if let Some(m) = monitor {
                win::place_on(h, m);
            }
            win::apply_backdrop(h, backdrop, false);
            // Still hidden here (eframe shows it after the first frame), so the
            // taskbar never sees a button. Starts transparent; fades in after the
            // first frame.
            win::install_window_rules(h, win::LAYER | win::FADE);
            win::set_window_alpha(h, 0);
        }

        let shell = Arc::new(shell::Shared::default());
        shell.layer_visible.store(true, Ordering::Relaxed);
        shell.pin_bottom.store(pin_bottom, Ordering::Relaxed);
        shell::spawn(cc.egui_ctx.clone(), shell.clone());

        Self {
            store,
            cards,
            bar: Arc::default(),
            shell,
            bar_faded_out: Arc::default(),
            layer_faded_out: Arc::default(),
            hwnd,
            layer_visible: true,
            backdrop,
            tint,
            dismiss_hides,
            settings_on_launch,
            snap,
            manual_start,
            library: Arc::default(),
            editing: None,
            active: None,
            revealed: None,
            highlighted: None,
            toast: None,
            appearing: Vec::new(),
            leaving: Vec::new(),
            sticky: StickyImport::default(),
            show_debug: false,
            autostarted,
            main_started,
            first_frame: None,
            frames: 0,
        }
    }

    fn area(&self, ui: &Ui) -> Vec2 {
        ui.max_rect().size()
    }

    fn add_from_capture(&mut self, text: &str, area: Vec2) {
        let parsed = parse_capture(text);
        if parsed.body.is_empty() && parsed.title.is_empty() {
            return;
        }
        let pos = card::free_slot(&self.cards, area);
        match self.store.insert(&parsed, pos) {
            Ok(c) => {
                self.appearing.push((c.id, Instant::now()));
                self.cards.push(c);
            }
            Err(e) => eprintln!("insert failed: {e}"),
        }
    }

    fn save(&self, idx: usize) {
        if let Err(e) = self.store.save(&self.cards[idx]) {
            eprintln!("save failed: {e}");
        }
    }

    fn commit_edit(&mut self) {
        let Some((id, buf)) = self.editing.take() else { return };
        let Some(idx) = self.cards.iter().position(|c| c.id == id) else { return };
        let buf = buf.trim();
        let c = &mut self.cards[idx];
        // Saved as written; an old separate title becomes the first line of the text.
        c.title.clear();
        c.body = buf.to_owned();
        c.tags = buf
            .split_whitespace()
            .filter_map(|w| w.strip_prefix('#'))
            .map(|t| t.trim_end_matches([',', '.', ';']).to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        self.save(idx);
    }

    fn cycle_monitor(&mut self) {
        let monitors = win::monitors();
        let current = self.hwnd.and_then(win::monitor_of).map(|m| m.handle);
        let idx = monitors.iter().position(|m| Some(m.handle) == current).map_or(0, |i| (i + 1) % monitors.len());
        if let Some(device) = monitors.get(idx).and_then(|m| m.device_ids.first()).cloned() {
            self.set_monitor(&device);
        }
    }

    // -----------------------------------------------------------------------

    fn header(&self, ui: &mut Ui) {
        let origin = ui.max_rect().min + vec2(32.0, 22.0);
        let painter = ui.painter();
        let title = painter.text(origin, Align2::LEFT_TOP, "Ebb", theme::semibold(22.0), TEXT);
        painter.text(
            pos2(title.right() + 14.0, title.bottom() - 3.0),
            Align2::LEFT_BOTTOM,
            {
                let label = |l: &std::sync::OnceLock<Option<&'static str>>| match l.get() {
                    Some(Some(label)) => *label,
                    Some(None) => "хоткей занят",
                    None => "…",
                };
                format!(
                    "{} карточек  ·  {} — записать  ·  {} — найти  ·  F1 — debug",
                    self.cards.len(),
                    label(&self.shell.hotkey_label),
                    label(&self.shell.search_hotkey_label),
                )
            },
            FontId::proportional(13.0),
            TEXT_MUTED,
        );
    }

    /// "⋯" in the top right corner of the layer: what the tray menu offers, without the tray.
    fn menu_ui(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let rect = Rect::from_min_size(pos2(full.right() - 32.0 - 40.0, full.top() + 16.0), vec2(40.0, 36.0));
        let button = ui.interact(rect, Id::new("layer-menu"), Sense::click());
        button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Меню"));
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&button));
        let fill = if open || button.hovered() { theme::glass_fill_hover() } else { theme::glass_fill() };
        ui.painter().rect(rect, CornerRadius::same(10), fill, Stroke::new(1.0, theme::glass_stroke()), StrokeKind::Inside);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "\u{E712}", theme::icons(16.0), if open { TEXT } else { TEXT_DIM });
        let button = button.on_hover_cursor(CursorIcon::PointingHand);

        let hotkey = |l: &std::sync::OnceLock<Option<&'static str>>| l.get().copied().flatten().unwrap_or("");
        let (capture_key, search_key) = (hotkey(&self.shell.hotkey_label), hotkey(&self.shell.search_hotkey_label));
        let dismiss_label = if self.dismiss_hides { "Скрыть слой" } else { "Убрать на фон" };
        let mut chosen = None;
        egui::Popup::menu(&button)
            .align(egui::RectAlign::BOTTOM_END)
            .gap(6.0)
            .width(250.0)
            .show(|ui| {
                ui.spacing_mut().button_padding = vec2(10.0, 6.0);
                let mut item = |ui: &mut Ui, label: &str, key: &str, event: MenuItem| {
                    let button = egui::Button::new(RichText::new(label).size(14.0)).shortcut_text(RichText::new(key).size(12.5));
                    if ui.add(button.min_size(vec2(ui.available_width(), 0.0))).clicked() {
                        chosen = Some(event);
                    }
                };
                item(ui, "Записать мысль", capture_key, MenuItem::Capture);
                item(ui, "Найти", search_key, MenuItem::Search);
                item(ui, "Архив и корзина", "Win+Alt+L", MenuItem::Library);
                ui.separator();
                item(ui, "Импорт из Sticky Notes…", "", MenuItem::Import);
                item(ui, "Настройки…", "", MenuItem::Settings);
                ui.separator();
                item(ui, dismiss_label, "Esc", MenuItem::Dismiss);
                item(ui, "Выход", "", MenuItem::Exit);
            });

        let Some(chosen) = chosen else { return };
        let event = match chosen {
            MenuItem::Capture => Event::Capture(Instant::now()),
            MenuItem::Search => Event::Search(Instant::now()),
            MenuItem::Library => Event::OpenLibrary(false),
            MenuItem::Import => Event::ImportSticky,
            MenuItem::Settings => Event::OpenLibrary(true),
            MenuItem::Exit => Event::Exit,
            MenuItem::Dismiss => {
                self.dismiss(ui.ctx());
                return;
            }
        };
        // Handled in logic() like the tray's, on the next pass.
        self.shell.events.lock().unwrap().push(event);
        ui.ctx().request_repaint();
    }

    fn cards_ui(&mut self, ui: &mut Ui) {
        let origin = ui.max_rect().min;
        // Not over a card while it's over the menu or another popup above the layer.
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|&p| ui.ctx().layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background));
        let hovered_id = pointer.and_then(|p| {
            self.cards
                .iter()
                .rev()
                .find(|c| Rect::from_min_size(origin + c.pos.to_vec2(), c.size).contains(p))
                .map(|c| c.id)
        });
        let area = self.area(ui);

        if let Some((_, t)) = self.revealed {
            if t.elapsed() >= REVEAL_FOR {
                self.revealed = None;
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
        let rects: Vec<(i64, Rect)> = self.cards.iter().map(|c| (c.id, Rect::from_min_size(c.pos, c.size))).collect();
        let magnet = self.snap.then_some(rects.as_slice());
        let mut actions: Vec<(usize, Action)> = Vec::new();
        for idx in 0..self.cards.len() {
            let id = self.cards[idx].id;
            let editing = self.editing.as_mut().filter(|(eid, _)| *eid == id).map(|(_, b)| b);
            let revealed = self.revealed.is_some_and(|(rid, _)| rid == id);
            let appear = self
                .appearing
                .iter()
                .find(|(aid, _)| *aid == id)
                .map_or(1.0, |(_, at)| ease(at.elapsed().as_secs_f32() / CARD_APPEAR.as_secs_f32()));
            let active = self.active == Some(id) || self.highlighted.is_some_and(|(hid, _)| hid == id);
            let card = &mut self.cards[idx];
            let hovered = hovered_id == Some(id);
            let produced = ui
                .scope(|ui| {
                    ui.multiply_opacity(appear);
                    card_ui(ui, origin + vec2(0.0, (1.0 - appear) * 10.0), area, card, hovered, editing, revealed, active, magnet)
                })
                .inner;
            for a in produced {
                actions.push((idx, a));
            }
        }
        for (card, at) in &mut self.leaving {
            let t = ease(at.elapsed().as_secs_f32() / CARD_LEAVE.as_secs_f32());
            ui.scope(|ui| {
                ui.disable();
                ui.multiply_opacity(1.0 - t);
                let _ = card_ui(ui, origin + vec2(0.0, t * 6.0), area, card, false, None, false, false, None);
            });
        }

        if let Some((hid, t)) = self.highlighted {
            match self.cards.iter().find(|c| c.id == hid) {
                Some(c) if t.elapsed() < HIGHLIGHT_FOR => {
                    let fade = 1.0 - t.elapsed().as_secs_f32() / HIGHLIGHT_FOR.as_secs_f32();
                    let rect = Rect::from_min_size(origin + c.pos.to_vec2(), c.size).expand(3.0);
                    let stroke = Stroke::new(2.0, c.kind.accent().gamma_multiply(fade));
                    ui.painter().rect_stroke(rect, CornerRadius::same(14), stroke, StrokeKind::Outside);
                    ui.ctx().request_repaint();
                }
                _ => self.highlighted = None,
            }
        }

        // Clicking empty space ends editing and deselects.
        if hovered_id.is_none() && ui.input(|i| i.pointer.any_pressed()) {
            self.commit_edit();
            self.active = None;
        }

        let mut to_front = None;
        let mut remove = None;
        for (idx, action) in actions {
            match action {
                Action::Front => {
                    to_front = Some(idx);
                    self.active = Some(self.cards[idx].id);
                }
                Action::Moved => self.save(idx),
                Action::TogglePin => {
                    self.cards[idx].pinned ^= true;
                    self.save(idx);
                }
                Action::Copy => {
                    let c = &self.cards[idx];
                    let text = if c.title.is_empty() { c.body.clone() } else { format!("{}\n{}", c.title, c.body) };
                    ui.ctx().copy_text(text);
                }
                Action::Duplicate => {
                    let src = self.cards[idx].clone();
                    let pos = card::beside(&self.cards, &src, area);
                    let parsed = card::Parsed { kind: src.kind, title: src.title, body: src.body, tags: src.tags };
                    match self.store.insert(&parsed, pos) {
                        Ok(mut copy) => {
                            copy.size = src.size;
                            self.appearing.push((copy.id, Instant::now()));
                            self.cards.push(copy);
                            self.save(self.cards.len() - 1);
                            self.library.lock().unwrap().invalidate();
                            self.bar.lock().unwrap().invalidate();
                        }
                        Err(e) => eprintln!("duplicate failed: {e}"),
                    }
                }
                Action::Archive => {
                    self.cards[idx].archived = true;
                    self.save(idx);
                    self.toast = Some(Toast::new("Карточка в архиве", Some(Undo::Unarchive(self.cards[idx].id))));
                    remove = Some(idx);
                }
                Action::Delete => {
                    let _ = self.store.delete(self.cards[idx].id);
                    let text = format!("Карточка в корзине, {} дней можно вернуть", crate::store::TRASH_DAYS);
                    self.toast = Some(Toast::new(text, Some(Undo::Restore(self.cards[idx].id))));
                    remove = Some(idx);
                }
                Action::StartEdit => {
                    self.commit_edit();
                    let c = &self.cards[idx];
                    let buf = if c.title.is_empty() { c.body.clone() } else { format!("{}\n{}", c.title, c.body) };
                    self.editing = Some((c.id, buf));
                }
                Action::Reveal => {
                    self.revealed = Some((self.cards[idx].id, Instant::now()));
                }
            }
        }
        if let Some(idx) = remove {
            let card = self.cards.remove(idx);
            self.leaving.push((card, Instant::now()));
            self.library.lock().unwrap().invalidate();
            self.bar.lock().unwrap().invalidate();
        } else if let Some(idx) = to_front {
            if idx + 1 != self.cards.len() {
                let c = self.cards.remove(idx);
                self.cards.push(c);
            }
        }
    }

    /// Bottom-center notice with an optional undo, dismissed after TOAST_FOR.
    fn toast_ui(&mut self, ui: &mut Ui) {
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
            let text = ui.painter().layout_no_wrap(toast.text.clone(), font, TEXT);
            let undo_w = if toast.undo.is_some() { 96.0 } else { 0.0 };
            let size = vec2(text.size().x + undo_w + 40.0, 44.0);
            let full = ui.max_rect();
            let center = pos2(full.center().x, full.bottom() - 48.0 - size.y / 2.0 + (1.0 - appear) * 12.0);
            let rect = Rect::from_center_size(center, size);
            glass_panel(ui, rect, false);
            ui.painter().galley(pos2(rect.left() + 20.0, rect.center().y - text.size().y / 2.0), text, TEXT);

            if let Some(action) = toast.undo {
                let button = Rect::from_min_size(pos2(rect.right() - undo_w - 8.0, rect.top() + 8.0), vec2(undo_w, 28.0));
                let resp = ui.interact(button, Id::new("toast-undo"), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Отменить"));
                let fill = if resp.hovered() { Color32::from_rgba_unmultiplied(96, 165, 250, 90) } else { theme::chip_fill() };
                ui.painter().rect_filled(button, CornerRadius::same(8), fill);
                ui.painter().text(button.center(), Align2::CENTER_CENTER, "Отменить", FontId::proportional(13.5), TEXT);
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    undo = Some(action);
                }
            }
        });
        if let Some(action) = undo {
            match action {
                Undo::Unarchive(id) => {
                    let _ = self.store.set_archived(id, false);
                }
                Undo::Restore(id) => {
                    let _ = self.store.restore(id);
                }
            }
            self.toast = None;
            self.reload_cards();
            self.bar.lock().unwrap().invalidate();
            self.library.lock().unwrap().invalidate();
        }
    }

    fn debug_ui(&mut self, ui: &mut Ui) {
        let rect = Rect::from_min_size(ui.max_rect().right_bottom() - vec2(420.0, 300.0), vec2(396.0, 276.0));
        glass_panel(ui, rect, false);
        let mut settings_clicked = false;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(16.0)), |ui| {
            ui.label(RichText::new("Debug").font(theme::semibold(15.0)).color(TEXT));
            ui.add_space(4.0);
            let (proc_ms, main_ms) = self.first_frame.unwrap_or((f64::NAN, f64::NAN));
            let (ws, private) = win::memory_mib();
            let latency = self.bar.lock().unwrap().latency_ms;
            let row = |ui: &mut Ui, k: &str, v: String| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).size(13.0).color(TEXT_MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(v).size(13.0).color(TEXT));
                    });
                });
            };
            row(ui, "Первый кадр от старта процесса", format!("{proc_ms:.0} мс"));
            row(ui, "Первый кадр от main()", format!("{main_ms:.0} мс"));
            row(
                ui,
                "Хоткей → кадр окна захвата",
                latency.map_or("—".into(), |l| format!("{l:.0} мс")),
            );
            row(ui, "Память: working set / private", format!("{ws:.1} / {private:.1} MiB"));
            row(ui, "Кадров отрисовано", self.frames.to_string());
            row(ui, "Фон (F2) / монитор (F3)", self.backdrop.key().to_owned());
            ui.add_space(4.0);
            settings_clicked = ui.button("Настройки…").clicked();
        });
        if settings_clicked {
            self.library.lock().unwrap().open(Tab::Settings);
        }
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }

    fn layer_settings(&self) -> library::LayerSettings {
        library::LayerSettings {
            monitor_device: self.hwnd.and_then(win::monitor_of).and_then(|m| m.device_ids.first().cloned()),
            backdrop: self.backdrop,
            tint: self.tint,
            pin_bottom: win::PIN_BOTTOM.load(Ordering::Relaxed),
            capture_hotkey: self.shell.hotkey_label.get().copied().flatten(),
            search_hotkey: self.shell.search_hotkey_label.get().copied().flatten(),
            dismiss_hides: self.dismiss_hides,
            settings_on_launch: self.settings_on_launch,
            snap: self.snap,
        }
    }

    /// Opens the settings window; `welcome` on the first run.
    fn open_settings(&mut self, ctx: &egui::Context, welcome: bool) {
        let mut lib = self.library.lock().unwrap();
        lib.settings = self.layer_settings();
        lib.open(Tab::Settings);
        lib.welcome |= welcome;
        ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::InnerSize(lib.size()));
        ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::Focus);
        ctx.request_repaint();
    }

    /// Shows the layer over the other windows.
    fn summon(&mut self, ctx: &egui::Context) {
        let already_up = self.layer_visible && win::RAISED.load(Ordering::Relaxed);
        let was_visible = self.layer_visible;
        // Fades in when it was hidden.
        self.set_layer_visible(ctx, true);
        let Some(h) = self.hwnd else { return };
        if already_up {
            win::raise(h);
            return;
        }
        if was_visible {
            // Was under the windows: come up the same way it appears when shown.
            // Alpha goes to 0 before the raise, so it never pops in at full opacity.
            win::fade(h, 0, 255, LAYER_FADE_IN, || {});
        }
        win::raise(h);
        let now = Instant::now();
        self.appearing = self.cards.iter().map(|c| (c.id, now)).collect();
        ctx.request_repaint();
    }

    /// Esc / second tray click: back under the windows, or hidden (a setting).
    fn dismiss(&mut self, ctx: &egui::Context) {
        if self.dismiss_hides {
            win::RAISED.store(false, Ordering::Relaxed);
            self.set_layer_visible(ctx, false);
        } else if let Some(h) = self.hwnd {
            win::lower(h);
        }
    }

    fn tray_click(&mut self, ctx: &egui::Context) {
        let summoned = win::RAISED.load(Ordering::Relaxed) || win::ms_since_auto_lowered() < TRAY_CLICK_AFTER_LOWER_MS;
        if self.layer_visible && summoned {
            self.dismiss(ctx);
        } else {
            self.summon(ctx);
        }
    }

    fn set_flag(&self, key: &str, on: bool) {
        let _ = self.store.set_setting(key, if on { "1" } else { "0" });
    }

    fn set_monitor(&mut self, device: &str) {
        let monitors = win::monitors();
        if let (Some(m), Some(h)) = (monitors.iter().find(|m| m.matches_device(device)), self.hwnd) {
            win::place_on(h, m);
            let _ = self.store.set_setting(SET_MONITOR, device);
        }
    }

    fn set_backdrop(&mut self, backdrop: Backdrop) {
        self.backdrop = backdrop;
        if let Some(h) = self.hwnd {
            win::apply_backdrop(h, backdrop, false);
        }
        let _ = self.store.set_setting(SET_BACKDROP, backdrop.key());
    }

    fn set_pin_bottom(&mut self, on: bool) {
        if let Some(h) = self.hwnd {
            win::set_pin_bottom(h, on);
        }
        self.shell.pin_bottom.store(on, Ordering::Relaxed);
        let _ = self.store.set_setting(SET_PIN_BOTTOM, if on { "1" } else { "0" });
    }

    fn apply_library_request(&mut self, ctx: &egui::Context, request: Request) {
        match request {
            Request::Open(id) => self.open_card(ctx, id),
            Request::Changed => {
                self.reload_cards();
                self.bar.lock().unwrap().invalidate();
            }
            Request::SetMonitor(device) => self.set_monitor(&device),
            Request::SetBackdrop(b) => self.set_backdrop(b),
            Request::SetTint(t) => {
                self.tint = t;
                let _ = self.store.set_setting(SET_TINT, &t.to_string());
                ctx.request_repaint_of(ViewportId::ROOT);
            }
            Request::SetPinBottom(on) => self.set_pin_bottom(on),
            Request::SetDismissHides(on) => {
                self.dismiss_hides = on;
                let _ = self.store.set_setting(SET_DISMISS, if on { "hide" } else { "back" });
            }
            Request::SetSettingsOnLaunch(on) => {
                self.settings_on_launch = on;
                self.set_flag(SET_SETTINGS_ON_LAUNCH, on);
            }
            Request::SetSnap(on) => {
                self.snap = on;
                self.set_flag(SET_SNAP, on);
            }
            Request::ImportSticky => {
                self.set_layer_visible(ctx, true);
                self.sticky.scan(ctx, &self.store, self.hwnd, true);
            }
        }
    }

    fn library_viewport(&self, ui: &Ui) {
        let (open, placed, size) = {
            let lib = self.library.lock().unwrap();
            (lib.open, lib.placed, lib.size())
        };
        if !open {
            // Not shown this frame: egui drops the viewport and eframe destroys the window.
            return;
        }
        let state = self.library.clone();
        ui.ctx().show_viewport_deferred(
            library::viewport_id(),
            ViewportBuilder::default()
                .with_title("Ebb Library")
                .with_inner_size(size)
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_minimize_button(false)
                .with_maximize_button(false)
                .with_taskbar(true)
                // Hidden until moved onto the cursor's monitor (see logic()).
                .with_visible(placed),
            move |ui, _class| library::ui(ui, &state),
        );
    }

    fn shutdown(&mut self) {
        self.commit_edit();
        shell::remove_tray_icon(&self.shell);
    }

    fn set_layer_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if visible == self.layer_visible {
            return;
        }
        self.layer_visible = visible;
        self.shell.layer_visible.store(visible, Ordering::Relaxed);
        if visible {
            if let Some(h) = self.hwnd {
                win::fade(h, 0, 255, LAYER_FADE_IN, || {});
            }
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(true));
            return;
        }
        self.commit_edit();
        let Some(h) = self.hwnd else {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(false));
            return;
        };
        // Fade out, then hide through eframe (logic() picks up the flag).
        let (done, ctx) = (self.layer_faded_out.clone(), ctx.clone());
        win::fade(h, 255, 0, LAYER_FADE_OUT, move || {
            done.store(true, Ordering::Relaxed);
            ctx.request_repaint();
            // Nothing is drawn while hidden; give the pages back until it's shown again.
            std::thread::sleep(Duration::from_millis(400));
            win::trim_working_set();
        });
    }

    fn cycle_backdrop(&mut self) {
        self.set_backdrop(self.backdrop.next());
    }

    fn bar_viewport(&self, ui: &Ui) {
        let (visible, size) = {
            let bar = self.bar.lock().unwrap();
            // `shown`, not `visible`: the window stays up while it fades out.
            (bar.shown, bar.size())
        };
        let state = self.bar.clone();
        ui.ctx().show_viewport_deferred(
            bar::viewport_id(),
            ViewportBuilder::default()
                .with_title("Ebb Capture")
                .with_inner_size(size)
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_close_button(false)
                .with_minimize_button(false)
                .with_maximize_button(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_visible(visible),
            move |ui, _class| bar::ui(ui, &state),
        );
    }

    /// Shows a card on the layer: brings it to front, or restores it from the archive
    /// into a free slot. Counts as viewed.
    fn open_card(&mut self, ctx: &egui::Context, id: i64) {
        self.set_layer_visible(ctx, true);
        let idx = match self.cards.iter().position(|c| c.id == id) {
            Some(idx) => idx,
            None => {
                let Ok(Some(mut card)) = self.store.card(id) else { return };
                card.archived = false;
                card.pos = card::free_slot(&self.cards, ctx.content_rect().size());
                card.size = card.size.max(MIN_SIZE);
                self.appearing.push((card.id, Instant::now()));
                self.cards.push(card);
                let idx = self.cards.len() - 1;
                self.save(idx);
                idx
            }
        };
        let card = self.cards.remove(idx);
        self.cards.push(card);
        let _ = self.store.touch(id);
        self.highlighted = Some((id, Instant::now()));
        ctx.request_repaint();
    }

    fn reload_cards(&mut self) {
        self.commit_edit();
        if let Ok(cards) = self.store.load() {
            let now = Instant::now();
            // Cards that weren't on the layer before appear; ones that left fade out.
            for c in cards.iter().filter(|c| !self.cards.iter().any(|old| old.id == c.id)) {
                self.appearing.push((c.id, now));
            }
            let gone = self.cards.drain(..).filter(|old| !cards.iter().any(|c| c.id == old.id));
            self.leaving.extend(gone.map(|c| (c, now)));
            self.cards = cards;
        }
    }
}

impl eframe::App for EbbApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Runs even while the layer is hidden, so tray and hotkey events work then too.
        let mut pressed = None;
        for event in self.shell.take_events() {
            match event {
                Event::Capture(t) => pressed = Some((Mode::Capture, t)),
                Event::Search(t) => pressed = Some((Mode::Search, t)),
                Event::ToggleLayer if self.layer_visible => self.set_layer_visible(ctx, false),
                Event::ToggleLayer => self.summon(ctx),
                Event::TrayClick => self.tray_click(ctx),
                Event::Launched => {
                    self.summon(ctx);
                    if self.settings_on_launch {
                        self.open_settings(ctx, false);
                    }
                }
                Event::TogglePinBottom => self.set_pin_bottom(!win::PIN_BOTTOM.load(Ordering::Relaxed)),
                Event::Exit => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
                Event::ImportSticky => self.apply_library_request(ctx, Request::ImportSticky),
                Event::OpenLibrary(true) => self.open_settings(ctx, false),
                Event::OpenLibrary(false) => {
                    let mut lib = self.library.lock().unwrap();
                    lib.settings = self.layer_settings();
                    lib.open(Tab::Archive);
                    ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::InnerSize(lib.size()));
                    ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::Focus);
                }
            }
        }

        if self.layer_faded_out.swap(false, Ordering::Relaxed) && !self.layer_visible {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(false));
        }

        // Library window: place it once created, apply its requests.
        let requests = {
            let mut lib = self.library.lock().unwrap();
            if lib.open && lib.hwnd.is_none() {
                lib.hwnd = win::find_library_window();
                if let Some(h) = lib.hwnd {
                    let (scale, size) = (win::dpi_scale(h), lib.size());
                    win::center_near_cursor(h, (size.x * scale) as i32, (size.y * scale) as i32);
                    win::apply_backdrop(h, Backdrop::AccentAcrylic, true);
                    win::install_window_rules(h, 0);
                    lib.placed = true;
                    // Above a layer that was just summoned in the same moment.
                    ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::Focus);
                    ctx.request_repaint();
                }
            }
            if !lib.open && lib.hwnd.take().is_some() {
                // Just closed: the window and its surface are gone; drop their pages.
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(800));
                    win::trim_working_set();
                });
            }
            std::mem::take(&mut lib.outbox)
        };
        for request in requests {
            self.apply_library_request(ctx, request);
        }

        let mut bar = self.bar.lock().unwrap();
        if bar.hwnd.is_none() {
            bar.hwnd = win::find_capture_window();
            if let Some(h) = bar.hwnd {
                win::apply_backdrop(h, Backdrop::AccentAcrylic, true);
                win::install_window_rules(h, win::FADE);
            }
        }
        let id = bar::viewport_id();
        let mut hide = std::mem::take(&mut bar.hide_requested);
        if let Some((mode, t)) = pressed {
            match bar.press(mode, t) {
                Press::Hide => hide = true,
                press => {
                    let size = bar.size();
                    if let (Press::Show, Some(h)) = (&press, bar.hwnd) {
                        win::move_near_cursor(h, (size.x * win::dpi_scale(h)) as i32);
                        // Transparent until the bar's first frame starts the fade-in.
                        win::set_alpha_now(h, 0);
                    }
                    // Explicit commands: with the layer hidden the root ui (which carries
                    // the viewport builder) doesn't run until something is visible.
                    ctx.send_viewport_cmd_to(id, ViewportCommand::InnerSize(size));
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
                }
            }
        }
        if hide {
            // Fade out, then hide through eframe so winit's visibility stays in sync.
            match bar.hwnd {
                Some(h) => {
                    let (done, ctx) = (self.bar_faded_out.clone(), ctx.clone());
                    win::fade(h, 255, 0, Duration::from_secs_f32(bar::DISAPPEAR_SECS), move || {
                        done.store(true, Ordering::Relaxed);
                        ctx.request_repaint();
                    });
                }
                None => {
                    bar.shown = false;
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
                }
            }
        }
        if self.bar_faded_out.swap(false, Ordering::Relaxed) && !bar.visible {
            bar.shown = false;
            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
        }
        let outbox = std::mem::take(&mut bar.outbox);
        drop(bar);

        let mut changed = false;
        for request in outbox {
            match request {
                Outbox::Captured(text) => {
                    self.add_from_capture(&text, ctx.content_rect().size());
                    changed = true;
                }
                Outbox::Open(id) => {
                    // From search the layer is usually under windows: bring it up.
                    self.summon(ctx);
                    self.open_card(ctx, id);
                    changed = true;
                }
                Outbox::Changed => {
                    self.reload_cards();
                    changed = true;
                }
            }
        }
        if changed {
            self.bar.lock().unwrap().invalidate();
            self.library.lock().unwrap().invalidate();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if self.first_frame.is_none() {
            let frame_at = win::now_filetime();
            let proc_ms = win::ms_since_process_start();
            let main_ms = self.main_started.elapsed().as_secs_f64() * 1000.0;
            self.first_frame = Some((proc_ms, main_ms));
            if let Some(h) = self.hwnd {
                // eframe shows the window right after this frame; it's still at alpha 0.
                win::fade(h, 0, 255, LAYER_FADE_IN, || {});
            }
            let autostarted = self.autostarted;
            std::thread::spawn(move || {
                log_first_frame(frame_at, proc_ms, main_ms, autostarted);
                // Startup touches lots of one-off pages (loader, driver init, font parsing).
                // Drop them from the working set once the first frames are out; private
                // bytes are unchanged and hotkey latency rises by ~3 ms.
                std::thread::sleep(Duration::from_secs(2));
                win::trim_working_set();
            });
            // Out of the way of logon and the first frames; not during benchmarks.
            if crate::bench::path().is_none() {
                self.sticky.maybe_offer(ui.ctx(), &self.store, self.hwnd);
                let first_run = self.store.setting(SET_ONBOARDED).is_none();
                if first_run || (self.manual_start && self.settings_on_launch) {
                    let _ = self.store.set_setting(SET_ONBOARDED, "1");
                    self.open_settings(ui.ctx(), first_run);
                }
            }
            if self.manual_start {
                // eframe shows the window after this frame; take the focus on the next.
                ui.ctx().request_repaint();
            }
            if let Some(out) = crate::bench::path() {
                let (shell, ctx) = (self.shell.clone(), ui.ctx().clone());
                let bar = self.bar.clone();
                let hooks = crate::bench::Hooks {
                    trigger: Some(Box::new(move |search| {
                        let t = Instant::now();
                        let event = if search { Event::Search(t) } else { Event::Capture(t) };
                        shell.events.lock().unwrap().push(event);
                        ctx.request_repaint();
                    })),
                    latency_ms: Box::new(move || bar.lock().unwrap().latency_ms),
                };
                let label = format!("ebb-{}", crate::renderer::NAME);
                crate::bench::start(ui.ctx().clone(), out, label, proc_ms, main_ms, hooks);
            }
        } else if self.frames == 2 && self.manual_start && win::RAISED.load(Ordering::Relaxed) {
            if let Some(h) = self.hwnd {
                win::raise(h);
            }
            if self.library.lock().unwrap().placed {
                ui.ctx().send_viewport_cmd_to(library::viewport_id(), ViewportCommand::Focus);
            }
        }

        let (f1, f2, f3, esc) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::F1),
                i.consume_key(Modifiers::NONE, Key::F2),
                i.consume_key(Modifiers::NONE, Key::F3),
                i.key_pressed(Key::Escape),
            )
        });
        if f1 {
            self.show_debug ^= true;
        }
        if f2 {
            self.cycle_backdrop();
        }
        if f3 {
            self.cycle_monitor();
        }
        if esc {
            if self.editing.is_some() {
                self.commit_edit();
            } else if egui::Popup::is_any_open(ui.ctx()) {
                // The menu closes itself on Esc; the layer stays.
            } else {
                self.dismiss(ui.ctx());
            }
        }

        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(self.tint));

        self.header(ui);
        self.cards_ui(ui);
        self.sticky.ui(ui, &mut self.store, &mut self.cards, self.hwnd);
        self.toast_ui(ui);
        self.menu_ui(ui);
        if self.show_debug {
            self.debug_ui(ui);
        }
        // Created up front even while hidden: creating it on the first hotkey press
        // saves ~4 MiB but doubles the first-show latency and flashes without acrylic.
        self.bar_viewport(ui);
        self.library_viewport(ui);
    }

    #[cfg(feature = "glow")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }

    #[cfg(not(feature = "glow"))]
    fn on_exit(&mut self) {
        self.shutdown();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

/// Appends one line to %LOCALAPPDATA%\Ebb\timing.log. Runs off the UI thread:
/// the logon-session and process-snapshot queries take a few milliseconds.
fn log_first_frame(frame_at: u64, proc_ms: f64, main_ms: f64, autostarted: bool) {
    let since = |t: Option<u64>| t.map_or("?".to_owned(), |t| format!("{:.0}", win::ms_between(t, frame_at)));
    let logon = win::logon_filetime();
    let rel = |t: Option<u64>| match (logon, t) {
        (Some(l), Some(t)) => format!("{:.0}", win::ms_between(l, t)),
        _ => "?".to_owned(),
    };
    let line = format!(
        "first frame: {proc_ms:.0} ms since process start, {main_ms:.0} ms since main(); \
         autostart={autostarted}; logon->frame {} ms, logon->process {} ms, logon->explorer {} ms\n",
        since(logon),
        rel(win::process_start_filetime()),
        rel(win::explorer_start_filetime()),
    );
    append_timing_log(&line);
}

pub(crate) fn append_timing_log(line: &str) {
    if let Some(dir) = crate::store::db_path().parent() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("timing.log")) {
            let _ = f.write_all(line.as_bytes());
        }
    }
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

pub(crate) fn glass_panel(ui: &Ui, rect: Rect, hovered: bool) {
    let radius = CornerRadius::same(12);
    let painter = ui.painter();
    painter.add(
        egui::epaint::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(70),
        }
        .as_shape(rect, radius),
    );
    let fill = if hovered { theme::glass_fill_hover() } else { theme::glass_fill() };
    painter.rect(rect, radius, fill, Stroke::new(1.0, theme::glass_stroke()), StrokeKind::Inside);
}

fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), Color32::from_white_alpha(22));
    }
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, glyph, theme::icons(12.5), color);
    // Name for screen readers and UI automation (the glyph itself says nothing).
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
}

/// Tags are shown as chips, so hide the `#tag` tokens from the text itself.
fn without_tags(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.split(' ')
                .filter(|w| !(w.starts_with('#') && w.len() > 1))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn age_label(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mins = (now - created_at).max(0) / 60;
    match mins {
        0 => "сейчас".into(),
        m if m < 60 => format!("{m} мин"),
        m if m < 60 * 24 => format!("{} ч", m / 60),
        m => format!("{} дн", m / (60 * 24)),
    }
}

/// Text of a card, or its editor.
fn card_body(ui: &mut Ui, card: &Card, editing: Option<&mut String>, revealed: bool) {
    if let Some(buf) = editing {
        let resp = ui.add(
            egui::TextEdit::multiline(buf)
                .font(FontId::proportional(14.0))
                .text_color(TEXT)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        return;
    }
    let hidden = card.kind == Kind::Private && !revealed;
    // A hidden Private card still says what it is (see card::private_label).
    let heading = if hidden { card::private_label(&card.title, &card.body) } else { None };
    let heading = heading.as_deref().or((!card.title.is_empty()).then_some(card.title.as_str()));
    if let Some(heading) = heading {
        ui.add(
            egui::Label::new(RichText::new(heading).font(theme::semibold(15.0)).color(TEXT))
                .wrap()
                .selectable(false),
        );
    }
    let text = if hidden { "••••••••••".to_owned() } else { without_tags(&card.body) };
    let color = if heading.is_none() { TEXT } else { TEXT_DIM };
    let size = if heading.is_none() { 14.5 } else { 13.5 };
    ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap().selectable(false));
}

/// `magnet`: every card's rect on the layer, to stick to while dragging.
#[allow(clippy::too_many_arguments)]
fn card_ui(
    ui: &mut Ui,
    origin: Pos2,
    area: Vec2,
    card: &mut Card,
    hovered: bool,
    editing: Option<&mut String>,
    revealed: bool,
    active: bool,
    magnet: Option<&[(i64, Rect)]>,
) -> Vec<Action> {
    let mut out = Vec::new();
    let id = Id::new(("card", card.id));
    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);

    // Registration order = hit-test priority: later widgets sit on top.
    let bg = ui.interact(rect, id.with("bg"), Sense::click());
    let header_rect = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H + 6.0));
    // A pinned card is locked in place: no moving, no resizing.
    let locked = card.pinned;
    let drag = ui.interact(header_rect, id.with("drag"), if locked { Sense::hover() } else { Sense::drag() });
    // Resize handles on every edge and corner; corners last so they win where they overlap.
    const EDGE: f32 = 6.0;
    const CORNER: f32 = 16.0;
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let side = |left, right, top, bottom| Sides { left, right, top, bottom };
    let handles = [
        (side(true, false, false, false), Rect::from_min_max(pos2(l, t), pos2(l + EDGE, b))),
        (side(false, true, false, false), Rect::from_min_max(pos2(r - EDGE, t), pos2(r, b))),
        (side(false, false, true, false), Rect::from_min_max(pos2(l, t), pos2(r, t + EDGE))),
        (side(false, false, false, true), Rect::from_min_max(pos2(l, b - EDGE), pos2(r, b))),
        (side(true, false, true, false), Rect::from_min_max(pos2(l, t), pos2(l + CORNER, t + CORNER))),
        (side(false, true, true, false), Rect::from_min_max(pos2(r - CORNER, t), pos2(r, t + CORNER))),
        (side(true, false, false, true), Rect::from_min_max(pos2(l, b - CORNER), pos2(l + CORNER, b))),
        (side(false, true, false, true), Rect::from_min_max(pos2(r - CORNER, b - CORNER), pos2(r, b))),
    ];
    let handles: Vec<(Sides, egui::Response)> = handles
        .into_iter()
        .filter(|_| !locked)
        .enumerate()
        .map(|(i, (sides, area))| (sides, ui.interact(area, id.with(("resize", i)), Sense::drag())))
        .collect();
    // The last registered handle under the pointer is the one egui hit.
    let resize = handles.iter().rev().find(|(_, h)| h.dragged() || h.drag_started() || h.drag_stopped());
    let resize_hover = handles.iter().rev().find(|(_, h)| h.hovered()).map(|(s, _)| *s);
    let corner_hovered = resize_hover.is_some_and(|s| s.right && s.bottom);

    if bg.clicked() || bg.double_clicked() || drag.drag_started() || resize.is_some_and(|(_, h)| h.drag_started()) {
        out.push(Action::Front);
    }
    if bg.double_clicked() && editing.is_none() && card.kind != Kind::Private {
        out.push(Action::StartEdit);
    }

    // The rect at the start of the gesture and the pointer's total movement live in
    // memory, so a stuck edge follows the pointer again once it moves past the snap
    // distance, and shrinking below the minimum doesn't lose track of the pointer.
    let raw_id = id.with("raw");
    let started = drag.drag_started() || resize.is_some_and(|(_, h)| h.drag_started());
    if started {
        ui.data_mut(|d| d.insert_temp(raw_id, (Rect::from_min_size(card.pos, card.size), Vec2::ZERO)));
    }
    // Alt places the card freely.
    let snapping = magnet.is_some() && !ui.input(|i| i.modifiers.alt);
    let bounds = Rect::from_min_size(Pos2::ZERO, area);
    let others: Vec<Rect> = match magnet {
        Some(all) if snapping && (drag.dragged() || drag.drag_stopped() || resize.is_some()) => {
            all.iter().filter(|(cid, _)| *cid != card.id).map(|(_, r)| *r).collect()
        }
        _ => Vec::new(),
    };
    let mut snapped: Option<card::Snapped> = None;
    let resizing = resize.filter(|(_, h)| h.dragged()).map(|(s, h)| (*s, h.drag_delta()));
    if drag.dragged() || resizing.is_some() {
        let (start, mut total) =
            ui.data(|d| d.get_temp::<(Rect, Vec2)>(raw_id)).unwrap_or((Rect::from_min_size(card.pos, card.size), Vec2::ZERO));
        total += resizing.map_or(drag.drag_delta(), |(_, d)| d);
        ui.data_mut(|d| d.insert_temp(raw_id, (start, total)));
        let raw = match resizing {
            None => {
                let moved = start.translate(total);
                let min = pos2(moved.min.x.clamp(0.0, (area.x - 80.0).max(0.0)), moved.min.y.clamp(0.0, (area.y - HEADER_H).max(0.0)));
                Rect::from_min_size(min, moved.size())
            }
            Some((sides, _)) => sides.resize(start, total, bounds),
        };
        let s = match (snapping, resizing) {
            (false, _) => card::Snapped { rect: raw, guides: Vec::new(), x: false, y: false },
            (true, None) => card::snap_move(raw, &others, bounds),
            (true, Some((sides, _))) => card::snap_resize(raw, &others, bounds, sides),
        };
        card.pos = s.rect.min;
        card.size = s.rect.size();
        snapped = Some(s);
    }
    let stopped_resize = resize.filter(|(_, h)| h.drag_stopped()).map(|(s, _)| *s);
    if drag.drag_stopped() || stopped_resize.is_some() {
        // Edges that stuck stay put; the rest go to the 8 pt grid.
        let rect = Rect::from_min_size(card.pos, card.size);
        let s = match (snapping, stopped_resize) {
            (false, _) => None,
            (true, None) => Some(card::snap_move(rect, &others, bounds)),
            (true, Some(sides)) => Some(card::snap_resize(rect, &others, bounds, sides)),
        };
        let (sx, sy) = s.as_ref().map_or((false, false), |s| (s.x, s.y));
        let grid = |v: f32, stuck: bool| if stuck { v } else { (v / 8.0).round() * 8.0 };
        let rect = match stopped_resize {
            None => Rect::from_min_size(pos2(grid(rect.min.x, sx), grid(rect.min.y, sy)), rect.size()),
            Some(sides) => {
                let mut g = rect;
                if sides.left { g.min.x = grid(g.min.x, sx).min(g.max.x - MIN_SIZE.x) }
                if sides.right { g.max.x = grid(g.max.x, sx).max(g.min.x + MIN_SIZE.x) }
                if sides.top { g.min.y = grid(g.min.y, sy).min(g.max.y - MIN_SIZE.y) }
                if sides.bottom { g.max.y = grid(g.max.y, sy).max(g.min.y + MIN_SIZE.y) }
                g
            }
        };
        card.pos = rect.min;
        card.size = rect.size();
        ui.data_mut(|d| d.remove::<(Rect, Vec2)>(raw_id));
        out.push(Action::Moved);
    }
    if drag.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if drag.hovered() && resize_hover.is_none() && !locked {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if let Some(sides) = resize.map(|(s, _)| *s).or(resize_hover) {
        ui.ctx().set_cursor_icon(sides.cursor());
    }

    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);
    glass_panel(ui, rect, hovered);
    if let Some(s) = &snapped {
        // Above every card, not just the ones painted before this one.
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, id.with("guides")));
        let stroke = Stroke::new(1.0, card.kind.accent().gamma_multiply(0.8));
        for [a, b] in &s.guides {
            painter.line_segment([origin + a.to_vec2(), origin + b.to_vec2()], stroke);
        }
    }

    let inner = rect.shrink2(vec2(14.0, 8.0));
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), HEADER_H - 4.0));
    let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - FOOTER_H), inner.max);
    let body = Rect::from_min_max(pos2(inner.left(), header.bottom() + 2.0), pos2(inner.right(), footer.top() - 2.0));
    let accent = card.kind.accent();

    // Kind, tags and age only while the card is hovered, edited or selected: at rest
    // a card is just its text.
    let details = ui.ctx().animate_bool_with_time(id.with("details"), hovered || active || editing.is_some(), 0.15);

    // Header: kind + hover actions.
    ui.scope_builder(
        UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.scope(|ui| {
                ui.multiply_opacity(details);
                ui.label(RichText::new(card.kind.icon()).font(theme::icons(12.0)).color(accent));
                ui.add(egui::Label::new(RichText::new(card.kind.label()).size(12.0).color(TEXT_MUTED)).selectable(false));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if hovered {
                    // Pinned: nothing that takes the card off the layer by accident.
                    if !locked && icon_button(ui, "\u{E74D}", "Удалить", TEXT_MUTED).clicked() {
                        out.push(Action::Delete);
                    }
                    if !locked && icon_button(ui, "\u{E7B8}", "В архив", TEXT_DIM).clicked() {
                        out.push(Action::Archive);
                    }
                    if icon_button(ui, "\u{E8C8}", "Дублировать", TEXT_DIM).clicked() {
                        out.push(Action::Duplicate);
                    }
                    // The check mark says the text is on the clipboard.
                    let copied_id = id.with("copied");
                    let copied = ui.data(|d| d.get_temp::<Instant>(copied_id)).filter(|t| t.elapsed() < COPIED_FOR);
                    if let Some(t) = copied {
                        ui.ctx().request_repaint_after(COPIED_FOR.saturating_sub(t.elapsed()));
                    }
                    let (glyph, tip, color) = match copied {
                        Some(_) => ("\u{E73E}", "Текст скопирован", theme::SUCCESS),
                        None => ("\u{E77F}", "Копировать текст", TEXT_DIM),
                    };
                    if icon_button(ui, glyph, tip, color).clicked() {
                        ui.data_mut(|d| d.insert_temp(copied_id, Instant::now()));
                        out.push(Action::Copy);
                    }
                    if card.kind == Kind::Private
                        && icon_button(ui, "\u{E890}", "Показать на 5 секунд", TEXT_DIM).clicked()
                    {
                        out.push(Action::Reveal);
                    }
                }
                if card.pinned || hovered {
                    let (glyph, color) = if card.pinned { ("\u{E841}", accent) } else { ("\u{E718}", TEXT_DIM) };
                    if icon_button(ui, glyph, if card.pinned { "Открепить" } else { "Закрепить на месте" }, color).clicked() {
                        out.push(Action::TogglePin);
                    }
                }
            });
        },
    );

    // Body: scrolls when the text doesn't fit; the floating bar shows only on hover.
    ui.scope_builder(
        UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .auto_shrink([false, false])
                .max_height(body.height())
                .show(ui, |ui| card_body(ui, card, editing, revealed));
        },
    );

    // Footer: tags + age.
    let mut painter = ui.painter().with_clip_rect(footer);
    painter.multiply_opacity(details);
    let age = painter.text(
        pos2(footer.right(), footer.center().y),
        Align2::RIGHT_CENTER,
        age_label(card.created_at),
        FontId::proportional(11.5),
        TEXT_MUTED,
    );
    let mut x = footer.left();
    for tag in &card.tags {
        let galley = painter.layout_no_wrap(format!("#{tag}"), FontId::proportional(11.5), TEXT_DIM);
        let chip = Rect::from_min_size(
            pos2(x, footer.center().y - galley.size().y / 2.0 - 2.0),
            galley.size() + vec2(12.0, 4.0),
        );
        if chip.right() > age.left() - 8.0 {
            break;
        }
        painter.rect_filled(chip, CornerRadius::same(6), theme::chip_fill());
        painter.galley(chip.min + vec2(6.0, 2.0), galley, TEXT_DIM);
        x = chip.right() + 4.0;
    }

    if hovered {
        let p = ui.painter();
        let c = rect.max - vec2(6.0, 6.0);
        let stroke = Stroke::new(1.2, Color32::from_white_alpha(if corner_hovered { 120 } else { 50 }));
        p.line_segment([c - vec2(8.0, 0.0), c - vec2(0.0, 8.0)], stroke);
        p.line_segment([c - vec2(4.0, 0.0), c - vec2(0.0, 4.0)], stroke);
    }

    out
}

