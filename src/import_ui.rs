//! Onboarding offer and result panel for the Sticky Notes import.
//!
//! Scanning (copy + parse + plan) runs on a background thread at background
//! I/O priority, well after the first frame; the UI thread only draws the panel
//! and writes the chosen notes in one transaction when the user agrees.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, vec2};

use crate::card::Card;
use crate::store::Store;
use crate::sticky::{self, Plan, Stats};
use crate::theme;

const SOURCE_PREFIX: &str = "sticky:";
/// settings key: "done" after an import, "never" when declined for good.
const SETTING: &str = "sticky_import";

enum State {
    Idle,
    Scanning,
    Offer(Offer),
    Done { batch: i64, stats: Stats, replaced: usize },
    Failed(String),
}

struct Offer {
    plan: Plan,
    /// Cards of an earlier import that are unchanged and get replaced.
    replace: Vec<i64>,
    /// Earlier imported notes left alone because they were changed in Ebb.
    kept: usize,
    /// Open notes that were on the layer's monitor (placed exactly).
    in_place: usize,
}

pub struct StickyImport {
    state: Arc<Mutex<State>>,
    /// Which panel was drawn last and when it first appeared, for the slide-in.
    shown: Mutex<Option<(std::mem::Discriminant<State>, std::time::Instant)>>,
}

impl Default for StickyImport {
    fn default() -> Self {
        Self { state: Arc::new(Mutex::new(State::Idle)), shown: Mutex::new(None) }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "1 заметка", "3 заметки", "11 заметок".
fn notes(n: usize) -> String {
    let word = match (n % 10, n % 100) {
        (_, 11..=14) => "заметок",
        (1, _) => "заметка",
        (2..=4, _) => "заметки",
        _ => "заметок",
    };
    format!("{n} {word}")
}

pub(crate) fn background_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

impl StickyImport {
    /// Offers the import on startup unless it was done or declined. The scan waits a
    /// few seconds to stay away from logon and the first frames.
    pub fn maybe_offer(&self, ctx: &egui::Context, store: &Store, layer: Option<isize>) {
        if sticky::plum_path().is_none() || matches!(store.setting(SETTING).as_deref(), Some("done" | "never")) {
            return;
        }
        self.scan(ctx, store, layer, false);
    }

    /// Scans Sticky Notes and shows the offer. `manual` (from the tray) ignores the
    /// setting, starts at once and also reports when there is nothing new. An earlier
    /// import is replaced, except cards changed since.
    pub fn scan(&self, ctx: &egui::Context, store: &Store, layer: Option<isize>, manual: bool) {
        let delay = if manual { Duration::ZERO } else { Duration::from_secs(3) };
        let Some(path) = sticky::plum_path() else {
            *self.state.lock().unwrap() = State::Failed("Sticky Notes не найдены на этом компьютере.".into());
            ctx.request_repaint();
            return;
        };
        {
            let mut state = self.state.lock().unwrap();
            if matches!(*state, State::Scanning) {
                return;
            }
            *state = State::Scanning;
        }
        let (keep, replace) = store.import_state(SOURCE_PREFIX).unwrap_or_default();
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("sticky-scan".into())
            .spawn(move || {
                std::thread::sleep(delay);
                background_priority();
                let started = std::time::Instant::now();
                let (ws0, _) = crate::win::memory_mib();
                let next = match sticky::read_notes(&path) {
                    Ok(notes) => {
                        let read_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let plan = sticky::plan(&notes, &keep, now());
                        let layer_monitor = layer.and_then(crate::win::monitor_of);
                        let in_place = plan
                            .notes
                            .iter()
                            .filter(|n| n.on_layer)
                            .filter_map(|n| n.window.as_ref())
                            .filter(|w| layer_monitor.as_ref().is_some_and(|m| m.matches_device(&w.device)))
                            .count();
                        let (ws1, private) = crate::win::memory_mib();
                        crate::app::append_timing_log(&format!(
                            "sticky scan: {} notes, read {read_ms:.0} ms, plan {:.0} ms, ws +{:.1} MiB (private {private:.1})\n",
                            notes.len(),
                            started.elapsed().as_secs_f64() * 1000.0 - read_ms,
                            ws1 - ws0,
                        ));
                        match (plan.notes.is_empty(), manual) {
                            (false, _) => State::Offer(Offer { kept: keep.len(), replace, in_place, plan }),
                            (true, true) => State::Failed(format!(
                                "Нечего импортировать: {} изменены в Ebb и оставлены как есть, пустых {}, дубликатов {}.",
                                plan.stats.already_imported, plan.stats.empty, plan.stats.duplicates
                            )),
                            (true, false) => State::Idle,
                        }
                    }
                    Err(e) => State::Failed(format!("Не удалось прочитать Sticky Notes: {e}")),
                };
                *state.lock().unwrap() = next;
                ctx.request_repaint();
                // Parsing hundreds of notes touched a few MiB; don't keep them resident.
                std::thread::sleep(Duration::from_millis(500));
                crate::win::trim_working_set();
            })
            .expect("spawn sticky scan");
    }

    fn apply(offer: &Offer, store: &mut Store, cards: &mut Vec<Card>, screen: &sticky::Screen) -> State {
        // Cards being replaced don't block their own spots.
        let occupied: Vec<Rect> = cards
            .iter()
            .filter(|c| !offer.replace.contains(&c.id))
            .map(|c| Rect::from_min_size(c.pos, c.size))
            .collect();
        let rects = sticky::layout(&offer.plan.notes, screen, &occupied);
        match store.import(SOURCE_PREFIX, &offer.plan.notes, &rects, &offer.replace) {
            Ok(batch) => {
                let _ = store.set_setting(SETTING, "done");
                if let Ok(loaded) = store.load() {
                    *cards = loaded;
                }
                State::Done { batch, stats: offer.plan.stats.clone(), replaced: offer.replace.len() }
            }
            Err(e) => State::Failed(format!("Импорт не удался: {e}")),
        }
    }

    /// Draws the panel, if any, in the top-right corner of the layer.
    pub fn ui(&self, ui: &mut Ui, store: &mut Store, cards: &mut Vec<Card>, layer: Option<isize>) {
        let mut state = self.state.lock().unwrap();
        if matches!(*state, State::Idle | State::Scanning) {
            *self.shown.lock().unwrap() = None;
            return;
        }
        // Slide in from the right while fading in, each time a new panel shows up.
        let since = {
            let mut shown = self.shown.lock().unwrap();
            let kind = std::mem::discriminant(&*state);
            match *shown {
                Some((k, at)) if k == kind => at,
                _ => {
                    let now = std::time::Instant::now();
                    *shown = Some((kind, now));
                    now
                }
            }
        };
        let appear = crate::app::panel_ease(since.elapsed().as_secs_f32() / crate::app::PANEL_APPEAR.as_secs_f32());
        if appear < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.multiply_opacity(appear);
        let height = match &*state {
            State::Offer(o) if !o.replace.is_empty() || o.kept > 0 => 258.0,
            State::Offer(_) => 236.0,
            State::Done { replaced, .. } if *replaced > 0 => 154.0,
            State::Done { .. } => 132.0,
            _ => 110.0,
        };
        let area = ui.max_rect().size();
        let pixels_per_point = ui.ctx().pixels_per_point();
        let rect = Rect::from_min_size(ui.max_rect().right_top() + vec2(-452.0 + (1.0 - appear) * 24.0, 64.0), vec2(420.0, height));
        crate::app::glass_panel(ui, rect, false);

        let mut next = None;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(20.0, 16.0))), |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            let title = |ui: &mut Ui, text: &str| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("\u{E8B5}").font(theme::icons(15.0)).color(theme::dim()));
                    ui.label(RichText::new(text).font(theme::semibold(15.0)).color(theme::text()));
                });
                ui.add_space(2.0);
            };
            let line = |ui: &mut Ui, text: String, color| {
                ui.label(RichText::new(text).size(13.0).color(color));
            };
            match &*state {
                State::Offer(offer) => {
                    let s = &offer.plan.stats;
                    title(ui, "Импорт из Sticky Notes");
                    line(ui, format!("Нашлось {} для импорта из {}.", notes(s.importable), s.total), theme::text());
                    line(
                        ui,
                        format!(
                            "На слой: {} открытых, как на экране ({} на свои места, {} рядом).",
                            s.on_layer,
                            offer.in_place,
                            s.on_layer - offer.in_place
                        ),
                        theme::dim(),
                    );
                    line(ui, format!("В архив: {} (давно не менялись: {}). Их найдёт поиск.", s.archived, s.old), theme::dim());
                    let kinds: Vec<String> = s.by_kind.iter().map(|(k, n)| format!("{} {n}", k.label())).collect();
                    line(ui, kinds.join(" · "), theme::dim());
                    line(ui, format!("Пропущу: пустых {}, дубликатов {}.", s.empty, s.duplicates), theme::muted());
                    if !offer.replace.is_empty() || offer.kept > 0 {
                        line(
                            ui,
                            format!(
                                "Прошлый импорт: заменю {}, оставлю изменённые тобой ({}).",
                                offer.replace.len(),
                                offer.kept
                            ),
                            theme::muted(),
                        );
                    }
                    line(ui, "Sticky Notes не меняются: читается копия базы.".into(), theme::muted());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Импортировать").clicked() {
                            let layer_monitor = layer.and_then(crate::win::monitor_of);
                            let screen = sticky::Screen { layer: layer_monitor.as_ref(), pixels_per_point, area };
                            next = Some(Self::apply(offer, store, cards, &screen));
                        }
                        if ui.button("Не сейчас").clicked() {
                            next = Some(State::Idle);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Не предлагать").clicked() {
                                let _ = store.set_setting(SETTING, "never");
                                next = Some(State::Idle);
                            }
                        });
                    });
                }
                State::Done { batch, stats, replaced } => {
                    title(ui, "Sticky Notes импортированы");
                    line(
                        ui,
                        format!("{}: {} на слое, {} в архиве.", notes(stats.importable), stats.on_layer, stats.archived),
                        theme::text(),
                    );
                    if *replaced > 0 {
                        line(ui, format!("Заменено из прошлого импорта: {replaced}.",), theme::muted());
                    }
                    line(ui, "Оригиналы в Sticky Notes остались на месте.".into(), theme::muted());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Готово").clicked() {
                            next = Some(State::Idle);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Отменить импорт").clicked() {
                                next = Some(match store.undo_import(*batch) {
                                    Ok(_) => {
                                        let _ = store.set_setting(SETTING, "undone");
                                        if let Ok(loaded) = store.load() {
                                            *cards = loaded;
                                        }
                                        State::Idle
                                    }
                                    Err(e) => State::Failed(format!("Отмена не удалась: {e}")),
                                });
                            }
                        });
                    });
                }
                State::Failed(message) => {
                    title(ui, "Импорт из Sticky Notes");
                    line(ui, message.clone(), theme::dim());
                    ui.add_space(6.0);
                    if ui.button("Закрыть").clicked() {
                        next = Some(State::Idle);
                    }
                }
                State::Idle | State::Scanning => {}
            }
        });
        if let Some(next) = next {
            *state = next;
        }
    }
}
