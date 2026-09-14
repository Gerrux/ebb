//! Deterministic, local-only selection of notes worth bringing back.

use crate::card::Kind;

pub const DAY: i64 = 86_400;
pub const COOLDOWN_DAYS: i64 = 14;
/// Rediscover cards brought onto the layer a day.
pub const REDISCOVER_LIMIT: usize = 3;
/// A note archived or opened more recently than this isn't forgotten yet.
pub const FORGOTTEN_AFTER_DAYS: i64 = 7;

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub kind: Kind,
    pub created_at: i64,
    pub last_viewed_at: i64,
    pub review_at: Option<i64>,
    pub last_resurfaced_at: Option<i64>,
    pub ignored_count: i64,
    pub priority: i64,
    pub pinned: bool,
    pub archived: bool,
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub id: i64,
    pub reason: String,
    pub score: i64,
}

fn type_weight(kind: Kind) -> i64 {
    match kind {
        Kind::Idea => 100,
        Kind::Goal => 80,
        Kind::Reminder => 70,
        Kind::Note => 50,
        Kind::Prompt => 30,
        Kind::Reference => 20,
        Kind::Link => 20,
        Kind::Private => 0,
    }
}

fn due(now: i64, review_at: Option<i64>) -> bool {
    review_at.is_some_and(|at| at <= now)
}

fn eligible(now: i64, c: &Candidate) -> bool {
    !c.deleted
        && c.archived
        && !c.pinned
        && c.kind != Kind::Private
        && !c.review_at.is_some_and(|at| at > now)
        // A reminder that has come due skips both waits.
        && (due(now, c.review_at)
            || (now - c.last_viewed_at >= FORGOTTEN_AFTER_DAYS * DAY
                && !c.last_resurfaced_at.is_some_and(|at| now - at < COOLDOWN_DAYS * DAY)))
}

pub fn candidates(now: i64, input: &[Candidate], limit: usize) -> Vec<Pick> {
    let mut ranked: Vec<Pick> = input
        .iter()
        .filter(|c| eligible(now, c))
        .map(|c| {
            let forgotten = ((now - c.last_viewed_at).max(0) / DAY).min(60);
            let reminder = i64::from(due(now, c.review_at));
            let score = forgotten * 10 + type_weight(c.kind) + c.priority * 30 + reminder * 500 - c.ignored_count * 20;
            Pick { id: c.id, reason: reason(now, c.created_at, c.review_at), score }
        })
        .collect();
    ranked.sort_by_key(|p| (-p.score, p.id));
    ranked.truncate(limit);
    ranked
}

/// Why a note is back: "Напоминание на сегодня", "Ты записал это 73 дня назад".
pub fn reason(now: i64, created_at: i64, review_at: Option<i64>) -> String {
    if due(now, review_at) {
        return "Напоминание на сегодня".to_owned();
    }
    match (now - created_at).max(0) / DAY {
        0 => "Давно не открывал".to_owned(),
        n => format!("Ты записал это {n} {} назад", days_word(n)),
    }
}

/// "день" / "дня" / "дней" for `n`.
pub fn days_word(n: i64) -> &'static str {
    let n = n.abs();
    match (n % 10, n % 100) {
        (_, 11..=14) => "дней",
        (1, _) => "день",
        (2..=4, _) => "дня",
        _ => "дней",
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Parses the small reminder grammar accepted by quick capture: "через 2 недели",
/// "через неделю", "in 3 days", "in a month". It deliberately stays local and
/// predictable; richer natural-language dates belong to a later capture pass.
pub fn review_at_from_text(text: &str, now: i64) -> Option<i64> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    words.iter().enumerate().filter(|(_, w)| matches!(**w, "через" | "in")).find_map(|(i, _)| {
        let unit_seconds = |w: &str| {
            let w = w.trim_end_matches(|c: char| !c.is_alphanumeric());
            if ["час", "hour"].iter().any(|p| w.starts_with(p)) {
                Some(3_600)
            } else if ["дн", "ден", "day"].iter().any(|p| w.starts_with(p)) {
                Some(DAY)
            } else if ["недел", "week"].iter().any(|p| w.starts_with(p)) {
                Some(7 * DAY)
            } else if ["месяц", "month"].iter().any(|p| w.starts_with(p)) {
                Some(30 * DAY)
            } else {
                None
            }
        };
        let first = *words.get(i + 1)?;
        // "через неделю": the unit alone means one.
        if let Some(unit) = unit_seconds(first) {
            return Some(now + unit);
        }
        let amount: i64 = match first {
            "один" | "одна" | "одну" | "a" | "an" | "one" => 1,
            "два" | "две" | "two" => 2,
            "три" | "three" => 3,
            n => n.parse().ok().filter(|n| (1..=1000).contains(n))?,
        };
        Some(now + amount * unit_seconds(words.get(i + 2)?)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, kind: Kind) -> Candidate {
        Candidate {
            id,
            kind,
            created_at: 0,
            last_viewed_at: 0,
            review_at: None,
            last_resurfaced_at: None,
            ignored_count: 0,
            priority: 0,
            pinned: false,
            archived: true,
            deleted: false,
        }
    }

    #[test]
    fn cooldown_and_private_are_excluded() {
        let now = 100 * DAY;
        let mut recent = candidate(1, Kind::Idea);
        recent.last_resurfaced_at = Some(now - DAY);
        let private = candidate(2, Kind::Private);
        assert!(candidates(now, &[recent, private], 5).is_empty());
    }

    #[test]
    fn just_archived_notes_are_not_forgotten_yet() {
        let now = 100 * DAY;
        let mut fresh = candidate(1, Kind::Idea);
        fresh.last_viewed_at = now - DAY;
        assert!(candidates(now, &[fresh.clone()], 5).is_empty());
        fresh.review_at = Some(now);
        assert_eq!(candidates(now, &[fresh], 5).len(), 1, "a due reminder comes back anyway");
    }

    #[test]
    fn snooze_hides_until_date_and_reminder_wins() {
        let now = 100 * DAY;
        let mut snoozed = candidate(1, Kind::Idea);
        snoozed.review_at = Some(now + DAY);
        let mut due = candidate(2, Kind::Note);
        due.review_at = Some(now);
        let picks = candidates(now, &[snoozed, due], 5);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].id, 2);
        assert_eq!(picks[0].reason, "Напоминание на сегодня");
    }

    #[test]
    fn pinned_and_live_cards_are_not_candidates() {
        let now = 100 * DAY;
        let mut pinned = candidate(1, Kind::Idea);
        pinned.pinned = true;
        let mut live = candidate(2, Kind::Idea);
        live.archived = false;
        assert!(candidates(now, &[pinned, live], 5).is_empty());
    }

    #[test]
    fn ranking_prefers_ideas_and_is_deterministic() {
        let now = 100 * DAY;
        let mut note = candidate(2, Kind::Note);
        note.last_viewed_at = 90 * DAY;
        let mut idea = candidate(1, Kind::Idea);
        idea.last_viewed_at = 90 * DAY;
        assert_eq!(candidates(now, &[note.clone(), idea.clone()], 2)[0].id, 1);
        assert_eq!(candidates(now, &[idea, note], 2)[0].id, 1);
    }

    #[test]
    fn reasons_decline_days() {
        assert_eq!(reason(73 * DAY, 0, None), "Ты записал это 73 дня назад");
        assert_eq!(reason(21 * DAY, 0, None), "Ты записал это 21 день назад");
        assert_eq!(reason(11 * DAY, 0, None), "Ты записал это 11 дней назад");
        assert_eq!(reason(5, 0, Some(1)), "Напоминание на сегодня");
    }

    #[test]
    fn parses_capture_reminders() {
        assert_eq!(review_at_from_text("через две недели проверить pricing", 10), Some(10 + 14 * DAY));
        assert_eq!(review_at_from_text("Через неделю проверить", 10), Some(10 + 7 * DAY));
        assert_eq!(review_at_from_text("позвонить через 3 дня", 10), Some(10 + 3 * DAY));
        assert_eq!(review_at_from_text("in 3 days review", 10), Some(10 + 3 * DAY));
        assert_eq!(review_at_from_text("check in a month.", 10), Some(10 + 30 * DAY));
        assert_eq!(review_at_from_text("обычная заметка", 10), None);
        // "in" inside a word, or not followed by a duration, isn't a reminder.
        assert_eq!(review_at_from_text("domain 3 days", 10), None);
        assert_eq!(review_at_from_text("in the office", 10), None);
    }
}
