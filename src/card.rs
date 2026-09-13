//! Card model and the quick-capture parser.

use egui::{Color32, Pos2, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Note,
    Idea,
    Prompt,
    Link,
    Goal,
    Reminder,
    Reference,
    Private,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Note,
        Kind::Idea,
        Kind::Prompt,
        Kind::Link,
        Kind::Goal,
        Kind::Reminder,
        Kind::Reference,
        Kind::Private,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Idea => "idea",
            Kind::Prompt => "prompt",
            Kind::Link => "link",
            Kind::Goal => "goal",
            Kind::Reminder => "reminder",
            Kind::Reference => "reference",
            Kind::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Kind {
        Kind::ALL.into_iter().find(|k| k.as_str() == s).unwrap_or(Kind::Note)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Note => "Заметка",
            Kind::Idea => "Идея",
            Kind::Prompt => "Prompt",
            Kind::Link => "Ссылка",
            Kind::Goal => "Цель",
            Kind::Reminder => "Напоминание",
            Kind::Reference => "Reference",
            Kind::Private => "Private",
        }
    }

    /// Segoe Fluent Icons glyph.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Note => "\u{E70B}",      // QuickNote
            Kind::Idea => "\u{EA80}",      // Lightbulb
            Kind::Prompt => "\u{E99A}",    // Robot
            Kind::Link => "\u{E71B}",      // Link
            Kind::Goal => "\u{E7C1}",      // Flag
            Kind::Reminder => "\u{E823}",  // Clock
            Kind::Reference => "\u{E943}", // Code
            Kind::Private => "\u{E72E}",   // Lock
        }
    }

    pub fn accent(self) -> Color32 {
        match self {
            Kind::Note => Color32::from_rgb(160, 174, 192),
            Kind::Idea => Color32::from_rgb(250, 204, 21),
            Kind::Prompt => Color32::from_rgb(167, 139, 250),
            Kind::Link => Color32::from_rgb(96, 165, 250),
            Kind::Goal => Color32::from_rgb(52, 211, 153),
            Kind::Reminder => Color32::from_rgb(251, 146, 60),
            Kind::Reference => Color32::from_rgb(45, 212, 191),
            Kind::Private => Color32::from_rgb(244, 114, 182),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Card {
    pub id: i64,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub archived: bool,
    pub pos: Pos2,
    pub size: Vec2,
    pub created_at: i64,
}

/// What a hidden Private card may show: its title, or else the first line when
/// more lines follow ("Wi-Fi офис" above the password). Single-line notes show
/// nothing, since that line may be the secret itself.
pub fn private_label(title: &str, body: &str) -> Option<String> {
    if !title.is_empty() {
        return Some(title.to_owned());
    }
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?;
    lines.next()?;
    Some(first.chars().take(60).collect())
}

pub const MIN_SIZE: Vec2 = vec2(200.0, 96.0);
pub const DEFAULT_SIZE: Vec2 = vec2(280.0, 150.0);

/// Result of parsing a quick-capture string.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}

const PREFIXES: &[(&str, Kind)] = &[
    ("идея:", Kind::Idea),
    ("idea:", Kind::Idea),
    ("промпт:", Kind::Prompt),
    ("prompt:", Kind::Prompt),
    ("цель:", Kind::Goal),
    ("goal:", Kind::Goal),
    ("ref:", Kind::Reference),
    ("private:", Kind::Private),
    ("секрет:", Kind::Private),
];

const REFERENCE_WORDS: &[&str] = &[
    "vpn", "ssh", "host", "server", "сервер", "endpoint", "path", "ip", "port", "порт",
];

pub fn parse_capture(input: &str) -> Parsed {
    let text = input.trim();
    let lower = text.to_lowercase();

    let mut kind = Kind::Note;
    let mut rest = text;
    for (prefix, k) in PREFIXES {
        if lower.starts_with(prefix) {
            kind = *k;
            // Prefixes are ASCII or Cyrillic; slice by char count to stay on a boundary.
            let n = prefix.chars().count();
            let byte = text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
            rest = text[byte..].trim_start();
            break;
        }
    }

    if kind == Kind::Note {
        let words: Vec<&str> = lower
            .split(|c: char| !c.is_alphanumeric() && c != '.' && c != ':' && c != '/')
            .filter(|w| !w.is_empty())
            .collect();
        if lower.contains("http://") || lower.contains("https://") {
            kind = Kind::Link;
        } else if is_reminder(&lower) {
            kind = Kind::Reminder;
        } else if words.iter().any(|w| REFERENCE_WORDS.contains(w)) {
            kind = Kind::Reference;
        }
    }

    let tags = rest
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('#'))
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_end_matches([',', '.', ';']).to_lowercase())
        .collect();

    // No automatic title: the first line is often not one, and splitting it off
    // changes how the note reads. The text is kept as written.
    Parsed {
        kind,
        title: String::new(),
        body: rest.to_owned(),
        tags,
    }
}

fn is_reminder(lower: &str) -> bool {
    if lower.starts_with("напомни") || lower.starts_with("remind") {
        return true;
    }
    const UNITS: &[&str] = &["дн", "день", "недел", "месяц", "час", "day", "week", "month"];
    let after = lower
        .split("через ")
        .nth(1)
        .or_else(|| lower.split("in ").nth(1));
    after.is_some_and(|a| a.split_whitespace().take(2).any(|w| UNITS.iter().any(|u| w.starts_with(u))))
}

/// Place a new card at the first free slot of a coarse grid.
pub fn free_slot(cards: &[Card], area: Vec2) -> Pos2 {
    let step = vec2(DEFAULT_SIZE.x + 16.0, DEFAULT_SIZE.y + 16.0);
    let origin = pos2(32.0, 72.0);
    let cols = ((area.x - origin.x) / step.x).floor().max(1.0) as usize;
    let rows = ((area.y - origin.y) / step.y).floor().max(1.0) as usize;
    for row in 0..rows {
        for col in 0..cols {
            let p = origin + vec2(col as f32 * step.x, row as f32 * step.y);
            let slot = egui::Rect::from_min_size(p, DEFAULT_SIZE);
            if !cards
                .iter()
                .filter(|c| !c.archived)
                .any(|c| egui::Rect::from_min_size(c.pos, c.size).intersects(slot))
            {
                return p;
            }
        }
    }
    origin + vec2(24.0, 24.0) * (cards.len() % 10) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kinds() {
        assert_eq!(parse_capture("идея: добавить weekly recap").kind, Kind::Idea);
        assert_eq!(parse_capture("идея: добавить weekly recap").body, "добавить weekly recap");
        assert_eq!(parse_capture("vpn staging vpn.staging.internal").kind, Kind::Reference);
        assert_eq!(
            parse_capture("через две недели проверить новую pricing модель").kind,
            Kind::Reminder
        );
        assert_eq!(parse_capture("https://egui.rs demo").kind, Kind::Link);
        assert_eq!(parse_capture("просто мысль").kind, Kind::Note);
    }

    #[test]
    fn private_label_never_shows_a_lone_line() {
        assert_eq!(private_label("", "Wi-Fi офис\nguest / pass").as_deref(), Some("Wi-Fi офис"));
        assert_eq!(private_label("", "hunter2"), None);
        assert_eq!(private_label("Title", "x").as_deref(), Some("Title"));
    }

    #[test]
    fn extracts_tags_and_keeps_text_as_written() {
        let p = parse_capture("Pricing\nпопробовать annual plan #product #pricing.");
        assert_eq!(p.title, "");
        assert_eq!(p.body, "Pricing\nпопробовать annual plan #product #pricing.");
        assert_eq!(p.tags, vec!["product", "pricing"]);
    }
}
