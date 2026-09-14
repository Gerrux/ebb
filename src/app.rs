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
use crate::rich_text;
use crate::shell::{self, Event};
use crate::card::{self, Card, Kind, MIN_SIZE, Placement, Sides, parse_capture};
use crate::store::Store;
use crate::theme;
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
/// The curtain: the layer slides down and fades before it shrinks to the tab.
const CURTAIN_ANIM: Duration = Duration::from_millis(200);
/// How far the layer slides down as it collapses, and the drag that fully collapses it.
const CURTAIN_DROP: f32 = 48.0;
const CURTAIN_DRAG: f32 = 160.0;
/// The collapsed layer: a tab at the top center of the desktop (a notch against
/// the screen's edge) or at the bottom center, above the taskbar.
const TAB_SIZE: (f32, f32) = (148.0, 40.0);
const TAB_GAP: f32 = 10.0;

fn place_tab(h: isize, m: &win::Monitor, top: bool) {
    win::place_edge_center(h, m, TAB_SIZE, if top { 0.0 } else { TAB_GAP }, top);
}

// Settings keys.
const SET_MONITOR: &str = "layer.monitor";
const SET_BACKDROP: &str = "layer.backdrop";
const SET_TINT: &str = "layer.tint";
const SET_PIN_BOTTOM: &str = "layer.pin_bottom";
/// "hide" or "back": what Esc / a second tray click does to a summoned layer.
const SET_DISMISS: &str = "layer.dismiss";
const SET_COLLAPSED: &str = "layer.collapsed";
/// "top" or "bottom": the edge the layer rolls up to.
const SET_CURTAIN: &str = "layer.curtain";
const SET_SETTINGS_ON_LAUNCH: &str = "app.settings_on_launch";
const SET_ONBOARDED: &str = "app.onboarded";
const SET_SNAP: &str = "cards.snap";
const SET_CARD_STYLE: &str = "cards.style";
const SET_THEME: &str = "app.theme";
/// A tray click this soon after another app took the focus from a summoned layer
/// (the taskbar does, on mouse down) counts as a click on the summoned layer.
const TRAY_CLICK_AFTER_LOWER_MS: u64 = 500;

#[derive(Clone, Copy)]
enum Undo {
    Unarchive(i64),
    Restore(i64),
    /// Back to the kind a card had.
    Kind(i64, Kind),
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
    Review,
    Import,
    Settings,
    Collapse,
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
    SetKind(Kind),
    SetTint(Option<card::Tint>),
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
    /// Rolled down into a tab at the bottom of the desktop, so what's under the
    /// layer (desktop icons) can be reached.
    collapsed: bool,
    /// The layer rolls up to a notch at the top edge; else down to a tab at the bottom.
    curtain_top: bool,
    /// 0 = the layer is up, 1 = slid away; follows a drag on the curtain handle.
    curtain: f32,
    /// Start, from and to of the curtain's animation.
    curtain_anim: Option<(Instant, f32, f32)>,
    /// The window was just resized (to the tab or back): fade it in once a frame
    /// at the new size is drawn.
    resize_pending: bool,
    /// Size of the full layer in points, for placing cards while it's collapsed.
    full_area: Vec2,
    settings_on_launch: bool,
    snap: bool,
    card_style: card::CardStyle,
    theme_mode: theme::ThemeMode,
    /// Started by the user (not at logon): bring the layer up, maybe open settings.
    manual_start: bool,
    library: Arc<Mutex<LibraryState>>,

    editing: Option<(i64, String)>,
    /// Last card clicked or dragged: shows its details until something else is clicked.
    active: Option<i64>,
    revealed: Option<(i64, Instant)>,
    /// Plaintext is kept only for the short reveal window; it is never part of Card.
    revealed_text: Option<(i64, String)>,
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
        mut cards: Vec<Card>,
        fresh: Vec<i64>,
        main_started: Instant,
        autostarted: bool,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        let card_style = store.setting(SET_CARD_STYLE).map_or_else(card::CardStyle::default, |v| card::CardStyle::from_setting(&v));
        theme::set_card_font(&cc.egui_ctx, card_style.font);
        // Before any window gets its backdrop: that follows light/dark too.
        let theme_mode = store.setting(SET_THEME).map_or(theme::ThemeMode::System, |v| theme::ThemeMode::from_key(&v));
        theme::apply(&cc.egui_ctx, theme_mode, card_style.background, &win::system_colors());

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
        // Launched from a shortcut, it comes up whole even if it was collapsed.
        let collapsed = !manual_start && store.setting(SET_COLLAPSED).is_some_and(|v| v == "1");
        let curtain_top = store.setting(SET_CURTAIN).is_none_or(|v| v != "bottom");
        let mut full_area = vec2(1280.0, 720.0);
        if let Some(h) = hwnd {
            let monitors = win::monitors();
            let saved = store.setting(SET_MONITOR);
            // The saved monitor if it's still connected, else the first secondary one.
            let monitor = saved.as_deref().and_then(|d| monitors.iter().find(|m| m.matches_device(d))).or(monitors.first());
            if let Some(m) = monitor {
                let (r, s) = (m.work, win::monitor_scale(m));
                full_area = vec2((r.right - r.left) as f32 / s, (r.bottom - r.top) as f32 / s);
                if collapsed {
                    place_tab(h, m, curtain_top);
                } else {
                    win::place_on(h, m);
                }
            }
            win::apply_backdrop(h, backdrop, collapsed);
            // Still hidden here (eframe shows it after the first frame), so the
            // taskbar never sees a button. Starts transparent; fades in after the
            // first frame.
            win::install_window_rules(h, win::LAYER | win::FADE);
            win::set_window_alpha(h, 0);
        }

        // Brought back from the archive today: into free slots, not where each
        // was when it was archived, and appearing like a new card.
        let appearing: Vec<(i64, Instant)> = fresh.iter().map(|id| (*id, Instant::now())).collect();
        let away = pos2(-1.0e5, -1.0e5);
        cards.iter_mut().filter(|c| fresh.contains(&c.id)).for_each(|c| c.pos = away);
        for id in &fresh {
            let Some(idx) = cards.iter().position(|c| c.id == *id) else { continue };
            cards[idx].pos = card::free_slot(&cards, full_area);
            cards[idx].size = cards[idx].size.max(MIN_SIZE);
            if let Err(e) = store.save(&cards[idx]) {
                eprintln!("save failed: {e}");
            }
        }
        // On a full layer they land on other cards: above them, not under.
        cards.sort_by_key(|c| fresh.contains(&c.id));
        for id in &fresh {
            let _ = store.raise(*id);
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
            collapsed,
            curtain_top,
            curtain: 0.0,
            curtain_anim: None,
            resize_pending: false,
            full_area,
            settings_on_launch,
            snap,
            card_style,
            theme_mode,
            manual_start,
            library: Arc::default(),
            editing: None,
            active: None,
            revealed: None,
            revealed_text: None,
            highlighted: None,
            toast: None,
            appearing,
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
        if self.cards[idx].kind == Kind::Private {
            // Encrypted by save; on the layer only the label stays in the open.
            let c = &mut self.cards[idx];
            c.title = card::private_parts(&c.title, &c.body).0;
            c.body.clear();
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
        if self.editing.as_ref().is_some_and(|(eid, _)| *eid == id) {
            self.commit_edit();
        }
        if self.revealed.is_some_and(|(rid, _)| rid == id) {
            self.revealed = None;
            self.revealed_text = None;
        }
        // The store splits the text into label and secret, or joins them back.
        if let Err(e) = self.store.set_kind(id, kind) {
            eprintln!("set kind failed: {e}");
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
        let origin = ui.max_rect().min + vec2(32.0, 20.0);
        let painter = ui.painter();
        let logo = Rect::from_min_size(origin, Vec2::splat(30.0));
        paint_logo(painter, logo);
        painter.text(
            pos2(logo.right() + 14.0, logo.center().y),
            Align2::LEFT_CENTER,
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
            theme::muted(),
        );
    }

    /// "⋯" in the top right corner of the layer: what the tray menu offers, without the tray.
    fn menu_ui(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let rect = Rect::from_min_size(pos2(full.right() - 32.0 - 40.0, full.top() + 16.0), vec2(40.0, 36.0));
        let button = ui.interact(rect, Id::new("layer-menu"), Sense::click());
        button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Меню"));
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&button));
        let t = hover_t(ui, button.id, open || button.hovered());
        glass_button(ui, rect.expand(t * 1.5), 10.0, t);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "\u{E712}", theme::icons(16.0), theme::dim().lerp_to_gamma(theme::text(), t));
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
                item(ui, "Еженедельный обзор", "", MenuItem::Review);
                ui.separator();
                item(ui, "Импорт из Sticky Notes…", "", MenuItem::Import);
                item(ui, "Настройки…", "", MenuItem::Settings);
                ui.separator();
                item(ui, if self.curtain_top { "Свернуть вверх" } else { "Свернуть вниз" }, "", MenuItem::Collapse);
                item(ui, dismiss_label, "Esc", MenuItem::Dismiss);
                item(ui, "Выход", "", MenuItem::Exit);
            });

        let Some(chosen) = chosen else { return };
        let event = match chosen {
            MenuItem::Capture => Event::Capture(Instant::now()),
            MenuItem::Search => Event::Search(Instant::now()),
            MenuItem::Library => Event::OpenLibrary(false),
            MenuItem::Review => Event::OpenReview,
            MenuItem::Import => Event::ImportSticky,
            MenuItem::Settings => Event::OpenLibrary(true),
            MenuItem::Exit => Event::Exit,
            MenuItem::Dismiss => {
                self.dismiss(ui.ctx());
                return;
            }
            MenuItem::Collapse => {
                self.curtain_anim = Some((Instant::now(), self.curtain, 1.0));
                ui.ctx().request_repaint();
                return;
            }
        };
        // Handled in logic() like the tray's, on the next pass.
        self.shell.events.lock().unwrap().push(event);
        ui.ctx().request_repaint();
    }

    /// Direction the layer slides as it collapses: up to the top edge or down to the bottom one.
    fn curtain_dir(&self) -> f32 {
        if self.curtain_top { -1.0 } else { 1.0 }
    }

    /// The handle at the top or bottom center of the layer: a click, or a drag
    /// toward that edge, rolls the layer into a tab, and the desktop under it is free.
    /// `full`: the layer's rect, taken before the cards are drawn (they can grow
    /// `ui.max_rect()` past the window's edge).
    fn curtain_ui(&mut self, ui: &mut Ui, full: Rect) {
        let top = self.curtain_top;
        let y = if top { full.top() + 18.0 } else { full.bottom() - 18.0 };
        let center = pos2(full.center().x, y);
        // The hit area is the handle's hovered size, so it doesn't jitter as the handle grows.
        let resp = ui.interact(Rect::from_center_size(center, vec2(96.0, 28.0)), Id::new("layer-curtain"), Sense::click_and_drag());
        let label = if top { "Свернуть вверх" } else { "Свернуть вниз" };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        // At rest a slim pill; on hover it widens and the arrow nudges toward the edge.
        let t = hover_t(ui, resp.id, resp.hovered() || resp.dragged());
        let rect = Rect::from_center_size(center, vec2(56.0 + 32.0 * t, 20.0 + 6.0 * t));
        glass_button(ui, rect, rect.height() / 2.0, t);
        let icon = if top { "\u{E70E}" } else { "\u{E70D}" };
        let nudge = vec2(0.0, self.curtain_dir() * 2.0 * t);
        let color = theme::dim().lerp_to_gamma(theme::text(), t);
        ui.painter().text(rect.center() + nudge, Align2::CENTER_CENTER, icon, theme::icons(12.0 + t), color);
        // A pointing hand, not a resize cursor: after a click the pointer stays over
        // the collapsed tab, and the cursor only changes once the mouse moves.
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);

        if resp.dragged() {
            self.curtain_anim = None;
            self.curtain = (self.curtain + self.curtain_dir() * resp.drag_delta().y / CURTAIN_DRAG).clamp(0.0, 1.0);
        }
        if resp.drag_stopped() {
            let to = if self.curtain > 0.3 { 1.0 } else { 0.0 };
            self.curtain_anim = Some((Instant::now(), self.curtain, to));
        } else if resp.clicked() {
            self.curtain_anim = Some((Instant::now(), self.curtain, 1.0));
        }
    }

    /// Steps the curtain's animation; the layer's window fades with it.
    fn step_curtain(&mut self, ctx: &egui::Context) {
        let active = self.curtain_anim.is_some() || self.curtain > 0.0;
        if let Some((at, from, to)) = self.curtain_anim {
            let t = at.elapsed().as_secs_f32() / CURTAIN_ANIM.as_secs_f32();
            self.curtain = from + (to - from) * panel_ease(t);
            if t >= 1.0 {
                self.curtain = to;
                self.curtain_anim = None;
            }
        }
        if !active {
            return;
        }
        // Also on the frame it comes back to 0, to leave the window opaque.
        if let Some(h) = self.hwnd {
            win::set_alpha_now(h, ((1.0 - self.curtain) * 255.0).round() as u8);
        }
        if self.curtain >= 1.0 && self.curtain_anim.is_none() {
            self.collapse(ctx);
        } else if self.curtain_anim.is_some() {
            ctx.request_repaint();
        }
    }

    /// Shrinks the (already faded out) layer into the tab at the edge of its monitor.
    fn collapse(&mut self, ctx: &egui::Context) {
        self.curtain = 0.0;
        self.curtain_anim = None;
        if self.collapsed {
            return;
        }
        self.commit_edit();
        self.active = None;
        self.collapsed = true;
        self.set_flag(SET_COLLAPSED, true);
        if let Some(h) = self.hwnd {
            win::set_alpha_now(h, 0);
            if let Some(m) = win::monitor_of(h) {
                place_tab(h, &m, self.curtain_top);
            }
            win::apply_backdrop(h, self.backdrop, true);
            if win::RAISED.load(Ordering::Relaxed) {
                // Summoned over the windows: the point was the desktop, go under them.
                win::lower(h);
            }
            self.resize_pending = true;
        }
        ctx.request_repaint();
    }

    /// Brings a collapsed layer back to its full size. True when it was collapsed.
    fn expand(&mut self, ctx: &egui::Context) -> bool {
        if !self.collapsed {
            // Mid-curtain (dragged or animating): back up.
            if self.curtain > 0.0 {
                self.curtain_anim = Some((Instant::now(), self.curtain, 0.0));
                ctx.request_repaint();
            }
            return false;
        }
        self.collapsed = false;
        self.set_flag(SET_COLLAPSED, false);
        if let Some(h) = self.hwnd {
            win::set_alpha_now(h, 0);
            if let Some(m) = win::monitor_of(h) {
                win::place_on(h, &m);
            }
            win::apply_backdrop(h, self.backdrop, false);
            self.resize_pending = true;
        }
        ctx.request_repaint();
        true
    }

    /// The collapsed layer: "Ebb" and the number of cards; a click brings the layer back.
    fn tab_ui(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let resp = ui.interact(full, Id::new("layer-tab"), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Развернуть Ebb"));
        let hovered = resp.hovered();
        let t = hover_t(ui, resp.id, hovered);
        let p = ui.painter();
        p.rect_filled(full, CornerRadius::ZERO, theme::scrim(self.tint));
        p.rect_filled(full, CornerRadius::ZERO, theme::glass_fill_hover().gamma_multiply(t));
        // A thin accent line on the side that faces the desktop, brighter on hover.
        let line_y = if self.curtain_top { full.bottom() - 1.0 } else { full.top() + 1.0 };
        let accent = Color32::from_rgb(45, 212, 191).gamma_multiply(0.35 + 0.5 * t);
        p.hline((full.center().x - 24.0 - 12.0 * t)..=(full.center().x + 24.0 + 12.0 * t), line_y, Stroke::new(2.0, accent));

        // Points the way the layer comes back, and moves that way on hover.
        let arrow = if self.curtain_top { "\u{E70D}" } else { "\u{E70E}" };
        let nudge = vec2(0.0, -self.curtain_dir() * 2.0 * t);
        let icon = p.layout_no_wrap(arrow.into(), theme::icons(12.0), theme::dim().lerp_to_gamma(theme::text(), t));
        let count = p.layout_no_wrap(self.cards.len().to_string(), theme::semibold(13.0), theme::muted().lerp_to_gamma(theme::text(), t));
        let logo = 22.0;
        let width = icon.size().x + 10.0 + logo + 10.0 + count.size().x;
        let mut x = full.center().x - width / 2.0;
        let cy = full.center().y;
        let icon_w = icon.size().x;
        p.galley(pos2(x, cy - icon.size().y / 2.0) + nudge, icon, theme::text());
        x += icon_w + 10.0;
        paint_logo(p, Rect::from_center_size(pos2(x + logo / 2.0, cy), Vec2::splat(logo + 2.0 * t)));
        x += logo + 10.0;
        p.galley(pos2(x, cy - count.size().y / 2.0), count, theme::text());

        if resp.contains_pointer() {
            // Also right after collapsing, before the pointer moves.
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.expand(ui.ctx());
        }
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
        let rects: Vec<(i64, Rect)> = self.cards.iter().map(|c| (c.id, Rect::from_min_size(c.pos, c.size))).collect();
        let magnet = self.snap.then_some(rects.as_slice());
        let style = self.card_style;
        let mut actions: Vec<(usize, Action)> = Vec::new();
        for idx in 0..self.cards.len() {
            let id = self.cards[idx].id;
            let editing = self.editing.as_mut().filter(|(eid, _)| *eid == id).map(|(_, b)| b);
            let revealed = self.revealed.is_some_and(|(rid, _)| rid == id);
            let revealed_text = self.revealed_text.as_ref().filter(|(rid, _)| *rid == id).map(|(_, text)| text.as_str());
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
                    card_ui(ui, origin + vec2(0.0, (1.0 - appear) * 10.0), area, card, hovered, editing, revealed, revealed_text, active, magnet, style)
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
                let _ = card_ui(ui, origin + vec2(0.0, t * 6.0), area, card, false, None, false, None, false, None, style);
            });
        }

        if let Some((hid, t)) = self.highlighted {
            match self.cards.iter().find(|c| c.id == hid) {
                Some(c) if t.elapsed() < HIGHLIGHT_FOR => {
                    let fade = 1.0 - t.elapsed().as_secs_f32() / HIGHLIGHT_FOR.as_secs_f32();
                    let rect = Rect::from_min_size(origin + c.pos.to_vec2(), c.size).expand(3.0);
                    let stroke = Stroke::new(2.0, c.accent().gamma_multiply(fade));
                    ui.painter().rect_stroke(rect, CornerRadius::same(self.card_style.radius + 2), stroke, StrokeKind::Outside);
                    ui.ctx().request_repaint();
                }
                _ => self.highlighted = None,
            }
        }

        // 1–8 change the selected card's kind (not while typing anywhere).
        if let Some(idx) = self.active.and_then(|a| self.cards.iter().position(|c| c.id == a))
            && self.editing.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
        {
            const DIGITS: [Key; 8] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8];
            let pressed = ui.input_mut(|i| DIGITS.iter().position(|k| i.consume_key(Modifiers::NONE, *k)));
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
        for (idx, action) in actions {
            match action {
                Action::Front => {
                    to_front = Some(idx);
                    self.active = Some(self.cards[idx].id);
                    if self.cards[idx].placement == Placement::Rediscover {
                        self.cards[idx].placement = Placement::Manual;
                        self.save(idx);
                    }
                    let _ = self.store.touch(self.cards[idx].id);
                }
                Action::Moved => {
                    self.cards[idx].placement = Placement::Manual;
                    self.save(idx);
                }
                Action::TogglePin => {
                    self.cards[idx].pinned ^= true;
                    self.cards[idx].placement = if self.cards[idx].pinned { Placement::Pinned } else { Placement::Manual };
                    self.save(idx);
                }
                Action::Copy => {
                    let c = &self.cards[idx];
                    let _ = self.store.touch(c.id);
                    if c.kind == Kind::Private {
                        if let Ok(Some(secret)) = self.store.secret(c.id) {
                            let _ = win::copy_private(&secret);
                        }
                    } else {
                        let text = if c.title.is_empty() { c.body.clone() } else { format!("{}\n{}", c.title, c.body) };
                        ui.ctx().copy_text(rich_text::strip_markup(&text));
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
                    let parsed = card::Parsed { kind: src.kind, title: src.title, body, tags: src.tags };
                    match self.store.insert(&parsed, pos) {
                        Ok(mut copy) => {
                            copy.size = src.size;
                            copy.tint = src.tint;
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
                    self.cards[idx].placement = Placement::Archive;
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
                    let _ = self.store.touch(c.id);
                    let text = if c.kind == Kind::Private {
                        // Without its value the editor would save the label as the secret.
                        match self.store.secret(c.id) {
                            Ok(Some(secret)) => card::private_text(&c.title, &secret),
                            _ => continue,
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
                Action::SetKind(kind) => self.set_kind(idx, kind),
                Action::SetTint(tint) => {
                    self.cards[idx].tint = tint;
                    self.save(idx);
                }
            }
        }
        if let Some(idx) = remove {
            let card = self.cards.remove(idx);
            self.leaving.push((card, Instant::now()));
            self.library.lock().unwrap().invalidate();
            self.bar.lock().unwrap().invalidate();
        } else if let Some(idx) = to_front
            && idx + 1 != self.cards.len()
        {
            let c = self.cards.remove(idx);
            if let Err(e) = self.store.raise(c.id) {
                eprintln!("raise failed: {e}");
            }
            self.cards.push(c);
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
            let text = ui.painter().layout_no_wrap(toast.text.clone(), font, theme::text());
            let undo_w = if toast.undo.is_some() { 96.0 } else { 0.0 };
            let size = vec2(text.size().x + undo_w + 40.0, 44.0);
            let full = ui.max_rect();
            let center = pos2(full.center().x, full.bottom() - 48.0 - size.y / 2.0 + (1.0 - appear) * 12.0);
            let rect = Rect::from_center_size(center, size);
            glass_panel(ui, rect, false);
            ui.painter().galley(pos2(rect.left() + 20.0, rect.center().y - text.size().y / 2.0), text, theme::text());

            if let Some(action) = toast.undo {
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
            match action {
                Undo::Unarchive(id) => {
                    let _ = self.store.set_archived(id, false);
                }
                Undo::Restore(id) => {
                    let _ = self.store.restore(id);
                }
                Undo::Kind(id, kind) => {
                    let _ = self.store.set_kind(id, kind);
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
            ui.label(RichText::new("Debug").font(theme::semibold(15.0)).color(theme::text()));
            ui.add_space(4.0);
            let (proc_ms, main_ms) = self.first_frame.unwrap_or((f64::NAN, f64::NAN));
            let (ws, private) = win::memory_mib();
            let latency = self.bar.lock().unwrap().latency_ms;
            let row = |ui: &mut Ui, k: &str, v: String| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).size(13.0).color(theme::muted()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(v).size(13.0).color(theme::text()));
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
            curtain_top: self.curtain_top,
            settings_on_launch: self.settings_on_launch,
            snap: self.snap,
            card_style: self.card_style,
            theme_mode: self.theme_mode,
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
        // Collapsed: grows back at alpha 0 and fades in once redrawn at full size
        // (this cancels the fade above).
        let expanded = self.expand(ctx);
        let Some(h) = self.hwnd else { return };
        if already_up && !expanded {
            win::raise(h);
            return;
        }
        if was_visible && !expanded {
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

    fn set_card_style(&mut self, ctx: &egui::Context, style: card::CardStyle) {
        let background_changed = style.background != self.card_style.background;
        self.card_style = style;
        theme::set_card_font(ctx, style.font);
        if background_changed {
            self.refresh_theme(ctx);
        }
        let _ = self.store.set_setting(SET_CARD_STYLE, &style.to_setting());
        self.library.lock().unwrap().settings.card_style = style;
        ctx.request_repaint_of(ViewportId::ROOT);
        ctx.request_repaint_of(library::viewport_id());
    }

    /// Re-reads Windows' colors and repaints every window in the resulting theme.
    fn refresh_theme(&mut self, ctx: &egui::Context) {
        theme::apply(ctx, self.theme_mode, self.card_style.background, &win::system_colors());
        if let Some(h) = self.hwnd {
            win::apply_backdrop(h, self.backdrop, self.collapsed);
        }
        for h in [win::find_capture_window(), win::find_library_window()].into_iter().flatten() {
            win::apply_backdrop(h, Backdrop::AccentAcrylic, true);
        }
        ctx.request_repaint_of(ViewportId::ROOT);
        ctx.request_repaint_of(library::viewport_id());
    }

    fn set_flag(&self, key: &str, on: bool) {
        let _ = self.store.set_setting(key, if on { "1" } else { "0" });
    }

    fn set_monitor(&mut self, device: &str) {
        let monitors = win::monitors();
        if let (Some(m), Some(h)) = (monitors.iter().find(|m| m.matches_device(device)), self.hwnd) {
            if self.collapsed {
                place_tab(h, m, self.curtain_top);
            } else {
                win::place_on(h, m);
            }
            let _ = self.store.set_setting(SET_MONITOR, device);
        }
    }

    fn set_curtain_top(&mut self, top: bool) {
        self.curtain_top = top;
        let _ = self.store.set_setting(SET_CURTAIN, if top { "top" } else { "bottom" });
        // A collapsed tab moves to the other edge right away.
        if let (true, Some(h)) = (self.collapsed, self.hwnd)
            && let Some(m) = win::monitor_of(h)
        {
            place_tab(h, &m, top);
        }
    }

    fn set_backdrop(&mut self, backdrop: Backdrop) {
        self.backdrop = backdrop;
        if let Some(h) = self.hwnd {
            win::apply_backdrop(h, backdrop, self.collapsed);
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
            Request::SetCurtainTop(top) => self.set_curtain_top(top),
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
            Request::SetCardStyle(style) => self.set_card_style(ctx, style),
            Request::SetTheme(mode) => {
                self.theme_mode = mode;
                let _ = self.store.set_setting(SET_THEME, mode.key());
                self.refresh_theme(ctx);
            }
            Request::ImportSticky => {
                self.set_layer_visible(ctx, true);
                self.expand(ctx);
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
        // A Private value copied less than 30 s ago doesn't outlive the app.
        win::clear_private_clipboard();
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
        self.expand(ctx);
        let idx = match self.cards.iter().position(|c| c.id == id) {
            Some(idx) => idx,
            None => {
                let Ok(Some(mut card)) = self.store.card(id) else { return };
                card.archived = false;
                card.placement = Placement::Manual;
                card.pos = card::free_slot(&self.cards, self.full_area);
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
        let _ = self.store.raise(id);
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
            // A card back from the archive (pinned in the review, say) keeps its old
            // spot if it's still on the layer; an imported one never had one.
            let layer = Rect::from_min_size(Pos2::ZERO, self.full_area);
            for idx in 0..self.cards.len() {
                let c = &self.cards[idx];
                let fresh = self.appearing.iter().any(|(id, t)| *id == c.id && *t == now);
                if fresh && (c.pos == Pos2::ZERO || !layer.contains_rect(Rect::from_min_size(c.pos, c.size))) {
                    self.cards[idx].pos = pos2(-1.0e5, -1.0e5);
                    self.cards[idx].pos = card::free_slot(&self.cards, self.full_area);
                    self.save(idx);
                }
            }
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
                Event::ToggleLayer if self.layer_visible && !self.collapsed => self.set_layer_visible(ctx, false),
                Event::ToggleLayer => self.summon(ctx),
                Event::TrayClick => self.tray_click(ctx),
                Event::Launched => {
                    self.summon(ctx);
                    if self.settings_on_launch {
                        self.open_settings(ctx, false);
                    }
                }
                Event::SystemColors => self.refresh_theme(ctx),
                Event::TogglePinBottom => self.set_pin_bottom(!win::PIN_BOTTOM.load(Ordering::Relaxed)),
                Event::Exit => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
                Event::ImportSticky => self.apply_library_request(ctx, Request::ImportSticky),
                Event::OpenLibrary(true) => self.open_settings(ctx, false),
                event @ (Event::OpenLibrary(false) | Event::OpenReview) => {
                    let mut lib = self.library.lock().unwrap();
                    lib.settings = self.layer_settings();
                    lib.open(if matches!(event, Event::OpenReview) { Tab::Review } else { Tab::Archive });
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
                    self.add_from_capture(&text, self.full_area);
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

        let (f1, f2, f3, f4, esc) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::F1),
                i.consume_key(Modifiers::NONE, Key::F2),
                i.consume_key(Modifiers::NONE, Key::F3),
                i.consume_key(Modifiers::NONE, Key::F4),
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
        if f4 {
            // Next card marker, to compare them on the real layer.
            let all = card::Marker::ALL;
            let next = all[(all.iter().position(|m| *m == self.card_style.marker).unwrap_or(0) + 1) % all.len()];
            self.set_card_style(ui.ctx(), card::CardStyle { marker: next, ..self.card_style });
            self.toast = Some(Toast::new(format!("Вид карточек: {} (F4 — дальше)", next.label()), None));
        }
        if esc && !self.collapsed {
            if self.editing.is_some() {
                self.commit_edit();
            } else if egui::Popup::is_any_open(ui.ctx()) {
                // The menu closes itself on Esc; the layer stays.
            } else {
                self.dismiss(ui.ctx());
            }
        }

        let full = ui.max_rect();
        // Told apart by size: the window may still be at its old size for a frame
        // after a resize.
        let at_tab_size = full.height() < TAB_SIZE.1 * 2.0;
        if self.resize_pending && at_tab_size == self.collapsed {
            self.resize_pending = false;
            if let Some(h) = self.hwnd {
                win::fade(h, 0, 255, LAYER_FADE_IN, || {});
            }
            if !self.collapsed {
                let now = Instant::now();
                self.appearing = self.cards.iter().map(|c| (c.id, now)).collect();
            }
        }
        if self.resize_pending {
            // Transparent until then; nothing to draw at the wrong size.
            ui.ctx().request_repaint();
        } else if self.collapsed {
            self.tab_ui(ui);
        } else {
            self.full_area = full.size();
            self.step_curtain(ui.ctx());
            ui.painter().rect_filled(full, CornerRadius::ZERO, theme::scrim(self.tint));
            // Slides down as the curtain closes.
            let drop = vec2(0.0, self.curtain_dir() * self.curtain * CURTAIN_DROP);
            let moved = full.translate(drop);
            // Cards in their own scope: one past the window's edge grows its ui's
            // max_rect, and the panels below place themselves by the window's edges.
            ui.scope_builder(UiBuilder::new().max_rect(moved), |ui| {
                self.header(ui);
                self.cards_ui(ui);
            });
            ui.scope_builder(UiBuilder::new().max_rect(moved), |ui| {
                self.sticky.ui(ui, &mut self.store, &mut self.cards, self.hwnd);
                self.toast_ui(ui);
                self.menu_ui(ui);
                if self.show_debug {
                    self.debug_ui(ui);
                }
                self.curtain_ui(ui, moved);
            });
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

/// The Ebb logo in a square `rect`, drawn from the geometry of scripts/make-icon.py
/// (a 140-unit square), so it stays sharp at any size. Below ~24 px it keeps
/// only the card and one bold wave, like the small icon sizes.
pub(crate) fn paint_logo(painter: &egui::Painter, rect: Rect) {
    const BG: Color32 = Color32::from_rgb(30, 33, 40);
    const CARD: Color32 = Color32::from_rgb(52, 211, 153);
    const INK: Color32 = Color32::from_rgb(11, 59, 43);
    const WAVE: Color32 = Color32::from_rgb(45, 212, 191);
    let k = rect.width() / 140.0;
    let at = |x: f32, y: f32| rect.min + vec2(x, y) * k;
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32, fill: Color32| {
        painter.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), CornerRadius::same((r * k).round() as u8), fill);
    };
    let wave = |y: f32, width: f32, color: Color32| {
        let points: Vec<Pos2> = (0..=48)
            .map(|i| {
                let x = 22.0 + i as f32 * 2.0;
                at(x, y - 5.0 * (std::f32::consts::PI * (x - 22.0) / 24.0).sin())
            })
            .collect();
        let r = width * k / 2.0;
        painter.circle_filled(points[0], r, color);
        painter.circle_filled(points[points.len() - 1], r, color);
        painter.add(egui::Shape::line(points, Stroke::new(width * k, color)));
    };

    rrect(0.0, 0.0, 140.0, 140.0, 32.0, BG);
    if rect.width() * painter.ctx().pixels_per_point() <= 24.0 {
        rrect(34.0, 22.0, 106.0, 78.0, 12.0, CARD);
        wave(108.0, 13.0, WAVE);
    } else {
        let edge = Rect::from_min_max(at(0.5, 0.5), at(139.5, 139.5));
        painter.rect_stroke(edge, CornerRadius::same((32.0 * k).round() as u8), Stroke::new((1.2 * k).max(0.6), Color32::from_white_alpha(30)), StrokeKind::Inside);
        rrect(42.0, 26.0, 98.0, 70.0, 10.0, CARD);
        rrect(54.0, 40.0, 86.0, 45.0, 2.5, INK);
        rrect(54.0, 51.0, 74.0, 56.0, 2.5, INK);
        wave(90.0, 6.0, WAVE);
        wave(110.0, 6.0, WAVE.gamma_multiply(115.0 / 255.0));
    }
}

/// 0..1 hover progress of a widget, eased over a short time.
fn hover_t(ui: &Ui, id: Id, hovered: bool) -> f32 {
    panel_ease(ui.ctx().animate_bool_with_time(id.with("hover"), hovered, 0.14))
}

/// A glass button face at hover progress `t`: fill and edge brighten, a soft
/// shadow comes up under it.
fn glass_button(ui: &Ui, rect: Rect, radius: f32, t: f32) {
    let p = ui.painter();
    let r = CornerRadius::same(radius.round() as u8);
    if t > 0.0 {
        p.rect_filled(rect.translate(vec2(0.0, 1.5)).expand(1.0), r, Color32::from_black_alpha((40.0 * t) as u8));
    }
    let fill = theme::glass_fill().lerp_to_gamma(theme::glass_fill_hover(), t);
    let stroke = theme::glass_stroke().lerp_to_gamma(theme::text().gamma_multiply(0.35), t);
    p.rect(rect, r, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
}

pub(crate) fn glass_panel(ui: &Ui, rect: Rect, hovered: bool) {
    let fill = if hovered { theme::glass_fill_hover() } else { theme::glass_fill() };
    panel(ui, rect, fill, CornerRadius::same(12), 50);
}

/// A card's panel: glass, or the Start/taskbar color (a setting).
fn card_panel(ui: &Ui, rect: Rect, hovered: bool, style: card::CardStyle) {
    panel(ui, rect, card_background(theme::card_fill(hovered), style, hovered), CornerRadius::same(style.radius), style.shadow);
}

/// `fill` at the style's opacity; a hovered card is a little more solid.
fn card_background(fill: Color32, style: card::CardStyle, hovered: bool) -> Color32 {
    let alpha = (f32::from(style.opacity) * 2.55 + if hovered { 12.0 } else { 0.0 }).min(255.0) as u8;
    Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), alpha)
}

/// `shadow` 0–100; 50 is the soft shadow floating panels always had.
fn panel(ui: &Ui, rect: Rect, fill: Color32, radius: CornerRadius, shadow: u8) {
    let painter = ui.painter();
    if shadow > 0 {
        let k = f32::from(shadow) / 50.0;
        let alpha = if theme::is_light() { 28.0 } else { 70.0 } * k;
        painter.add(
            egui::epaint::Shadow {
                offset: [0, (8.0 * k.min(1.5)).round() as i8],
                blur: (28.0 * k.min(1.5)).round() as u8,
                spread: 0,
                color: Color32::from_black_alpha(alpha.min(160.0) as u8),
            }
            .as_shape(rect, radius),
        );
    }
    painter.rect(rect, radius, fill, Stroke::new(1.0, theme::glass_stroke()), StrokeKind::Inside);
}

/// The card's color, drawn the way the user picked in settings. `strength` 50
/// (the default) is the look each marker was tuned at.
pub(crate) fn paint_marker(ui: &Ui, rect: Rect, accent: Color32, style: card::CardStyle, hovered: bool) {
    use card::Marker;
    let s = f32::from(style.strength) / 50.0;
    let radius = CornerRadius::same(style.radius);
    let painter = ui.painter();
    // A band along one edge: the rounded rect clipped, so it follows the corners.
    let band = |band: Rect| {
        painter.with_clip_rect(band.intersect(ui.clip_rect())).rect_filled(rect, radius, accent.gamma_multiply((0.85 * s).min(1.0)));
    };
    let w = f32::from(style.strip);
    match style.marker {
        Marker::None => {}
        Marker::Glow => corner_glow(painter, rect, f32::from(style.radius), accent, (0.2 * s).min(0.6)),
        Marker::StripTop => band(Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + w))),
        Marker::StripLeft => band(Rect::from_min_max(rect.min, pos2(rect.min.x + w, rect.max.y))),
        Marker::Tint => {
            painter.rect_filled(rect, radius, accent.gamma_multiply((0.1 * s).min(0.4)));
        }
        Marker::Border => {
            painter.rect_stroke(rect, radius, Stroke::new(1.5, accent.gamma_multiply((0.55 * s).min(1.0))), StrokeKind::Inside);
        }
        Marker::Fill => {
            // Opaque, like paper: a muted shade of the color over the card's own
            // background (dark shades in the dark theme, pastels in the light one).
            let base = theme::card_fill(false);
            let t = if theme::is_light() { 0.38 } else { 0.3 } * s.min(2.0);
            let l = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t.min(0.9)).round() as u8;
            let fill = card_background(Color32::from_rgb(l(base.r(), accent.r()), l(base.g(), accent.g()), l(base.b(), accent.b())), style, false);
            painter.rect_filled(rect, radius, fill);
            if hovered {
                painter.rect_filled(rect, radius, theme::wash(10));
            }
        }
        Marker::Outline => {
            painter.rect_filled(rect, radius, accent.gamma_multiply((0.07 * s).min(0.3)));
            painter.rect_stroke(rect, radius, Stroke::new(2.0, accent.gamma_multiply((0.8 * s).min(1.0))), StrokeKind::Inside);
        }
    }
}

/// A soft wash of `color` spreading from the top left corner of a rounded card.
/// egui has no radial gradient: a grid mesh with per-vertex alpha, its outer
/// vertices pulled inside the rounded corners so nothing spills past them.
fn corner_glow(painter: &egui::Painter, rect: Rect, radius: f32, color: Color32, strength: f32) {
    const STEP: f32 = 14.0;
    let reach = (rect.width().max(rect.height()) * 0.75).min(240.0);
    let area = Rect::from_min_size(rect.min, vec2(reach.min(rect.width()), reach.min(rect.height())));
    let (nx, ny) = ((area.width() / STEP).ceil() as u32, (area.height() / STEP).ceil() as u32);
    let inside = |p: Pos2| {
        // Nearest point of the rounded rect: clamp into the corner's circle.
        let c = pos2(p.x.clamp(rect.left() + radius, rect.right() - radius), p.y.clamp(rect.top() + radius, rect.bottom() - radius));
        let d = p - c;
        if d.length() > radius { c + d.normalized() * radius } else { p }
    };
    let mut mesh = egui::Mesh::default();
    for j in 0..=ny {
        for i in 0..=nx {
            let p = pos2(
                (area.left() + i as f32 * STEP).min(area.right()),
                (area.top() + j as f32 * STEP).min(area.bottom()),
            );
            let p = inside(p);
            let t = (1.0 - (p - rect.min).length() / reach).max(0.0);
            mesh.colored_vertex(p, color.gamma_multiply(strength * t * t));
        }
    }
    let row = nx + 1;
    for j in 0..ny {
        for i in 0..nx {
            let k = j * row + i;
            mesh.add_triangle(k, k + 1, k + row);
            mesh.add_triangle(k + 1, k + row + 1, k + row);
        }
    }
    painter.add(mesh);
}

fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme::wash(22));
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

/// Text of a card, or its editor. True when a click opened a link in the text.
fn card_body(
    ui: &mut Ui,
    card: &Card,
    editing: Option<&mut String>,
    revealed: bool,
    revealed_text: Option<&str>,
    style: card::CardStyle,
) -> bool {
    let base = style.text_size();
    let editor_id = Id::new(("card", card.id)).with("editor");
    // Where the last click in the text landed, in chars of the editor's text.
    let click_id = editor_id.with("click");
    if let Some(buf) = editing {
        let mut toolbar_action = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            toolbar_action = format_button(ui, "B", "Жирный (Ctrl+B)").then_some(rich_text::Action::Bold);
            if toolbar_action.is_none() {
                toolbar_action = format_button(ui, "I", "Курсив (Ctrl+I)").then_some(rich_text::Action::Italic);
            }
            if toolbar_action.is_none() {
                toolbar_action = format_button(ui, "U", "Подчёркивание (Ctrl+U)").then_some(rich_text::Action::Underline);
            }
            if toolbar_action.is_none() {
                toolbar_action = format_button(ui, "S", "Зачёркивание (Ctrl+Shift+S)").then_some(rich_text::Action::Strikethrough);
            }
            if toolbar_action.is_none() {
                toolbar_action = format_button(ui, "H", "Выделение маркером (Ctrl+Shift+H)").then_some(rich_text::Action::Highlight);
            }
            if toolbar_action.is_none() {
                toolbar_action = format_button(ui, "`_`", "Моноширинный код (Ctrl+Shift+K)").then_some(rich_text::Action::Code);
            }
            if toolbar_action.is_none() {
                toolbar_action = format_button(ui, "Tx", "Снять форматирование").then_some(rich_text::Action::Clear);
            }
        });
        let shortcut_action = ui
            .memory(|m| m.has_focus(editor_id))
            .then(|| {
                ui.input(|input| {
                    input.events.iter().find_map(|event| match event {
                        egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                            if modifiers.command && !modifiers.alt => match (key, modifiers.shift) {
                                (Key::B, false) => Some(rich_text::Action::Bold),
                                (Key::I, false) => Some(rich_text::Action::Italic),
                                (Key::U, false) => Some(rich_text::Action::Underline),
                                (Key::S, true) => Some(rich_text::Action::Strikethrough),
                                (Key::H, true) => Some(rich_text::Action::Highlight),
                                (Key::K, true) => Some(rich_text::Action::Code),
                                _ => None,
                            },
                        _ => None,
                    })
                })
            })
            .flatten();
        if let Some(action) = toolbar_action.or(shortcut_action) {
            apply_editor_action(ui, editor_id, buf, action);
        }
        // Opening: the cursor goes where the text was clicked, else to the end,
        // never where it was the last time this card was edited.
        if !ui.memory(|m| m.has_focus(editor_id)) {
            let at = ui.data_mut(|d| d.remove_temp::<usize>(click_id)).unwrap_or_else(|| buf.chars().count());
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), editor_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
            state.store(ui.ctx(), editor_id);
        }
        let resp = ui.add(
            egui::TextEdit::multiline(buf)
                .id(editor_id)
                .font(theme::card_font(base))
                .text_color(theme::card_text())
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        return false;
    }
    if let Some(reason) = card.resurface_reason(crate::resurface::unix_now()) {
        ui.label(RichText::new(reason).size(11.5).color(theme::card_dim()));
        ui.add_space(3.0);
    }
    let body = without_tags(if card.kind == Kind::Private && revealed { revealed_text.unwrap_or("") } else { &card.body });
    if card.kind == Kind::Private && !revealed {
        // The heading stays readable (see card::private_label); only the rest is barred.
        let label = (!card.title.is_empty()).then(|| card.title.clone());
        let rest = "••••••••••".to_owned();
        let heading_at = label.as_ref().and_then(|label| {
            let job = egui::text::LayoutJob::simple(label.clone(), theme::card_bold(base + 0.5), theme::card_text(), ui.available_width());
            let (pos, galley, resp) = egui::Label::new(job).wrap().selectable(false).layout_in_ui(ui);
            let at = char_at(ui, &galley, pos, resp.rect);
            ui.painter().galley(pos, galley, theme::card_text());
            at
        });
        let size = if label.is_none() { base } else { base - 1.0 };
        let rest_at = redacted(ui, &rest, theme::card_font(size), theme::card_dim().gamma_multiply(0.45));
        remember_click(ui, click_id, card, label.as_deref(), heading_at, &rest, rest_at);
        return false;
    }
    // A Prompt's first line is its name.
    let (heading, text) = match card.kind {
        Kind::Prompt if card.title.is_empty() => match card::prompt_name(&body) {
            Some((name, rest)) => (Some(name.to_owned()), rest.to_owned()),
            None => (None, body),
        },
        _ => ((!card.title.is_empty()).then(|| card.title.clone()), body),
    };
    let displayed_heading = heading.as_ref().map(|heading| rich_text::strip_markup(heading));
    let heading_at = heading.as_ref().and_then(|heading| {
        let heading = rich_text::strip_markup(heading);
        let job = egui::text::LayoutJob::simple(heading, theme::card_bold(base + 0.5), theme::card_text(), ui.available_width());
        let (pos, galley, resp) = egui::Label::new(job).wrap().selectable(false).layout_in_ui(ui);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, galley.text()));
        let at = char_at(ui, &galley, pos, resp.rect);
        ui.painter().galley(pos, galley, theme::card_text());
        at
    });
    let color = if heading.is_none() { theme::card_text() } else { theme::card_dim() };
    let size = if heading.is_none() { base } else { base - 1.0 };
    // Addresses in link blue and clickable, in any kind; in a Reference, commands,
    // paths and hosts in monospace, the words around them as usual.
    let mut job = egui::text::LayoutJob::default();
    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new(); // char ranges in the galley
    let mut chars = 0;
    for span in rich_text::spans(&text) {
        for piece in span.text.split_inclusive('\n') {
            let (line, newline) = piece.strip_suffix('\n').map_or((piece, ""), |line| (line, "\n"));
            let technical = card.kind == Kind::Reference && card::looks_technical(line);
            // Returns the galley's length so far, in chars.
            let mut append = |s: &str, c: Color32| {
                let from = chars;
                let font = if span.style.code || technical {
                    FontId::monospace(size - 1.0)
                } else if span.style.bold {
                    theme::card_bold(size)
                } else {
                    theme::card_font(size)
                };
                job.append(s, 0.0, rich_format(font, c, span.style));
                chars += s.chars().count();
                from..chars
            };
            let mut rest = line;
            while let Some(start) = [rest.find("https://"), rest.find("http://")].into_iter().flatten().min() {
                let end = rest[start..].find(char::is_whitespace).map_or(rest.len(), |e| start + e);
                // "(see https://x.org)." — the closing punctuation isn't part of the address.
                let url = rest[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']);
                let end = start + url.len();
                append(&rest[..start], color);
                let range = append(url, theme::link());
                links.push((range, url.to_owned()));
                rest = &rest[end..];
            }
            append(rest, color);
            append(newline, color);
        }
    }
    let (pos, galley, resp) = egui::Label::new(job).wrap().selectable(false).layout_in_ui(ui);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, galley.text()));
    let text_at = char_at(ui, &galley, pos, resp.rect);
    let link = text_at.and_then(|at| links.iter().find(|(range, _)| range.contains(&at)).map(|(_, url)| url.as_str()));
    ui.painter().galley(pos, galley, color);
    let display_text = rich_text::strip_markup(&text);
    remember_click(ui, click_id, card, displayed_heading.as_deref(), heading_at, &display_text, text_at);
    let mut link_clicked = false;
    if let Some(url) = link {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        if ui.input(|i| i.pointer.primary_clicked()) {
            win::open_url(url);
            link_clicked = true;
        }
    }
    if card.kind == Kind::Link
        && let Some(domain) = card::link_domain(&card.body)
    {
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(domain).size(12.0).color(theme::card_muted())).selectable(false));
    }
    link_clicked
}

fn rich_format(font: FontId, color: Color32, style: rich_text::Style) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font,
        color,
        italics: style.italic,
        background: if style.highlight { Color32::from_rgba_unmultiplied(250, 204, 21, 42) } else { Color32::TRANSPARENT },
        underline: if style.underline { Stroke::new(1.0, color) } else { Stroke::NONE },
        strikethrough: if style.strikethrough { Stroke::new(1.0, color) } else { Stroke::NONE },
        ..Default::default()
    }
}

fn format_button(ui: &mut Ui, label: &str, tip: &str) -> bool {
    ui.add_sized(
        vec2(30.0, 22.0),
        egui::Button::new(RichText::new(label).size(12.5).color(theme::card_text())),
    )
    .on_hover_text(tip)
    .clicked()
}

fn apply_editor_action(ui: &Ui, editor_id: Id, buf: &mut String, action: rich_text::Action) {
    let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), editor_id) else { return };
    let cursor = state.cursor.char_range().unwrap_or(egui::text::CCursorRange::one(egui::text::CCursor::new(0)));
    let range = cursor.as_sorted_char_range();
    let (text, selection) = rich_text::apply(buf, range.start.0..range.end.0, action);
    *buf = text;
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(selection.start),
        egui::text::CCursor::new(selection.end),
    )));
    state.store(ui.ctx(), editor_id);
}

/// What the editor opens with: the note as written, an old separate title as its first line.
fn edit_text(card: &Card) -> String {
    if card.title.is_empty() { card.body.clone() } else { format!("{}\n{}", card.title, card.body) }
}

/// The char under the pointer in a galley painted at `pos` over `rect`.
/// Not `Response::hover_pos`: while the card's background is being clicked egui
/// hovers nothing else, so a label would never see the click.
fn char_at(ui: &Ui, galley: &egui::Galley, pos: Pos2, rect: Rect) -> Option<usize> {
    let p = ui.input(|i| i.pointer.interact_pos()).filter(|_| ui.rect_contains_pointer(rect))?;
    Some(galley.cursor_from_pos(p - pos).index.0)
}

/// On a click in a card's text, keeps where it landed in the editor's text, for
/// the cursor when the click opens the editor. `heading_at`/`text_at`: chars in
/// the shown heading and text.
fn remember_click(
    ui: &Ui,
    click_id: Id,
    card: &Card,
    heading: Option<&str>,
    heading_at: Option<usize>,
    text: &str,
    text_at: Option<usize>,
) {
    if !ui.input(|i| i.pointer.primary_clicked()) {
        return;
    }
    let shown = match heading {
        Some(h) => format!("{h}\n{text}"),
        None => text.to_owned(),
    };
    let heading_len = heading.map_or(0, |h| h.chars().count() + 1);
    let at = match (heading_at, text_at) {
        (Some(at), _) => at,
        (None, Some(at)) => heading_len + at,
        (None, None) => return ui.data_mut(|d| d.remove::<usize>(click_id)),
    };
    let at = shown_to_source(&shown, at, &edit_text(card));
    ui.data_mut(|d| d.insert_temp(click_id, at));
}

/// Where char `at` of `shown` sits in `source`. The shown text is the source with
/// bits left out (tags, trimmed space), so its chars are matched in order.
fn shown_to_source(shown: &str, at: usize, source: &str) -> usize {
    let mut rest = source.chars().enumerate();
    let mut end = 0;
    for c in shown.chars().take(at) {
        match rest.by_ref().find(|&(_, s)| s == c) {
            Some((i, _)) => end = i + 1,
            None => return source.chars().count(),
        }
    }
    end
}

/// Text laid out as usual but painted as one bar per word: the note keeps its
/// shape (line and word lengths, paragraphs) without a readable letter.
/// Returns the char under the pointer, like [`char_at`].
fn redacted(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> Option<usize> {
    let galley = ui.painter().layout(text.to_owned(), font, color, ui.available_width());
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    let at = char_at(ui, &galley, rect.min, rect);
    let p = ui.painter();
    for row in &galley.rows {
        let r = row.rect().translate(rect.min.to_vec2());
        let y = (r.top() + r.height() * 0.28)..=(r.bottom() - r.height() * 0.2);
        let bar = |(a, b): (f32, f32)| p.rect_filled(Rect::from_x_y_ranges((r.left() + a)..=(r.left() + b), y.clone()), 2.0, color);
        let mut run: Option<(f32, f32)> = None;
        for g in &row.glyphs {
            if g.chr.is_whitespace() {
                run.take().map(bar);
            } else {
                run = Some((run.map_or(g.pos.x, |(a, _)| a), g.pos.x + g.advance_width));
            }
        }
        run.map(bar);
    }
    at
}

/// Menu under a card's kind: every kind (with its digit key), then the card's color.
/// `below`: the anchor is at the top of the card, so the menu opens downwards.
fn kind_menu(anchor: &egui::Response, card: &Card, out: &mut Vec<Action>, below: bool) {
    egui::Popup::menu(anchor)
        .align(if below { egui::RectAlign::BOTTOM_START } else { egui::RectAlign::TOP_END })
        .gap(4.0)
        .width(236.0)
        .show(|ui| {
            ui.spacing_mut().button_padding = vec2(8.0, 5.0);
            for kind in Kind::ALL {
                let mut job = egui::text::LayoutJob::default();
                let format = |font: FontId, color: Color32| egui::TextFormat { font_id: font, color, valign: Align::Center, ..Default::default() };
                job.append(kind.icon(), 0.0, format(theme::icons(13.0), kind.accent()));
                job.append(kind.label(), 10.0, format(FontId::proportional(14.0), theme::text()));
                let button = egui::Button::new(job)
                    .selected(kind == card.kind)
                    .shortcut_text(RichText::new(kind.key().to_string()).size(12.5))
                    .min_size(vec2(ui.available_width(), 0.0));
                if ui.add(button).clicked() {
                    out.push(Action::SetKind(kind));
                }
            }
            ui.separator();
            ui.add(egui::Label::new(RichText::new("Цвет").size(12.0).color(theme::muted())).selectable(false));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
                let swatches = std::iter::once(None).chain(card::Tint::ALL.into_iter().map(Some));
                for tint in swatches {
                    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                    let color = tint.map_or(card.kind.accent(), card::Tint::color);
                    let p = ui.painter();
                    if tint.is_none() {
                        // "By kind": a ring in the kind's color.
                        p.circle_stroke(rect.center(), 6.0, Stroke::new(2.0, color));
                    } else {
                        p.circle_filled(rect.center(), 7.0, color);
                    }
                    if tint == card.tint {
                        p.circle_stroke(rect.center(), 9.5, Stroke::new(1.5, theme::text()));
                    } else if resp.hovered() {
                        p.circle_stroke(rect.center(), 9.5, Stroke::new(1.0, theme::muted()));
                    }
                    let label = tint.map_or("По типу", card::Tint::label);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
                    if resp.on_hover_text(label).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        out.push(Action::SetTint(tint));
                    }
                }
            });
        });
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
    revealed_text: Option<&str>,
    active: bool,
    magnet: Option<&[(i64, Rect)]>,
    style: card::CardStyle,
) -> Vec<Action> {
    let mut out = Vec::new();
    let id = Id::new(("card", card.id));
    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);

    // Registration order = hit-test priority: later widgets sit on top.
    // The background senses drags too, though it doesn't move the card: egui
    // hands a press to the topmost click widget and, separately, the topmost
    // drag widget, so a click-only background let the header or edge of a card
    // underneath catch the drag and come to the front.
    let bg = ui.interact(rect, id.with("bg"), Sense::click_and_drag());
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
    // One click edits; a Private card takes a double click, so a stray click
    // doesn't lay its text open.
    // Pushed once the body is drawn: a click on a link there opens it instead.
    let edit_gesture = if card.kind == Kind::Private { bg.double_clicked() } else { bg.clicked() };
    let start_edit = edit_gesture && editing.is_none();

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
    card_panel(ui, rect, hovered, style);
    if let Some(s) = &snapped {
        // Above every card, not just the ones painted before this one.
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, id.with("guides")));
        let stroke = Stroke::new(1.0, card.accent().gamma_multiply(0.8));
        for [a, b] in &s.guides {
            painter.line_segment([origin + a.to_vec2(), origin + b.to_vec2()], stroke);
        }
    }

    let inner = rect.shrink2(vec2(14.0, 8.0));
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), HEADER_H - 4.0));
    let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - FOOTER_H), inner.max);
    let body = Rect::from_min_max(pos2(inner.left(), header.bottom() + 2.0), pos2(inner.right(), footer.top() - 2.0));
    let accent = theme::on_card(card.accent());

    paint_marker(ui, rect, accent, style, hovered);

    // Tags and age only while the card is hovered, edited or selected; at rest a
    // card is its text, its color marker (a setting) and the kind's icon.
    let details = ui.ctx().animate_bool_with_time(id.with("details"), hovered || active || editing.is_some(), 0.15);

    // Header: hover actions.
    ui.scope_builder(
        UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if hovered {
                    // Pinned: nothing that takes the card off the layer by accident.
                    if !locked && icon_button(ui, "\u{E74D}", "Удалить", theme::card_muted()).clicked() {
                        out.push(Action::Delete);
                    }
                    if !locked && icon_button(ui, "\u{E7B8}", "В архив", theme::card_dim()).clicked() {
                        out.push(Action::Archive);
                    }
                    if icon_button(ui, "\u{E8C8}", "Дублировать", theme::card_dim()).clicked() {
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
                        None => ("\u{E77F}", "Копировать текст", theme::card_dim()),
                    };
                    if icon_button(ui, glyph, tip, color).clicked() {
                        ui.data_mut(|d| d.insert_temp(copied_id, Instant::now()));
                        out.push(Action::Copy);
                    }
                    if card.kind == Kind::Private
                        && icon_button(ui, "\u{E890}", "Показать на 5 секунд", theme::card_dim()).clicked()
                    {
                        out.push(Action::Reveal);
                    }
                }
                if card.pinned || hovered {
                    let (glyph, color) = if card.pinned { ("\u{E841}", accent) } else { ("\u{E718}", theme::card_dim()) };
                    if icon_button(ui, glyph, if card.pinned { "Открепить" } else { "Закрепить на месте" }, color).clicked() {
                        out.push(Action::TogglePin);
                    }
                }
            });
        },
    );

    // Body: scrolls when the text doesn't fit; the floating bar shows only on hover.
    let link_clicked = ui
        .scope_builder(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)), |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .auto_shrink([false, false])
                .max_height(body.height())
                .show(ui, |ui| card_body(ui, card, editing, revealed, revealed_text, style))
                .inner
        })
        .inner;
    if start_edit && !link_clicked {
        out.push(Action::StartEdit);
    }

    // Footer: tags + age, and the kind's icon in the corner, which opens the menu
    // to change the kind.
    let icon_size = if style.bold_icon { vec2(26.0, 24.0) } else { vec2(22.0, 20.0) };
    let kind_rect = match style.icon {
        card::IconSpot::TopLeft => Rect::from_center_size(pos2(header.left() + 8.0, header.center().y), icon_size),
        _ => Rect::from_center_size(pos2(footer.right() - 8.0, footer.center().y), icon_size),
    };
    let icon_shown = if style.icon == card::IconSpot::Hover { details } else { 1.0 };
    let sense = if icon_shown > 0.5 { Sense::click() } else { Sense::hover() };
    let kind = ui.interact(kind_rect, id.with("kind"), sense);
    kind.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Тип: {}", card.kind.label())));
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&kind));
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(icon_shown);
    if kind.hovered() && icon_shown > 0.5 || menu_open {
        painter.rect_filled(kind_rect, CornerRadius::same(6), theme::wash(22));
    }
    if style.bold_icon {
        // Segoe Fluent Icons has no bold weight: the glyph drawn a few times, a
        // fraction of a point apart, thickens its strokes.
        for d in [vec2(-0.4, 0.0), vec2(0.4, 0.0), vec2(0.0, -0.4), vec2(0.0, 0.4), Vec2::ZERO] {
            painter.text(kind_rect.center() + d, Align2::CENTER_CENTER, card.kind.icon(), theme::icons(16.0), accent);
        }
    } else {
        painter.text(kind_rect.center(), Align2::CENTER_CENTER, card.kind.icon(), theme::icons(12.5), accent);
    }
    if kind.clicked() {
        out.push(Action::Front);
    }
    let kind = if icon_shown > 0.5 {
        kind.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!("{} — сменить тип", card.kind.label()))
    } else {
        kind
    };
    kind_menu(&kind, card, &mut out, style.icon == card::IconSpot::TopLeft);

    let mut painter = ui.painter().with_clip_rect(footer);
    painter.multiply_opacity(details);
    let age = painter.text(
        pos2(if style.icon == card::IconSpot::TopLeft { footer.right() } else { kind_rect.left() - 6.0 }, footer.center().y),
        Align2::RIGHT_CENTER,
        age_label(card.created_at),
        FontId::proportional(11.5),
        theme::card_muted(),
    );
    let mut x = footer.left();
    for tag in &card.tags {
        let galley = painter.layout_no_wrap(format!("#{tag}"), FontId::proportional(11.5), theme::card_dim());
        let chip = Rect::from_min_size(
            pos2(x, footer.center().y - galley.size().y / 2.0 - 2.0),
            galley.size() + vec2(12.0, 4.0),
        );
        if chip.right() > age.left() - 8.0 {
            break;
        }
        painter.rect_filled(chip, CornerRadius::same(6), theme::chip_fill());
        painter.galley(chip.min + vec2(6.0, 2.0), galley, theme::card_dim());
        x = chip.right() + 4.0;
    }

    if hovered {
        let p = ui.painter();
        let c = rect.max - vec2(6.0, 6.0);
        let stroke = Stroke::new(1.2, theme::wash(if corner_hovered { 120 } else { 50 }));
        p.line_segment([c - vec2(8.0, 0.0), c - vec2(0.0, 8.0)], stroke);
        p.line_segment([c - vec2(4.0, 0.0), c - vec2(0.0, 4.0)], stroke);
    }

    out
}
