//! The ambient layer (root viewport) and the quick-capture bar (deferred viewport).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Layout, Modifiers, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId, pos2, vec2,
};

use std::sync::atomic::Ordering;

use crate::import_ui::StickyImport;
use crate::shell::{self, Event};
use crate::autostart;
use crate::card::{self, Card, Kind, MIN_SIZE, parse_capture};
use crate::store::Store;
use crate::theme::{self, TEXT, TEXT_DIM, TEXT_MUTED};
use crate::win::{self, Backdrop};

const CAPTURE_SIZE: Vec2 = vec2(640.0, 132.0);
const HEADER_H: f32 = 30.0;
const FOOTER_H: f32 = 22.0;
const REVEAL_FOR: Duration = Duration::from_secs(5);

fn capture_id() -> ViewportId {
    ViewportId::from_hash_of("capture")
}

/// State shared with the capture viewport, which renders on its own callback.
#[derive(Default)]
struct CaptureState {
    visible: bool,
    text: String,
    /// When the hotkey was pressed for the current showing.
    pressed_at: Option<Instant>,
    request_focus: bool,
    latency_ms: Option<f64>,
    submitted: Vec<String>,
    hwnd: Option<isize>,
}

#[derive(Default)]
enum AutostartUi {
    #[default]
    Unknown,
    Busy,
    Known(Result<autostart::Status, String>),
}

/// Runs a Task Scheduler call off the UI thread, then refreshes the shown state.
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

enum Action {
    Front,
    Moved,
    TogglePin,
    Copy,
    Archive,
    Delete,
    StartEdit,
    Reveal,
}

pub struct AmbientApp {
    store: Store,
    cards: Vec<Card>,
    capture: Arc<Mutex<CaptureState>>,
    shell: Arc<shell::Shared>,

    hwnd: Option<isize>,
    layer_visible: bool,
    backdrop: Backdrop,
    monitor: usize,
    tint: u8,

    editing: Option<(i64, String)>,
    revealed: Option<(i64, Instant)>,

    sticky: StickyImport,

    show_debug: bool,
    /// Autostart task state, filled in by a background query when the debug panel opens.
    autostart: Arc<Mutex<AutostartUi>>,
    autostarted: bool,
    main_started: Instant,
    first_frame: Option<(f64, f64)>,
    frames: u64,
}

impl AmbientApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Store,
        cards: Vec<Card>,
        main_started: Instant,
        autostarted: bool,
    ) -> Self {
        theme::install(&cc.egui_ctx);

        let hwnd = win::hwnd_of(cc);
        // DWM acrylic turns flat grey when the window is inactive; the accent one stays blurred.
        let backdrop = Backdrop::AccentAcrylic;
        if let Some(h) = hwnd {
            if let Some(m) = win::monitors().first() {
                win::place_on(h, m);
            }
            win::apply_backdrop(h, backdrop);
            // Still hidden here (eframe shows it after the first frame), so the
            // taskbar never sees a button.
            win::install_window_rules(h, win::LAYER);
        }

        let shell = Arc::new(shell::Shared::default());
        shell.layer_visible.store(true, Ordering::Relaxed);
        shell.pin_bottom.store(win::PIN_BOTTOM.load(Ordering::Relaxed), Ordering::Relaxed);
        shell::spawn(cc.egui_ctx.clone(), shell.clone());

        Self {
            store,
            cards,
            capture: Arc::default(),
            shell,
            hwnd,
            layer_visible: true,
            backdrop,
            monitor: 0,
            tint: 70,
            editing: None,
            revealed: None,
            sticky: StickyImport::default(),
            show_debug: false,
            autostart: Arc::default(),
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
            Ok(c) => self.cards.push(c),
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
        match buf.split_once('\n') {
            Some((t, b)) => {
                c.title = t.trim().to_owned();
                c.body = b.trim().to_owned();
            }
            None => {
                c.title.clear();
                c.body = buf.to_owned();
            }
        }
        c.tags = buf
            .split_whitespace()
            .filter_map(|w| w.strip_prefix('#'))
            .map(|t| t.trim_end_matches([',', '.', ';']).to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        self.save(idx);
    }

    fn cycle_monitor(&mut self) {
        let mons = win::monitors();
        if mons.is_empty() {
            return;
        }
        self.monitor = (self.monitor + 1) % mons.len();
        if let Some(h) = self.hwnd {
            win::place_on(h, &mons[self.monitor]);
        }
    }

    // -----------------------------------------------------------------------

    fn header(&self, ui: &mut Ui) {
        let origin = ui.max_rect().min + vec2(32.0, 22.0);
        let painter = ui.painter();
        let title = painter.text(origin, Align2::LEFT_TOP, "Ambient", theme::semibold(22.0), TEXT);
        painter.text(
            pos2(title.right() + 14.0, title.bottom() - 3.0),
            Align2::LEFT_BOTTOM,
            format!(
                "{} карточек  ·  {} — записать  ·  F1 — debug",
                self.cards.len(),
                match self.shell.hotkey_label.get() {
                    Some(Some(label)) => label,
                    Some(None) => "хоткей занят",
                    None => "…",
                }
            ),
            FontId::proportional(13.0),
            TEXT_MUTED,
        );
    }

    fn cards_ui(&mut self, ui: &mut Ui) {
        let origin = ui.max_rect().min;
        let pointer = ui.input(|i| i.pointer.hover_pos());
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

        let mut actions: Vec<(usize, Action)> = Vec::new();
        for idx in 0..self.cards.len() {
            let id = self.cards[idx].id;
            let editing = self.editing.as_mut().filter(|(eid, _)| *eid == id).map(|(_, b)| b);
            let revealed = self.revealed.is_some_and(|(rid, _)| rid == id);
            for a in card_ui(ui, origin, area, &mut self.cards[idx], hovered_id == Some(id), editing, revealed) {
                actions.push((idx, a));
            }
        }

        // Clicking empty space ends editing.
        if self.editing.is_some() && hovered_id.is_none() && ui.input(|i| i.pointer.any_pressed()) {
            self.commit_edit();
        }

        let mut to_front = None;
        let mut remove = None;
        for (idx, action) in actions {
            match action {
                Action::Front => to_front = Some(idx),
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
                Action::Archive => {
                    self.cards[idx].archived = true;
                    self.save(idx);
                    remove = Some(idx);
                }
                Action::Delete => {
                    let _ = self.store.delete(self.cards[idx].id);
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
            self.cards.remove(idx);
        } else if let Some(idx) = to_front {
            if idx + 1 != self.cards.len() {
                let c = self.cards.remove(idx);
                self.cards.push(c);
            }
        }
    }

    fn debug_ui(&mut self, ui: &mut Ui) {
        let rect = Rect::from_min_size(ui.max_rect().right_bottom() - vec2(420.0, 300.0), vec2(396.0, 276.0));
        glass_panel(ui, rect, false);
        let mut backdrop_clicked = false;
        let mut monitor_clicked = false;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(16.0)), |ui| {
            ui.label(RichText::new("Debug").font(theme::semibold(15.0)).color(TEXT));
            ui.add_space(4.0);
            let (proc_ms, main_ms) = self.first_frame.unwrap_or((f64::NAN, f64::NAN));
            let (ws, private) = win::memory_mib();
            let latency = self.capture.lock().unwrap().latency_ms;
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
            ui.horizontal(|ui| {
                ui.label(RichText::new("Тонировка").size(13.0).color(TEXT_MUTED));
                ui.add(egui::Slider::new(&mut self.tint, 0..=220).show_value(false));
            });
            ui.horizontal(|ui| {
                backdrop_clicked = ui.button(format!("F2: {}", self.backdrop.label())).clicked();
            });
            monitor_clicked = ui.button(format!("F3: монитор {}", self.monitor)).clicked();
            self.autostart_ui(ui);
        });
        if backdrop_clicked {
            self.cycle_backdrop();
        }
        if monitor_clicked {
            self.cycle_monitor();
        }
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }

    fn autostart_ui(&self, ui: &mut Ui) {
        let mut state = self.autostart.lock().unwrap();
        if matches!(*state, AutostartUi::Unknown) {
            drop(state);
            autostart_job(&self.autostart, ui.ctx(), None);
            state = self.autostart.lock().unwrap();
        }
        // (checked, short note, full detail on hover)
        let (mut on, note, detail) = match &*state {
            AutostartUi::Unknown | AutostartUi::Busy => (false, Some("…"), None),
            AutostartUi::Known(Ok(autostart::Status::Off)) => (false, None, None),
            AutostartUi::Known(Ok(autostart::Status::On { current_exe: true, .. })) => (true, None, None),
            AutostartUi::Known(Ok(autostart::Status::On { command, .. })) => {
                (true, Some("другой exe"), Some(command.clone()))
            }
            AutostartUi::Known(Err(e)) => (false, Some("ошибка"), Some(e.clone())),
        };
        let busy = matches!(*state, AutostartUi::Busy | AutostartUi::Unknown);
        drop(state);
        ui.horizontal(|ui| {
            let resp = ui.add_enabled(!busy, egui::Checkbox::new(&mut on, "Запускать при входе в Windows"));
            if resp.changed() {
                autostart_job(&self.autostart, ui.ctx(), Some(on));
            }
            if let Some(note) = note {
                let label = ui.label(RichText::new(note).size(11.5).color(TEXT_MUTED));
                if let Some(detail) = detail {
                    label.on_hover_text(detail);
                }
            }
        });
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
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(visible));
        if !visible {
            self.commit_edit();
            // Nothing is drawn while hidden; give the pages back until it's shown again.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(500));
                win::trim_working_set();
            });
        }
    }

    fn cycle_backdrop(&mut self) {
        self.backdrop = self.backdrop.next();
        if let Some(h) = self.hwnd {
            win::apply_backdrop(h, self.backdrop);
        }
    }

    fn capture_viewport(&self, ui: &Ui) {
        let visible = self.capture.lock().unwrap().visible;
        let state = self.capture.clone();
        ui.ctx().show_viewport_deferred(
            capture_id(),
            ViewportBuilder::default()
                .with_title("Ambient Capture")
                .with_inner_size(CAPTURE_SIZE)
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_close_button(false)
                .with_minimize_button(false)
                .with_maximize_button(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_visible(visible),
            move |ui, _class| capture_ui(ui, &state),
        );
    }
}

impl eframe::App for AmbientApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Runs even while the layer is hidden, so tray and hotkey events work then too.
        let mut pressed = None;
        for event in self.shell.take_events() {
            match event {
                Event::Capture(t) => pressed = Some(t),
                Event::ToggleLayer => self.set_layer_visible(ctx, !self.layer_visible),
                Event::ShowLayer => self.set_layer_visible(ctx, true),
                Event::TogglePinBottom => {
                    let on = !win::PIN_BOTTOM.load(Ordering::Relaxed);
                    if let Some(h) = self.hwnd {
                        win::set_pin_bottom(h, on);
                    }
                    self.shell.pin_bottom.store(on, Ordering::Relaxed);
                }
                Event::Exit => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
                Event::ImportSticky => {
                    self.set_layer_visible(ctx, true);
                    self.sticky.scan(ctx, &self.store, &self.cards, true);
                }
            }
        }

        let mut cap = self.capture.lock().unwrap();
        if cap.hwnd.is_none() {
            cap.hwnd = win::find_capture_window();
            if let Some(h) = cap.hwnd {
                win::apply_backdrop(h, Backdrop::AccentAcrylic);
                win::install_window_rules(h, 0);
            }
        }
        if let Some(t) = pressed {
            if cap.visible {
                cap.visible = false;
                ctx.send_viewport_cmd_to(capture_id(), ViewportCommand::Visible(false));
            } else {
                if let Some(h) = cap.hwnd {
                    win::move_near_cursor(h, (CAPTURE_SIZE.x * win::dpi_scale(h)) as i32);
                }
                cap.visible = true;
                cap.pressed_at = Some(t);
                cap.latency_ms = None;
                cap.request_focus = true;
                // Explicit command: with the layer hidden the root ui (which carries
                // the viewport builder) doesn't run until something is visible.
                ctx.send_viewport_cmd_to(capture_id(), ViewportCommand::Visible(true));
                ctx.send_viewport_cmd_to(capture_id(), ViewportCommand::Focus);
            }
        }
        let submitted = std::mem::take(&mut cap.submitted);
        drop(cap);

        if !submitted.is_empty() {
            let area = ctx.content_rect().size();
            for text in submitted {
                self.add_from_capture(&text, area);
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if self.first_frame.is_none() {
            let frame_at = win::now_filetime();
            let proc_ms = win::ms_since_process_start();
            let main_ms = self.main_started.elapsed().as_secs_f64() * 1000.0;
            self.first_frame = Some((proc_ms, main_ms));
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
                self.sticky.maybe_offer(ui.ctx(), &self.store, &self.cards);
            }
            if let Some(out) = crate::bench::path() {
                let (shell, ctx) = (self.shell.clone(), ui.ctx().clone());
                let capture = self.capture.clone();
                let hooks = crate::bench::Hooks {
                    trigger: Some(Box::new(move || {
                        shell.events.lock().unwrap().push(Event::Capture(Instant::now()));
                        ctx.request_repaint();
                    })),
                    latency_ms: Box::new(move || capture.lock().unwrap().latency_ms),
                };
                let label = format!("ambient-{}", crate::renderer::NAME);
                crate::bench::start(ui.ctx().clone(), out, label, proc_ms, main_ms, hooks);
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
            self.commit_edit();
        }

        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(self.tint));

        self.header(ui);
        self.cards_ui(ui);
        let area = self.area(ui);
        self.sticky.ui(ui, &mut self.store, &mut self.cards, area);
        if self.show_debug {
            self.debug_ui(ui);
        }
        // Created up front even while hidden: creating it on the first hotkey press
        // saves ~4 MiB but doubles the first-show latency and flashes without acrylic.
        self.capture_viewport(ui);
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

/// Appends one line to %LOCALAPPDATA%\Ambient\timing.log. Runs off the UI thread:
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

fn card_ui(
    ui: &mut Ui,
    origin: Pos2,
    area: Vec2,
    card: &mut Card,
    hovered: bool,
    editing: Option<&mut String>,
    revealed: bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    let id = Id::new(("card", card.id));
    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);

    // Registration order = hit-test priority: later widgets sit on top.
    let bg = ui.interact(rect, id.with("bg"), Sense::click());
    let header_rect = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H + 6.0));
    let drag = ui.interact(header_rect, id.with("drag"), Sense::drag());
    let grip_rect = Rect::from_min_max(rect.max - vec2(18.0, 18.0), rect.max);
    let grip = ui.interact(grip_rect, id.with("grip"), Sense::drag());

    if bg.clicked() || bg.double_clicked() || drag.drag_started() || grip.drag_started() {
        out.push(Action::Front);
    }
    if bg.double_clicked() && editing.is_none() && card.kind != Kind::Private {
        out.push(Action::StartEdit);
    }

    if drag.dragged() {
        card.pos += drag.drag_delta();
        card.pos.x = card.pos.x.clamp(0.0, (area.x - 80.0).max(0.0));
        card.pos.y = card.pos.y.clamp(0.0, (area.y - HEADER_H).max(0.0));
    }
    if grip.dragged() {
        card.size = (card.size + grip.drag_delta()).max(MIN_SIZE);
    }
    if drag.drag_stopped() || grip.drag_stopped() {
        card.pos = (card.pos.to_vec2() / 8.0).round().to_pos2() * 8.0;
        card.size = ((card.size / 8.0).round() * 8.0).max(MIN_SIZE);
        out.push(Action::Moved);
    }
    if drag.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if drag.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if grip.hovered() || grip.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
    }

    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);
    glass_panel(ui, rect, hovered);

    let inner = rect.shrink2(vec2(14.0, 8.0));
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), HEADER_H - 4.0));
    let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - FOOTER_H), inner.max);
    let body = Rect::from_min_max(pos2(inner.left(), header.bottom() + 2.0), pos2(inner.right(), footer.top() - 2.0));
    let accent = card.kind.accent();

    // Header: kind + hover actions.
    ui.scope_builder(
        UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.label(RichText::new(card.kind.icon()).font(theme::icons(12.0)).color(accent));
            ui.add(egui::Label::new(RichText::new(card.kind.label()).size(12.0).color(TEXT_MUTED)).selectable(false));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if hovered {
                    if icon_button(ui, "\u{E74D}", "Удалить", TEXT_MUTED).clicked() {
                        out.push(Action::Delete);
                    }
                    if icon_button(ui, "\u{E7B8}", "В архив", TEXT_DIM).clicked() {
                        out.push(Action::Archive);
                    }
                    if icon_button(ui, "\u{E8C8}", "Копировать", TEXT_DIM).clicked() {
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
                    if icon_button(ui, glyph, if card.pinned { "Открепить" } else { "Закрепить" }, color).clicked() {
                        out.push(Action::TogglePin);
                    }
                }
            });
        },
    );

    // Body.
    ui.scope_builder(
        UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
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
            if !card.title.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&card.title).font(theme::semibold(15.0)).color(TEXT))
                        .wrap()
                        .selectable(false),
                );
            }
            let text = if card.kind == Kind::Private && !revealed {
                "••••••••••".to_owned()
            } else {
                without_tags(&card.body)
            };
            let color = if card.title.is_empty() { TEXT } else { TEXT_DIM };
            let size = if card.title.is_empty() { 14.5 } else { 13.5 };
            ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap().selectable(false));
        },
    );

    // Footer: tags + age.
    let painter = ui.painter().with_clip_rect(footer);
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
        let stroke = Stroke::new(1.2, Color32::from_white_alpha(if grip.hovered() { 120 } else { 50 }));
        p.line_segment([c - vec2(8.0, 0.0), c - vec2(0.0, 8.0)], stroke);
        p.line_segment([c - vec2(4.0, 0.0), c - vec2(0.0, 4.0)], stroke);
    }

    out
}

fn capture_ui(ui: &mut Ui, state: &Mutex<CaptureState>) {
    let mut st = state.lock().unwrap();
    if !st.visible {
        return;
    }
    if st.latency_ms.is_none() {
        if let Some(t) = st.pressed_at {
            st.latency_ms = Some(t.elapsed().as_secs_f64() * 1000.0);
        }
    }

    let (submit, cancel) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    let lost_focus = ui.input(|i| i.viewport().focused == Some(false))
        && st.pressed_at.is_some_and(|t| t.elapsed() > Duration::from_millis(400));

    let rect = ui.max_rect();
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        Color32::from_rgba_unmultiplied(22, 24, 30, 140),
        Stroke::new(1.0, theme::glass_stroke()),
        StrokeKind::Inside,
    );

    let parsed = parse_capture(&st.text);
    let inner = rect.shrink2(vec2(18.0, 14.0));
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
                        .text_color(TEXT)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2),
                ),
            );
        });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(parsed.kind.icon()).font(theme::icons(11.0)).color(parsed.kind.accent()));
                ui.label(RichText::new(parsed.kind.label()).size(12.0).color(TEXT_DIM));
                for tag in &parsed.tags {
                    ui.label(RichText::new(format!("#{tag}")).size(12.0).color(TEXT_MUTED));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hint = match st.latency_ms {
                        Some(l) => format!("Enter — сохранить · Shift+Enter — строка · Esc   ·   {l:.0} мс"),
                        None => "Enter — сохранить · Shift+Enter — строка · Esc".into(),
                    };
                    ui.label(RichText::new(hint).size(11.5).color(TEXT_MUTED));
                });
            });
        });
    });

    if st.request_focus {
        if let Some(r) = &edit_resp {
            r.request_focus();
        }
        st.request_focus = false;
    }

    let mut changed = false;
    if submit && !st.text.trim().is_empty() {
        let text = std::mem::take(&mut st.text);
        st.submitted.push(text);
        st.visible = false;
        changed = true;
    } else if cancel || lost_focus {
        st.visible = false;
        changed = true;
    }
    if changed {
        ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    }
}
