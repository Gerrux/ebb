//! Background upkeep on a daily timer: the weekly backup and the invitation to
//! the weekly review. The work runs on its own thread with its own connection,
//! at background priority; the UI thread only draws the invitation panel and
//! sleeps until the next moment anything can change (`request_repaint_after`).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{Rect, RichText, Ui, UiBuilder, vec2};

use crate::resurface;
use crate::review::{self, Invite};
use crate::store::{Store, backup_dir};
use crate::theme;

/// The first run waits this long after start, so it never lands before the
/// first frame or on top of logon.
const AFTER_START: i64 = 3;

/// An invitation to show: `count` cards wait for the slot's review.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Offer {
    count: usize,
    slot: i64,
    until: i64,
}

/// What a run found: the invitation to show, if any, and when to look again.
struct Outcome {
    offer: Option<Offer>,
    wake: i64,
}

/// What the user chose on the invitation panel.
pub enum Choice {
    None,
    /// Open the review.
    Start,
}

pub struct Upkeep {
    /// Not during benchmarks: it would land in their idle window.
    enabled: bool,
    /// When the next run is due (unix seconds).
    next: i64,
    job: Option<mpsc::Receiver<Outcome>>,
    /// The inputs changed while a run was in flight: run again once it's back.
    dirty: bool,
    offer: Option<Offer>,
    /// When the panel first appeared, for the slide-in.
    shown_at: Option<Instant>,
}

impl Upkeep {
    pub fn new(now: i64) -> Self {
        Self {
            enabled: crate::bench::path().is_none(),
            next: now + AFTER_START,
            job: None,
            dirty: false,
            offer: None,
            shown_at: None,
        }
    }

    /// The schedule or the clock changed: the invitation is dropped and the job
    /// runs again now.
    pub fn rerun(&mut self, ctx: &egui::Context) {
        self.offer = None;
        if self.job.is_some() {
            self.dirty = true;
        } else {
            self.next = 0;
        }
        ctx.request_repaint();
    }

    /// Cards changed in the library: a review that has started since the slot
    /// closes the invitation (one settings read).
    pub fn cards_changed(&mut self, store: &Store) {
        if let Some(offer) = self.offer
            && store.last_review_at() >= Some(offer.slot)
        {
            self.offer = None;
        }
        // A run in flight may have read the queue before the review began.
        if self.job.is_some() {
            self.dirty = true;
        }
    }

    /// Takes a finished run's outcome; false while it's still running. An
    /// outcome from before the inputs changed is dropped and the run repeated.
    fn collect(&mut self) -> bool {
        let Some(job) = &self.job else {
            return true;
        };
        let outcome = match job.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return false,
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        self.job = None;
        if std::mem::take(&mut self.dirty) {
            self.next = 0;
        } else if let Some(outcome) = outcome {
            self.offer = outcome.offer;
            self.next = outcome.wake;
        }
        true
    }

    /// Collects a finished run and starts the next one when it's due; otherwise
    /// asks for a repaint at that moment.
    pub fn step(&mut self, ctx: &egui::Context) {
        if !self.enabled || !self.collect() {
            return;
        }
        let (now, offset) = (resurface::unix_now(), crate::search::local_offset_secs());
        if now < self.next {
            ctx.request_repaint_after(Duration::from_secs((self.next - now) as u64));
            return;
        }
        // If the thread dies without an answer, look again tomorrow.
        self.next = resurface::next_day_start(now, offset);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("upkeep".into())
            .spawn(move || {
                crate::import_ui::background_priority();
                let outcome = run(now, offset).unwrap_or_else(|e| {
                    eprintln!("upkeep failed: {e}");
                    Outcome {
                        offer: None,
                        wake: resurface::next_day_start(now, offset),
                    }
                });
                let _ = tx.send(outcome);
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.job = Some(rx);
        }
    }

    /// Draws the invitation, if there is one, where the import panel goes.
    /// `store` is the layer's: a dismissal is written through it.
    pub fn ui(&mut self, ui: &mut Ui, store: &Store) -> Choice {
        let Some(offer) = self.offer else {
            self.shown_at = None;
            return Choice::None;
        };
        let since = *self.shown_at.get_or_insert_with(Instant::now);
        let appear =
            super::panel_ease(since.elapsed().as_secs_f32() / super::PANEL_APPEAR.as_secs_f32());
        if appear < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.multiply_opacity(appear);
        let rect = Rect::from_min_size(
            ui.max_rect().right_top() + vec2(-452.0 + (1.0 - appear) * 24.0, 64.0),
            vec2(420.0, 108.0),
        );
        super::glass_panel(ui, rect, false);

        let mut choice = Choice::None;
        let mut dismissed = false;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(20.0, 16.0))), |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new("\u{E787}").font(theme::icons(15.0)).color(theme::dim()));
                ui.label(
                    RichText::new("Еженедельный обзор")
                        .font(theme::semibold(15.0))
                        .color(theme::text()),
                );
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(review::invite_text(offer.count))
                    .size(13.0)
                    .color(theme::dim()),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Начать").clicked() {
                    choice = Choice::Start;
                }
                if ui.button("Не сейчас").clicked() {
                    dismissed = true;
                }
            });
        });
        if dismissed {
            let _ = store.set_setting(review::SET_INVITE_DISMISSED, &offer.slot.to_string());
            self.offer = None;
        }
        choice
    }
}

fn run(now: i64, offset: i64) -> rusqlite::Result<Outcome> {
    let store = Store::open()?;
    if let Err(e) = store.backup_if_due(&backup_dir(), now, offset) {
        eprintln!("backup failed: {e}");
    }
    let Some(schedule) = review::schedule(&store) else {
        return Ok(Outcome {
            offer: None,
            wake: resurface::next_day_start(now, offset),
        });
    };
    let invite = review::invite(now, offset, schedule, review::handled(&store));
    let offer = match invite {
        Invite::Open { slot, until } => {
            let count = store.review_queue(now, review::REVIEW_CARDS)?.len();
            (count >= review::INVITE_MIN_CARDS).then_some(Offer { count, slot, until })
        }
        Invite::Closed { .. } => None,
    };
    Ok(Outcome {
        offer,
        wake: wake_time(now, offset, invite),
    })
}

/// The earliest moment anything can change: the invitation closing, or the next
/// slot opening, but no later than the next day, when the backup is re-checked.
fn wake_time(now: i64, offset: i64, invite: Invite) -> i64 {
    let invite_at = match invite {
        Invite::Open { until, .. } => until,
        Invite::Closed { next } => next,
    };
    invite_at.min(resurface::next_day_start(now, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn wakes_at_the_end_of_an_open_invitation_if_it_comes_first() {
        let soon = NOW + 600;
        let wake = wake_time(NOW, 0, Invite::Open { slot: NOW - 100, until: soon });
        assert_eq!(wake, soon.min(resurface::next_day_start(NOW, 0)));
    }

    #[test]
    fn wakes_daily_while_the_next_slot_is_far() {
        let wake = wake_time(NOW, 0, Invite::Closed { next: NOW + 5 * resurface::DAY });
        assert_eq!(wake, resurface::next_day_start(NOW, 0));
        assert!(wake > NOW);
    }

    fn idle() -> Upkeep {
        Upkeep {
            enabled: true,
            next: NOW,
            job: None,
            dirty: false,
            offer: None,
            shown_at: None,
        }
    }

    const OFFER: Offer = Offer {
        count: 5,
        slot: NOW - 100,
        until: NOW + 1_000,
    };

    #[test]
    fn a_finished_run_brings_its_offer_and_wake_time() {
        let mut upkeep = idle();
        let (tx, rx) = mpsc::channel();
        upkeep.job = Some(rx);
        assert!(!upkeep.collect());
        tx.send(Outcome { offer: Some(OFFER), wake: NOW + 500 }).unwrap();
        assert!(upkeep.collect());
        assert_eq!((upkeep.offer, upkeep.next), (Some(OFFER), NOW + 500));
        assert!(upkeep.job.is_none());
    }

    #[test]
    fn a_run_overtaken_by_a_change_is_dropped_and_repeated() {
        let mut upkeep = idle();
        let (tx, rx) = mpsc::channel();
        upkeep.job = Some(rx);
        // E.g. invitations turned off while the run was reading the schedule.
        upkeep.dirty = true;
        tx.send(Outcome { offer: Some(OFFER), wake: NOW + 500 }).unwrap();
        assert!(upkeep.collect());
        assert_eq!((upkeep.offer, upkeep.next), (None, 0));
        assert!(!upkeep.dirty);
    }

    #[test]
    fn a_dead_run_keeps_the_fallback_wake_time() {
        let mut upkeep = idle();
        let (tx, rx) = mpsc::channel::<Outcome>();
        upkeep.job = Some(rx);
        drop(tx);
        assert!(upkeep.collect());
        assert_eq!(upkeep.next, NOW);
    }

    #[test]
    fn wakes_at_the_next_slot_if_it_comes_before_tomorrow() {
        let next = NOW + 60;
        assert_eq!(wake_time(NOW, 0, Invite::Closed { next }), next);
    }
}
