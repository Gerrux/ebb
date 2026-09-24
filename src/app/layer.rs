//! The layer's window: shown, summoned and dismissed, rolled into a tab and
//! back, on which monitor and with which backdrop; its header, menu and debug panel.

use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Pos2, Rect, RichText,
    Sense, Stroke, Ui, UiBuilder, Vec2, ViewportCommand, ViewportId, pos2, vec2,
};

use std::sync::atomic::Ordering;

use crate::card;
use crate::library::Tab;
use crate::shell::Event;
use crate::theme;
use crate::win::{self, Backdrop};

use super::cards::note_pos_at;
use super::paint::{glass_button, glass_panel, hover_t, panel_ease, paint_logo};
use super::{EbbApp, LAYER_FADE_IN, SET_BACKDROP, SET_COLLAPSED, SET_CURTAIN, SET_MONITOR, SET_PIN_BOTTOM};

const LAYER_FADE_OUT: Duration = Duration::from_millis(160);
/// The curtain: the layer slides down and fades before it shrinks to the tab.
const CURTAIN_ANIM: Duration = Duration::from_millis(200);
/// How far the layer slides down as it collapses, and the drag that fully collapses it.
pub(super) const CURTAIN_DROP: f32 = 48.0;
const CURTAIN_DRAG: f32 = 160.0;
/// The collapsed layer: a tab at the top center of the desktop (a notch against
/// the screen's edge) or at the bottom center, above the taskbar.
pub(super) const TAB_SIZE: (f32, f32) = (148.0, 40.0);
const TAB_GAP: f32 = 10.0;

pub(super) fn place_tab(h: isize, m: &win::Monitor, top: bool) {
    win::place_edge_center(h, m, TAB_SIZE, if top { 0.0 } else { TAB_GAP }, top);
}

/// A tray click this soon after another app took the focus from a summoned layer
/// (the taskbar does, on mouse down) counts as a click on the summoned layer.
const TRAY_CLICK_AFTER_LOWER_MS: u64 = 500;

/// Bands of the layer where a double-click makes no note: the header with the
/// "+"/"⋯" buttons (and the top handle), the bottom handle, a toast over it.
const HEADER_BAND: f32 = 64.0;
const HANDLE_BAND: f32 = 40.0;
const TOAST_BAND: f32 = 100.0;

#[derive(Clone, Copy)]
enum MenuItem {
    NewNote,
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

impl EbbApp {
    pub(super) fn cycle_monitor(&mut self) {
        let monitors = win::monitors();
        let current = self.hwnd.and_then(win::monitor_of).map(|m| m.handle);
        let idx = monitors.iter().position(|m| Some(m.handle) == current).map_or(0, |i| (i + 1) % monitors.len());
        if let Some(device) = monitors.get(idx).and_then(|m| m.device_ids.first()).cloned() {
            self.set_monitor(&device);
        }
    }

    pub(super) fn header(&self, ui: &mut Ui) {
        let origin = ui.max_rect().min + vec2(32.0, 20.0);
        let painter = ui.painter();
        let logo = Rect::from_min_size(origin, Vec2::splat(30.0));
        paint_logo(painter, logo);
        painter.text(
            pos2(logo.right() + 14.0, logo.center().y),
            Align2::LEFT_CENTER,
            {
                let keys = self.shell.hotkeys();
                let label = |key: Option<Option<&'static str>>| match key {
                    Some(Some(label)) => label,
                    Some(None) => "хоткей занят",
                    None => "…",
                };
                format!(
                    "{} карточек  ·  {} — записать  ·  {} — найти  ·  F1 — debug",
                    self.cards.len(),
                    label(keys.map(|k| k.capture)),
                    label(keys.map(|k| k.search)),
                )
            },
            FontId::proportional(13.0),
            theme::muted(),
        );
    }

    /// The layer's empty space: a double-click there makes a new note, its top
    /// left under the pointer. Call before the cards are drawn, so they sit on
    /// top of it. Where the note goes (layer coordinates), on a double-click.
    pub(super) fn background_ui(&self, ui: &mut Ui) -> Option<Pos2> {
        // Taken now: a card past the window's edge grows max_rect as it's drawn.
        let full = ui.max_rect();
        let resp = ui.interact(full, Id::new("layer-background"), Sense::click());
        if !resp.double_clicked() {
            return None;
        }
        let at = resp.interact_pointer_pos()?;
        // A card that senses only drag lets the click through to here: the
        // double-click is the card's (it edits), not a new note under it.
        let on_card = self
            .cards
            .iter()
            .any(|c| Rect::from_min_size(full.min + c.pos.to_vec2(), c.shown_size()).contains(at));
        // Not over a popup, the header and "+"/"⋯" row, the bottom handle or the toast.
        let over_popup = ui.ctx().layer_id_at(at).is_some_and(|l| l.order != egui::Order::Background);
        let in_header = at.y < full.top() + HEADER_BAND;
        let at_bottom = at.y > full.bottom() - if self.toast.is_some() { TOAST_BAND } else { HANDLE_BAND };
        if on_card || over_popup || in_header || at_bottom {
            return None;
        }
        Some(note_pos_at(at - full.min.to_vec2(), full.size()))
    }

    /// "+" left of "⋯" (`menu`, its rect): a new empty note, its editor open.
    fn new_note_button(&mut self, ui: &mut Ui, menu: Rect) {
        let rect = menu.translate(vec2(-(menu.width() + 8.0), 0.0));
        let button = ui.interact(rect, Id::new("layer-new-note"), Sense::click());
        button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Новая заметка"));
        let t = hover_t(ui, button.id, button.hovered());
        glass_button(ui, rect.expand(t * 1.5), 10.0, t);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "\u{E710}", theme::icons(16.0), theme::dim().lerp_to_gamma(theme::text(), t));
        let button = button.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Новая заметка");
        if button.clicked() {
            self.new_note(card::free_slot(&self.cards, self.area(ui)));
        }
    }

    /// "⋯" in the top right corner of the layer: what the tray menu offers, without the tray.
    pub(super) fn menu_ui(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let rect = Rect::from_min_size(pos2(full.right() - 32.0 - 40.0, full.top() + 16.0), vec2(40.0, 36.0));
        self.new_note_button(ui, rect);
        let button = ui.interact(rect, Id::new("layer-menu"), Sense::click());
        button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Меню"));
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&button));
        let t = hover_t(ui, button.id, open || button.hovered());
        glass_button(ui, rect.expand(t * 1.5), 10.0, t);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "\u{E712}", theme::icons(16.0), theme::dim().lerp_to_gamma(theme::text(), t));
        let button = button.on_hover_cursor(CursorIcon::PointingHand);

        let keys = self.shell.hotkeys().unwrap_or_default();
        let (capture_key, search_key) = (keys.capture.unwrap_or(""), keys.search.unwrap_or(""));
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
                item(ui, "Новая заметка", "", MenuItem::NewNote);
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
            MenuItem::NewNote => {
                self.new_note(card::free_slot(&self.cards, self.area(ui)));
                return;
            }
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
    pub(super) fn curtain_dir(&self) -> f32 {
        if self.curtain_top { -1.0 } else { 1.0 }
    }

    /// The handle at the top or bottom center of the layer: a click, or a drag
    /// toward that edge, rolls the layer into a tab, and the desktop under it is free.
    /// `full`: the layer's rect, taken before the cards are drawn (they can grow
    /// `ui.max_rect()` past the window's edge).
    pub(super) fn curtain_ui(&mut self, ui: &mut Ui, full: Rect) {
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
    pub(super) fn step_curtain(&mut self, ctx: &egui::Context) {
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
    pub(super) fn expand(&mut self, ctx: &egui::Context) -> bool {
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
    pub(super) fn tab_ui(&mut self, ui: &mut Ui) {
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

    pub(super) fn debug_ui(&mut self, ui: &mut Ui) {
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

    /// Shows the layer over the other windows.
    pub(super) fn summon(&mut self, ctx: &egui::Context) {
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
    pub(super) fn dismiss(&mut self, ctx: &egui::Context) {
        if self.dismiss_hides {
            win::RAISED.store(false, Ordering::Relaxed);
            self.set_layer_visible(ctx, false);
        } else if let Some(h) = self.hwnd {
            win::lower(h);
        }
    }

    pub(super) fn tray_click(&mut self, ctx: &egui::Context) {
        let summoned = win::RAISED.load(Ordering::Relaxed) || win::ms_since_auto_lowered() < TRAY_CLICK_AFTER_LOWER_MS;
        if self.layer_visible && summoned {
            self.dismiss(ctx);
        } else {
            self.summon(ctx);
        }
    }

    pub(super) fn set_monitor(&mut self, device: &str) {
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

    pub(super) fn set_curtain_top(&mut self, top: bool) {
        self.curtain_top = top;
        let _ = self.store.set_setting(SET_CURTAIN, if top { "top" } else { "bottom" });
        // A collapsed tab moves to the other edge right away.
        if let (true, Some(h)) = (self.collapsed, self.hwnd)
            && let Some(m) = win::monitor_of(h)
        {
            place_tab(h, &m, top);
        }
    }

    pub(super) fn set_backdrop(&mut self, backdrop: Backdrop) {
        self.backdrop = backdrop;
        if let Some(h) = self.hwnd {
            win::apply_backdrop(h, backdrop, self.collapsed);
        }
        let _ = self.store.set_setting(SET_BACKDROP, backdrop.key());
    }

    pub(super) fn set_pin_bottom(&mut self, on: bool) {
        if let Some(h) = self.hwnd {
            win::set_pin_bottom(h, on);
        }
        self.shell.pin_bottom.store(on, Ordering::Relaxed);
        let _ = self.store.set_setting(SET_PIN_BOTTOM, if on { "1" } else { "0" });
    }

    pub(super) fn set_layer_visible(&mut self, ctx: &egui::Context, visible: bool) {
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

    pub(super) fn cycle_backdrop(&mut self) {
        self.set_backdrop(self.backdrop.next());
    }
}
