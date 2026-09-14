//! Deterministic, local-only selection of notes worth bringing back.

use crate::card::Kind;

pub const DAY: i64 = 86_400;
pub const COOLDOWN_DAYS: i64 = 14;

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

fn eligible(now: i64, c: &Candidate) -> bool {
    !c.deleted
        && c.archived
        && !c.pinned
        && c.kind != Kind::Private
        && !c.review_at.is_some_and(|at| at > now)
        && !c
            .last_resurfaced_at
            .is_some_and(|at| now - at < COOLDOWN_DAYS * DAY)
}

pub fn candidates(now: i64, input: &[Candidate], limit: usize) -> Vec<Pick> {
    let mut ranked: Vec<Pick> = input
        .iter()
        .filter(|c| eligible(now, c))
        .map(|c| {
            let forgotten = ((now - c.last_viewed_at).max(0) / DAY).min(60);
            let reminder = i64::from(c.review_at.is_some_and(|at| at <= now));
            let score = forgotten * 10 + type_weight(c.kind) + c.priority * 30 + reminder * 500
                - c.ignored_count * 20;
            let reason = if reminder != 0 {
                "Напоминание на сегодня".to_owned()
            } else if forgotten > 0 {
                format!("Ты записал это {forgotten} дн. назад")
            } else {
                "Давно не открывал".to_owned()
            };
            Pick {
                id: c.id,
                reason,
                score,
            }
        })
        .collect();
    ranked.sort_by_key(|p| (-p.score, p.id));
    ranked.truncate(limit);
    ranked
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Parses the small reminder grammar accepted by quick capture. It deliberately
/// stays local and predictable; richer natural-language dates belong to a later
/// capture pass.
pub fn review_at_from_text(text: &str, now: i64) -> Option<i64> {
    let lower = text.to_lowercase();
    let tail = lower
        .split("через ")
        .nth(1)
        .or_else(|| lower.split("in ").nth(1))?;
    let words: Vec<&str> = tail.split_whitespace().take(3).collect();
    let (amount, unit) = match words.as_slice() {
        [unit] => (1, *unit),
        [amount, unit, ..] => (
            match *amount {
                "один" | "одна" | "a" | "one" => 1,
                "два" | "две" | "two" => 2,
                "три" | "three" => 3,
                n => n.parse().ok()?,
            },
            *unit,
        ),
        _ => return None,
    };
    let seconds = if unit.starts_with("час") || unit.starts_with("hour") {
        amount * 3_600
    } else if unit.starts_with("дн") || unit.starts_with("day") {
        amount * DAY
    } else if unit.starts_with("недел") || unit.starts_with("week") {
        amount * 7 * DAY
    } else if unit.starts_with("месяц") || unit.starts_with("month") {
        amount * 30 * DAY
    } else {
        return None;
    };
    Some(now + seconds)
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
        recent.last_resurfaced_at = Some(now - 1 * DAY);
        let private = candidate(2, Kind::Private);
        assert!(candidates(now, &[recent, private], 5).is_empty());
    }

    #[test]
    fn snooze_hides_until_date_and_reminder_wins() {
        let now = 100 * DAY;
        let mut snoozed = candidate(1, Kind::Idea);
        snoozed.review_at = Some(now + DAY);
        let mut due = candidate(2, Kind::Note);
        due.review_at = Some(now);
        let picks = candidates(now, &[snoozed, due], 5);
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
    fn parses_capture_reminders() {
        assert_eq!(
            review_at_from_text("через две недели проверить pricing", 10),
            Some(10 + 14 * DAY)
        );
        assert_eq!(
            review_at_from_text("in 3 days review", 10),
            Some(10 + 3 * DAY)
        );
        assert_eq!(review_at_from_text("обычная заметка", 10), None);
    }
}
