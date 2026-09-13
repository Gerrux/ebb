//! Onboarding offer and result panel for the Sticky Notes import.
//!
//! Scanning (copy + parse + plan) runs on a background thread at background
//! I/O priority, well after the first frame; the UI thread only draws the panel
//! and writes the chosen notes in one transaction when the user agrees.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, vec2};

use crate::card::{self, Card};
use crate::store::Store;
use crate::sticky::{self, Plan, Stats};
use crate::theme::{self, TEXT, TEXT_DIM, TEXT_MUTED};

const SOURCE_PREFIX: &str = "sticky:";
/// settings key: "done" after an import, "never" when declined for good.
const SETTING: &str = "sticky_import";

enum State {
    Idle,
    Scanning,
    Offer(Plan),
    Done { batch: i64, stats: Stats },
    Failed(String),
}

pub struct StickyImport {
    state: Arc<Mutex<State>>,
}

impl Default for StickyImport {
    fn default() -> Self {
        Self { state: Arc::new(Mutex::new(State::Idle)) }
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

fn background_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

impl StickyImport {
    /// Offers the import on startup unless it was done or declined. The scan waits a
    /// few seconds to stay away from logon and the first frames.
    pub fn maybe_offer(&self, ctx: &egui::Context, store: &Store, cards: &[Card]) {
        if sticky::plum_path().is_none() || matches!(store.setting(SETTING).as_deref(), Some("done" | "never")) {
            return;
        }
        self.scan(ctx, store, cards, false);
    }

    /// Scans Sticky Notes and shows the offer. `manual` (from the tray) ignores the
    /// setting, starts at once and also reports when there is nothing new.
    pub fn scan(&self, ctx: &egui::Context, store: &Store, cards: &[Card], manual: bool) {
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
        let already = store.imported_ids(SOURCE_PREFIX).unwrap_or_default();
        let layer_free = sticky::LAYER_TARGET.saturating_sub(cards.iter().filter(|c| !c.archived).count());
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
                        let plan = sticky::plan(&notes, &already, layer_free, now());
                        let (ws1, private) = crate::win::memory_mib();
                        crate::app::append_timing_log(&format!(
                            "sticky scan: {} notes, read {read_ms:.0} ms, plan {:.0} ms, ws +{:.1} MiB (private {private:.1})\n",
                            notes.len(),
                            started.elapsed().as_secs_f64() * 1000.0 - read_ms,
                            ws1 - ws0,
                        ));
                        match (plan.notes.is_empty(), manual) {
                            (false, _) => State::Offer(plan),
                            (true, true) => State::Failed(format!(
                                "Новых заметок нет: {} уже импортированы, пустых {}, дубликатов {}.",
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

    fn apply(&self, plan: &Plan, store: &mut Store, cards: &mut Vec<Card>, area: egui::Vec2) -> State {
        // Positions for the notes that go onto the layer, into free grid slots.
        let mut layout = cards.clone();
        let positions: Vec<Option<egui::Pos2>> = plan
            .notes
            .iter()
            .map(|n| {
                n.on_layer.then(|| {
                    let pos = card::free_slot(&layout, area);
                    layout.push(Card {
                        id: -1,
                        kind: n.kind,
                        title: String::new(),
                        body: String::new(),
                        tags: Vec::new(),
                        pinned: false,
                        archived: false,
                        pos,
                        size: card::DEFAULT_SIZE,
                        created_at: 0,
                    });
                    pos
                })
            })
            .collect();
        match store.import(SOURCE_PREFIX, &plan.notes, &positions) {
            Ok(batch) => {
                let _ = store.set_setting(SETTING, "done");
                if let Ok(loaded) = store.load() {
                    *cards = loaded;
                }
                State::Done { batch, stats: plan.stats.clone() }
            }
            Err(e) => State::Failed(format!("Импорт не удался: {e}")),
        }
    }

    /// Draws the panel, if any, in the top-right corner of the layer.
    pub fn ui(&self, ui: &mut Ui, store: &mut Store, cards: &mut Vec<Card>, area: egui::Vec2) {
        let mut state = self.state.lock().unwrap();
        if matches!(*state, State::Idle | State::Scanning) {
            return;
        }
        let height = match &*state {
            State::Offer(_) => 236.0,
            State::Done { .. } => 132.0,
            _ => 110.0,
        };
        let rect = Rect::from_min_size(ui.max_rect().right_top() + vec2(-452.0, 64.0), vec2(420.0, height));
        crate::app::glass_panel(ui, rect, false);

        let mut next = None;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(20.0, 16.0))), |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            let title = |ui: &mut Ui, text: &str| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("\u{E8B5}").font(theme::icons(15.0)).color(TEXT_DIM));
                    ui.label(RichText::new(text).font(theme::semibold(15.0)).color(TEXT));
                });
                ui.add_space(2.0);
            };
            let line = |ui: &mut Ui, text: String, color| {
                ui.label(RichText::new(text).size(13.0).color(color));
            };
            match &*state {
                State::Offer(plan) => {
                    let s = &plan.stats;
                    title(ui, "Импорт из Sticky Notes");
                    line(ui, format!("Нашлось {} для импорта из {}.", notes(s.importable), s.total), TEXT);
                    line(ui, format!("На слой: {} — открытые и недавние.", s.on_layer), TEXT_DIM);
                    line(
                        ui,
                        format!("В архив: {} (не менялись больше года: {}). Их найдёт поиск.", s.archived, s.old),
                        TEXT_DIM,
                    );
                    let kinds: Vec<String> = s.by_kind.iter().map(|(k, n)| format!("{} {n}", k.label())).collect();
                    line(ui, kinds.join(" · "), TEXT_DIM);
                    let mut skipped = vec![format!("пустых {}", s.empty), format!("дубликатов {}", s.duplicates)];
                    if s.already_imported > 0 {
                        skipped.push(format!("уже импортированных {}", s.already_imported));
                    }
                    line(ui, format!("Пропущу: {}.", skipped.join(", ")), TEXT_MUTED);
                    line(ui, "Sticky Notes не меняются: читается копия базы.".into(), TEXT_MUTED);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Импортировать").clicked() {
                            next = Some(self.apply(plan, store, cards, area));
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
                State::Done { batch, stats } => {
                    title(ui, "Sticky Notes импортированы");
                    line(
                        ui,
                        format!("{}: {} на слое, {} в архиве.", notes(stats.importable), stats.on_layer, stats.archived),
                        TEXT,
                    );
                    line(ui, "Оригиналы в Sticky Notes остались на месте.".into(), TEXT_MUTED);
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
                    line(ui, message.clone(), TEXT_DIM);
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
