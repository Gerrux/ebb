//! When to invite the user to the weekly review: pure schedule arithmetic over
//! unix times and a fixed UTC offset, no I/O besides reading two settings. The
//! invitation opens at the scheduled weekday and hour and stays for a few days;
//! dismissing it, or starting a review, closes it until the next week's slot.

use crate::resurface::DAY;
use crate::store::Store;

/// "0" turns the invitation off; on by default.
pub const SET_INVITE: &str = "review.invite";
/// Weekday of the invitation, 0 = Monday … 6 = Sunday.
pub const SET_DAY: &str = "review.day";
/// Local hour of the invitation, 0–23.
pub const SET_HOUR: &str = "review.hour";
/// The slot (unix time) whose invitation the user dismissed.
pub const SET_INVITE_DISMISSED: &str = "review.invite_dismissed";

/// Cards in one review.
pub const REVIEW_CARDS: usize = 15;
/// How long an invitation stays open after its slot.
pub const INVITE_DAYS: i64 = 3;
/// Fewer candidates than this and there's nothing worth inviting to.
pub const INVITE_MIN_CARDS: usize = 3;

/// The most a daylight saving change moves a slot by.
const DST_SLACK: i64 = 3_600;

/// Seconds a card is assumed to take in a review.
const SECONDS_PER_CARD: usize = 15;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Schedule {
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u8,
    pub hour: u8,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            weekday: 4,
            hour: 16,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Invite {
    /// The invitation for the slot is showing until `until`.
    Open { slot: i64, until: i64 },
    /// Nothing to show; the next slot comes at `next`.
    Closed { next: i64 },
}

/// The schedule from settings; `None` when invitations are off. Missing or
/// out-of-range values fall back to the defaults.
pub fn schedule(store: &Store) -> Option<Schedule> {
    if store.setting(SET_INVITE).as_deref() == Some("0") {
        return None;
    }
    let default = Schedule::default();
    let read = |key: &str, max: u8, fallback: u8| {
        store
            .setting(key)
            .and_then(|v| v.trim().parse::<u8>().ok())
            .filter(|v| *v <= max)
            .unwrap_or(fallback)
    };
    Some(Schedule {
        weekday: read(SET_DAY, 6, default.weekday),
        hour: read(SET_HOUR, 23, default.hour),
    })
}

/// The most recent scheduled moment at or before `now`.
pub fn last_slot(now: i64, utc_offset: i64, s: Schedule) -> i64 {
    let local = now + utc_offset;
    let day = local.div_euclid(DAY);
    // 1970-01-01 was a Thursday (3, counting from Monday).
    let weekday = (day + 3).rem_euclid(7);
    let back = (weekday - i64::from(s.weekday)).rem_euclid(7);
    let mut slot = (day - back) * DAY + i64::from(s.hour) * 3_600;
    if slot > local {
        slot -= 7 * DAY;
    }
    slot - utc_offset
}

/// The first scheduled moment after `now`.
pub fn next_slot(now: i64, utc_offset: i64, s: Schedule) -> i64 {
    last_slot(now, utc_offset, s) + 7 * DAY
}

/// `handled` is the latest time the user dealt with the invitation: dismissed
/// it or started a review. It counts for the current slot only if it's not
/// earlier than the slot itself, give or take an hour: the slot is worked out
/// with today's UTC offset, so after a daylight saving change it can land an
/// hour off the one that was dismissed.
pub fn invite(now: i64, utc_offset: i64, s: Schedule, handled: Option<i64>) -> Invite {
    let slot = last_slot(now, utc_offset, s);
    let until = slot + INVITE_DAYS * DAY;
    if now < until && handled.is_none_or(|h| h < slot - DST_SLACK) {
        Invite::Open { slot, until }
    } else {
        Invite::Closed {
            next: next_slot(now, utc_offset, s),
        }
    }
}

/// The later of the dismissed slot and the start of the last review.
pub fn handled(store: &Store) -> Option<i64> {
    let dismissed = store
        .setting(SET_INVITE_DISMISSED)
        .and_then(|v| v.parse::<i64>().ok());
    dismissed.max(store.last_review_at())
}

/// "Разобрать 12 заметок · ~3 мин".
pub fn invite_text(cards: usize) -> String {
    let minutes = (cards * SECONDS_PER_CARD).div_ceil(60).max(1);
    let noun = match (cards % 100, cards % 10) {
        (11..=14, _) => "заметок",
        (_, 1) => "заметку",
        (_, 2..=4) => "заметки",
        _ => "заметок",
    };
    format!("Разобрать {cards} {noun} · ~{minutes} мин")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::days_from_civil;

    const FRIDAY_16: Schedule = Schedule {
        weekday: 4,
        hour: 16,
    };

    fn at(y: i64, m: i64, d: i64, h: i64) -> i64 {
        days_from_civil(y, m, d) * DAY + h * 3_600
    }

    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("ebb-review-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::open_at(dir.join("t.db")).unwrap()
    }

    #[test]
    fn epoch_was_a_thursday() {
        let thursday = Schedule {
            weekday: 3,
            hour: 0,
        };
        assert_eq!(last_slot(5 * 3_600, 0, thursday), 0);
        assert_eq!(next_slot(5 * 3_600, 0, thursday), 7 * DAY);
    }

    #[test]
    fn slots_across_a_week_boundary() {
        // 2024-05-10 is a Friday.
        let slot = at(2024, 5, 10, 16);
        assert_eq!(last_slot(slot, 0, FRIDAY_16), slot);
        assert_eq!(last_slot(slot - 1, 0, FRIDAY_16), slot - 7 * DAY);
        // Monday after, and the Friday before the next slot.
        assert_eq!(last_slot(at(2024, 5, 13, 9), 0, FRIDAY_16), slot);
        assert_eq!(last_slot(at(2024, 5, 17, 15), 0, FRIDAY_16), slot);
        assert_eq!(next_slot(at(2024, 5, 13, 9), 0, FRIDAY_16), slot + 7 * DAY);
        // Same weekday earlier in the day belongs to the previous week.
        assert_eq!(last_slot(at(2024, 5, 10, 8), 0, FRIDAY_16), slot - 7 * DAY);
    }

    #[test]
    fn slots_follow_the_local_offset() {
        let offset = 3 * 3_600;
        let slot = at(2024, 5, 10, 13); // Friday 16:00 at UTC+3
        assert_eq!(last_slot(slot, offset, FRIDAY_16), slot);
        assert_eq!(last_slot(slot - 1, offset, FRIDAY_16), slot - 7 * DAY);
        // 22:00 UTC Thursday is already Friday 01:00 locally, but before the hour.
        assert_eq!(
            last_slot(at(2024, 5, 9, 22), offset, FRIDAY_16),
            slot - 7 * DAY
        );
        // Negative offset: Friday 16:00 at UTC-5 is Friday 21:00 UTC.
        let west = -5 * 3_600;
        assert_eq!(
            last_slot(at(2024, 5, 11, 1), west, FRIDAY_16),
            at(2024, 5, 10, 21)
        );
    }

    #[test]
    fn invitation_opens_at_the_slot_and_closes_three_days_later() {
        let slot = at(2024, 5, 10, 16);
        let until = slot + INVITE_DAYS * DAY;
        assert_eq!(
            invite(slot, 0, FRIDAY_16, None),
            Invite::Open { slot, until }
        );
        assert_eq!(
            invite(until - 1, 0, FRIDAY_16, None),
            Invite::Open { slot, until }
        );
        assert_eq!(
            invite(until, 0, FRIDAY_16, None),
            Invite::Closed {
                next: slot + 7 * DAY
            }
        );
        assert_eq!(
            invite(slot - 1, 0, FRIDAY_16, None),
            Invite::Closed { next: slot }
        );
    }

    #[test]
    fn dismissed_slot_stays_closed_but_the_next_week_opens() {
        let slot = at(2024, 5, 10, 16);
        let now = slot + DAY;
        assert_eq!(
            invite(now, 0, FRIDAY_16, Some(slot)),
            Invite::Closed {
                next: slot + 7 * DAY
            }
        );
        // An older dismissal doesn't count.
        assert!(matches!(
            invite(now, 0, FRIDAY_16, Some(slot - 7 * DAY)),
            Invite::Open { .. }
        ));
        let next = slot + 7 * DAY;
        assert_eq!(
            invite(next, 0, FRIDAY_16, Some(slot)),
            Invite::Open {
                slot: next,
                until: next + INVITE_DAYS * DAY
            }
        );
    }

    #[test]
    fn a_review_started_after_the_slot_closes_it() {
        let slot = at(2024, 5, 10, 16);
        assert!(matches!(
            invite(slot + 3_600, 0, FRIDAY_16, Some(slot + 600)),
            Invite::Closed { .. }
        ));
        // A review well before the slot doesn't.
        assert!(matches!(
            invite(slot + 3_600, 0, FRIDAY_16, Some(slot - 2 * 3_600)),
            Invite::Open { .. }
        ));
    }

    #[test]
    fn a_dismissal_survives_the_end_of_daylight_saving() {
        // Dismissed on Friday 16:00 at UTC+2; on Monday the zone is UTC+1, so the
        // same local slot works out an hour later in UTC.
        let (summer, winter) = (2 * 3_600, 3_600);
        let dismissed = last_slot(at(2024, 10, 25, 15), summer, FRIDAY_16);
        assert_eq!(dismissed, at(2024, 10, 25, 14));
        let monday = at(2024, 10, 28, 9);
        assert_eq!(last_slot(monday, winter, FRIDAY_16), dismissed + 3_600);
        assert!(matches!(
            invite(monday, winter, FRIDAY_16, Some(dismissed)),
            Invite::Closed { .. }
        ));
    }

    #[test]
    fn schedule_reads_settings_with_fallbacks() {
        let store = temp_store("schedule");
        assert_eq!(schedule(&store), Some(Schedule::default()));
        store.set_setting(SET_DAY, "1").unwrap();
        store.set_setting(SET_HOUR, "9").unwrap();
        assert_eq!(
            schedule(&store),
            Some(Schedule {
                weekday: 1,
                hour: 9
            })
        );
        store.set_setting(SET_DAY, "9").unwrap();
        store.set_setting(SET_HOUR, "late").unwrap();
        assert_eq!(schedule(&store), Some(Schedule::default()));
        store.set_setting(SET_INVITE, "0").unwrap();
        assert_eq!(schedule(&store), None);
    }

    #[test]
    fn handled_is_the_later_of_dismissal_and_last_review() {
        let store = temp_store("handled");
        assert_eq!(store.last_review_at(), None);
        assert_eq!(handled(&store), None);
        store.set_setting(SET_INVITE_DISMISSED, "500").unwrap();
        assert_eq!(handled(&store), Some(500));
        store.begin_review(300).unwrap();
        store.begin_review(900).unwrap();
        assert_eq!(store.last_review_at(), Some(900));
        assert_eq!(handled(&store), Some(900));
        store.set_setting(SET_INVITE_DISMISSED, "1200").unwrap();
        assert_eq!(handled(&store), Some(1200));
    }

    #[test]
    fn invite_text_has_russian_plurals_and_minutes() {
        assert_eq!(invite_text(12), "Разобрать 12 заметок · ~3 мин");
        assert_eq!(invite_text(1), "Разобрать 1 заметку · ~1 мин");
        assert_eq!(invite_text(3), "Разобрать 3 заметки · ~1 мин");
        assert_eq!(invite_text(5), "Разобрать 5 заметок · ~2 мин");
        assert_eq!(invite_text(11), "Разобрать 11 заметок · ~3 мин");
        assert_eq!(invite_text(14), "Разобрать 14 заметок · ~4 мин");
        assert_eq!(invite_text(21), "Разобрать 21 заметку · ~6 мин");
        assert_eq!(invite_text(22), "Разобрать 22 заметки · ~6 мин");
        assert_eq!(invite_text(15), "Разобрать 15 заметок · ~4 мин");
    }
}
