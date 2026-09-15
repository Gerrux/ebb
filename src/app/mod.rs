//! The ambient layer (root viewport); the capture/search bar lives in `bar`.
//!
//! `EbbApp` and its event loop are here; its parts live in the submodules:
//! `layer` (window, curtain, menu), `cards` (the cards on the layer and what
//! their actions do), `toast`, and the widgets they draw with (`card_ui`,
//! `card_text`, `paint`).

mod card_text;
mod card_ui;
mod cards;
mod layer;
mod paint;
mod toast;

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use egui::{
    CornerRadius, Key, Modifiers, Pos2, Rect, Ui, UiBuilder, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId, pos2, vec2,
};

use std::sync::atomic::Ordering;

use crate::bar::{self, BarState, Mode, Outbox, Press};
use crate::import_ui::StickyImport;
use crate::library::{self, LibraryState, Request, Tab};
use crate::resurface;
use crate::shell::{self, Event};
use crate::card::{self, Card, MIN_SIZE, Placement};
use crate::store::Store;
use crate::theme;
use crate::win::{self, Backdrop};

pub(crate) use paint::{PANEL_APPEAR, glass_panel, paint_marker, panel_ease};

use cards::{PromptFill, TagInput};
use layer::{CURTAIN_DROP, TAB_SIZE, place_tab};
use toast::Toast;

const LAYER_FADE_IN: Duration = Duration::from_millis(220);

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
const SET_HIDE_FROM_CAPTURE: &str = "private.hide_from_capture";
const SET_CARD_STYLE: &str = "cards.style";
const SET_THEME: &str = "app.theme";

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
    /// A shown Private value keeps the layer out of screenshots and recordings.
    hide_from_capture: bool,
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
    /// The layer window is currently kept out of screen capture (while revealing).
    capture_excluded: bool,
    /// Layer focus last frame: a reveal ends when the layer loses focus.
    was_focused: Option<bool>,
    /// Card just opened from search: outlined for a moment.
    highlighted: Option<(i64, Instant)>,
    toast: Option<Toast>,
    /// Copying a Prompt with `{{variables}}`: the values asked for before it's copied.
    prompt_fill: Option<PromptFill>,
    tag_input: Option<TagInput>,
    /// Tags by use, for suggestions; read when the tag field opens, dropped when text changes.
    tag_counts: Option<Vec<(String, i64)>>,
    /// Cards that just arrived on the layer / just left it, for their animations.
    appearing: Vec<(i64, Instant)>,
    leaving: Vec<(Card, Instant)>,

    sticky: StickyImport,

    show_debug: bool,
    autostarted: bool,
    main_started: Instant,
    first_frame: Option<(f64, f64)>,
    frames: u64,
    /// When the next morning's Rediscover pick is due (unix seconds).
    next_rediscover: i64,
    /// The pick running on its own thread: the cards it brought back, or None
    /// when there was nothing to do (already made today, or it failed).
    rediscover_job: Option<mpsc::Receiver<Option<Vec<i64>>>>,
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
        let hide_from_capture = store.setting(SET_HIDE_FROM_CAPTURE).is_none_or(|v| v != "0");
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

        // Brought back from the archive today, appearing like a new card.
        let appearing: Vec<(i64, Instant)> = fresh.iter().map(|id| (*id, Instant::now())).collect();
        place_resurfaced(&store, &mut cards, &fresh, full_area);

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
            hide_from_capture,
            card_style,
            theme_mode,
            manual_start,
            library: Arc::default(),
            editing: None,
            active: None,
            revealed: None,
            revealed_text: None,
            capture_excluded: false,
            was_focused: None,
            highlighted: None,
            toast: None,
            prompt_fill: None,
            tag_input: None,
            tag_counts: None,
            appearing,
            leaving: Vec::new(),
            sticky: StickyImport::default(),
            show_debug: false,
            autostarted,
            main_started,
            first_frame: None,
            frames: 0,
            next_rediscover: resurface::next_day_start(resurface::unix_now(), crate::search::local_offset_secs()),
            rediscover_job: None,
        }
    }

    /// The morning Rediscover pick while Ebb keeps running: a timer to the start
    /// of the next day, the pick itself on a background thread with its own
    /// connection, and the cards it changed brought onto the layer when it's done.
    fn schedule_rediscover(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.rediscover_job {
            match job.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                Ok(Some(fresh)) => self.bring_resurfaced(&fresh),
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {}
            }
            self.rediscover_job = None;
        }
        let (now, offset) = (resurface::unix_now(), crate::search::local_offset_secs());
        if now < self.next_rediscover {
            ctx.request_repaint_after(Duration::from_secs((self.next_rediscover - now) as u64));
            return;
        }
        // Not from under someone typing: once the editor closes.
        if self.editing.is_some() {
            ctx.request_repaint_after(Duration::from_secs(60));
            return;
        }
        self.next_rediscover = resurface::next_day_start(now, offset);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("rediscover".into()).spawn(move || {
            crate::import_ui::background_priority();
            let result = Store::open().and_then(|store| {
                if store.resurfaced_today(now, offset) {
                    return Ok(None);
                }
                store.refresh_resurfacing(now, offset, resurface::REDISCOVER_LIMIT).map(Some)
            });
            let fresh = result.unwrap_or_else(|e| {
                eprintln!("resurfacing failed: {e}");
                None
            });
            let _ = tx.send(fresh.map(|picks| picks.into_iter().map(|p| p.id).collect()));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.rediscover_job = Some(rx);
        }
    }

    /// After a Rediscover pick: yesterday's unanswered cards leave, today's come in.
    fn bring_resurfaced(&mut self, fresh: &[i64]) {
        self.reload_cards();
        place_resurfaced(&self.store, &mut self.cards, fresh, self.full_area);
        self.library.lock().unwrap().invalidate();
        self.bar.lock().unwrap().invalidate();
    }

    fn area(&self, ui: &Ui) -> Vec2 {
        ui.max_rect().size()
    }

    fn add_from_capture(&mut self, text: String, plain: bool, area: Vec2) {
        let parsed = card::parse_capture_with(&text, !plain);
        if parsed.body.is_empty() && parsed.title.is_empty() {
            return;
        }
        let pos = card::free_slot(&self.cards, area);
        let inserted = self.store.insert(&parsed, pos);
        match self.report(inserted, "сохранить заметку, текст остался в окне захвата") {
            Some(c) => {
                self.tag_counts = None;
                self.appearing.push((c.id, Instant::now()));
                self.cards.push(c);
            }
            None => self.bar.lock().unwrap().restore_capture(text, plain),
        }
    }

    /// A write that didn't reach the database is said on screen: release builds
    /// have no console for the error to go to.
    fn report<T>(&mut self, result: rusqlite::Result<T>, what: &str) -> Option<T> {
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("{what}: {e}");
                self.toast = Some(Toast::new(format!("Не удалось {what}"), None));
                None
            }
        }
    }

    /// Whether the card's change reached the database.
    fn save(&mut self, idx: usize) -> bool {
        let saved = self.store.save(&self.cards[idx]);
        self.report(saved, "сохранить изменения").is_some()
    }

    fn layer_settings(&self) -> library::LayerSettings {
        library::LayerSettings {
            monitor_device: self.hwnd.and_then(win::monitor_of).and_then(|m| m.device_ids.first().cloned()),
            backdrop: self.backdrop,
            tint: self.tint,
            pin_bottom: win::PIN_BOTTOM.load(Ordering::Relaxed),
            capture_hotkey: self.shell.hotkeys().and_then(|k| k.capture),
            search_hotkey: self.shell.hotkeys().and_then(|k| k.search),
            dismiss_hides: self.dismiss_hides,
            curtain_top: self.curtain_top,
            settings_on_launch: self.settings_on_launch,
            snap: self.snap,
            hide_from_capture: self.hide_from_capture,
            card_style: self.card_style,
            theme_mode: self.theme_mode,
        }
    }

    /// Opens the library window on `tab`.
    fn open_library(&mut self, ctx: &egui::Context, tab: Tab) {
        let mut lib = self.library.lock().unwrap();
        lib.settings = self.layer_settings();
        lib.open(tab);
        ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::InnerSize(lib.size()));
        ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::Focus);
        ctx.request_repaint();
    }

    /// Opens the settings window; `welcome` on the first run.
    fn open_settings(&mut self, ctx: &egui::Context, welcome: bool) {
        // Settings show the hotkeys: a moment to pick up one freed since startup.
        shell::retry_hotkeys(&self.shell);
        let mut lib = self.library.lock().unwrap();
        lib.settings = self.layer_settings();
        lib.open(Tab::Settings);
        lib.welcome |= welcome;
        ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::InnerSize(lib.size()));
        ctx.send_viewport_cmd_to(library::viewport_id(), ViewportCommand::Focus);
        ctx.request_repaint();
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
            Request::SetHideFromCapture(on) => {
                self.hide_from_capture = on;
                self.set_flag(SET_HIDE_FROM_CAPTURE, on);
                ctx.request_repaint();
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
                card.size = card.size.max(MIN_SIZE);
                card.pos = card::free_slot_for(&self.cards, card.size, self.full_area);
                self.appearing.push((card.id, Instant::now()));
                self.leaving.retain(|(old, _)| old.id != card.id);
                self.cards.push(card);
                let idx = self.cards.len() - 1;
                self.save(idx);
                idx
            }
        };
        // Opened from search to be read: a collapsed card opens.
        if self.cards[idx].collapsed && self.store.set_collapsed(id, false).is_ok() {
            self.cards[idx].collapsed = false;
        }
        let card = self.cards.remove(idx);
        self.cards.push(card);
        let _ = self.store.raise(id);
        let _ = self.store.touch(id);
        self.highlighted = Some((id, Instant::now()));
        ctx.request_repaint();
    }

    fn reload_cards(&mut self) {
        self.commit_edit();
        self.tag_counts = None;
        if let Ok(cards) = self.store.load() {
            let now = Instant::now();
            // Cards that weren't on the layer before appear; ones that left fade out.
            for c in cards.iter().filter(|c| !self.cards.iter().any(|old| old.id == c.id)) {
                self.appearing.push((c.id, now));
            }
            let gone = self.cards.drain(..).filter(|old| !cards.iter().any(|c| c.id == old.id));
            self.leaving.extend(gone.map(|c| (c, now)));
            // Back before its leave animation ended (a quick Undo): drawn once, not twice.
            self.leaving.retain(|(old, _)| !cards.iter().any(|c| c.id == old.id));
            self.cards = cards;
            // A card back from the archive (pinned in the review, say) keeps its old
            // spot if it's still on the layer; an imported one never had one.
            let layer = Rect::from_min_size(Pos2::ZERO, self.full_area);
            for idx in 0..self.cards.len() {
                let c = &self.cards[idx];
                let fresh = self.appearing.iter().any(|(id, t)| *id == c.id && *t == now);
                if fresh && (c.pos == Pos2::ZERO || !layer.contains_rect(Rect::from_min_size(c.pos, c.size))) {
                    self.cards[idx].pos = pos2(-1.0e5, -1.0e5);
                    self.cards[idx].pos = card::free_slot_for(&self.cards, self.cards[idx].size, self.full_area);
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
                Event::HotkeysChanged => {
                    let keys = self.shell.hotkeys().unwrap_or_default();
                    let mut lib = self.library.lock().unwrap();
                    (lib.settings.capture_hotkey, lib.settings.search_hotkey) = (keys.capture, keys.search);
                    ctx.request_repaint_of(library::viewport_id());
                }
                // Slept through the morning, or the clock moved: look again now
                // (a pick already made today is left as it is).
                Event::ClockChanged => self.next_rediscover = 0,
                Event::TogglePinBottom => self.set_pin_bottom(!win::PIN_BOTTOM.load(Ordering::Relaxed)),
                Event::Exit => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
                Event::ImportSticky => self.apply_library_request(ctx, Request::ImportSticky),
                Event::OpenLibrary(true) => self.open_settings(ctx, false),
                event @ (Event::OpenLibrary(false) | Event::OpenReview) => {
                    self.open_library(ctx, if matches!(event, Event::OpenReview) { Tab::Review } else { Tab::Archive });
                }
            }
        }

        self.schedule_rediscover(ctx);

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
                Outbox::Captured(text, plain) => {
                    self.add_from_capture(text, plain, self.full_area);
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
                if self.sticky.ui(ui, &mut self.store, &mut self.cards, self.hwnd) {
                    self.open_library(ui.ctx(), Tab::Import);
                }
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

/// Cards brought back from the archive by Rediscover go into free slots, not
/// where each was when it was archived, and above the cards already there.
fn place_resurfaced(store: &Store, cards: &mut [Card], fresh: &[i64], area: Vec2) {
    if fresh.is_empty() {
        return;
    }
    let away = pos2(-1.0e5, -1.0e5);
    cards.iter_mut().filter(|c| fresh.contains(&c.id)).for_each(|c| c.pos = away);
    for id in fresh {
        let Some(idx) = cards.iter().position(|c| c.id == *id) else { continue };
        cards[idx].size = cards[idx].size.max(MIN_SIZE);
        cards[idx].pos = card::free_slot_for(cards, cards[idx].size, area);
        if let Err(e) = store.save(&cards[idx]) {
            eprintln!("save failed: {e}");
        }
    }
    // On a full layer they land on other cards: above them, not under.
    cards.sort_by_key(|c| fresh.contains(&c.id));
    for id in fresh {
        let _ = store.raise(*id);
    }
}
