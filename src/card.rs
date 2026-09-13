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

/// Where a copy of `of` goes: right against it, else below, left or above, the
/// first spot that is on the layer and free; else the first free slot.
pub fn beside(cards: &[Card], of: &Card, area: Vec2) -> Pos2 {
    let layer = egui::Rect::from_min_size(Pos2::ZERO, area);
    let (w, h) = (of.size.x, of.size.y);
    [vec2(w, 0.0), vec2(0.0, h), vec2(-w, 0.0), vec2(0.0, -h)]
        .into_iter()
        .map(|d| of.pos + d)
        .find(|&p| {
            // Shrunk a little: touching a neighbour's edge is fine.
            let slot = egui::Rect::from_min_size(p, of.size).shrink(0.5);
            layer.contains_rect(slot)
                && !cards
                    .iter()
                    .filter(|c| !c.archived)
                    .any(|c| egui::Rect::from_min_size(c.pos, c.size).intersects(slot))
        })
        .unwrap_or_else(|| free_slot(cards, area))
}

/// Space between cards that stick side by side: none, they sit edge to edge.
pub const SNAP_GAP: f32 = 0.0;
/// How close an edge has to come before it sticks.
pub const SNAP_DISTANCE: f32 = 10.0;
/// Cards at most this far apart (across the snapping axis) align their edges.
const ALIGN_REACH: f32 = 64.0;
/// Margins of the layer that edges stick to (the header takes the top).
const LAYER_MARGIN: egui::Margin = egui::Margin { left: 32, right: 32, top: 72, bottom: 32 };

/// A card rect after snapping, with a guide line per axis that stuck.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapped {
    pub rect: egui::Rect,
    pub guides: Vec<[Pos2; 2]>,
    pub x: bool,
    pub y: bool,
}

/// Best candidate on one axis: (offset to apply, guide line).
type Candidate = Option<(f32, [Pos2; 2])>;

fn consider(best: &mut Candidate, from: f32, to: f32, guide: impl FnOnce(f32) -> [Pos2; 2]) {
    let d = to - from;
    if d.abs() <= SNAP_DISTANCE && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
        *best = Some((d, guide((from + to) / 2.0)));
    }
}

/// Span of two ranges, for guide lines.
fn span(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0.min(b.0), a.1.max(b.1))
}

/// Candidates for moving edges; `edges` says which of left/right (or top/bottom)
/// are free to move. `x` picks the axis.
fn axis(rect: egui::Rect, others: &[egui::Rect], bounds: egui::Rect, x: bool, edges: (bool, bool)) -> Candidate {
    // (low edge, high edge) along the axis, and the range across it.
    let along = |r: egui::Rect| if x { (r.left(), r.right()) } else { (r.top(), r.bottom()) };
    let across = |r: egui::Rect| if x { (r.top(), r.bottom()) } else { (r.left(), r.right()) };
    let line = |at: f32, (a, b): (f32, f32)| if x { [pos2(at, a), pos2(at, b)] } else { [pos2(a, at), pos2(b, at)] };
    let (lo, hi) = along(rect);
    let cross = across(rect);
    let mut best = None;
    for &o in others {
        let (olo, ohi) = along(o);
        let ocross = across(o);
        let s = span(cross, ocross);
        let gap_across = (ocross.0 - cross.1).max(cross.0 - ocross.1);
        // Side by side: they overlap across the axis.
        if gap_across < 0.0 {
            if edges.0 {
                consider(&mut best, lo, ohi + SNAP_GAP, |at| line(at, s));
            }
            if edges.1 {
                consider(&mut best, hi, olo - SNAP_GAP, |at| line(at, s));
            }
        }
        // Aligned: one above the other (or near), same edge.
        if gap_across <= ALIGN_REACH {
            if edges.0 {
                consider(&mut best, lo, olo, |at| line(at, s));
            }
            if edges.1 {
                consider(&mut best, hi, ohi, |at| line(at, s));
            }
        }
    }
    let (blo, bhi) = if x {
        (bounds.left() + LAYER_MARGIN.left as f32, bounds.right() - LAYER_MARGIN.right as f32)
    } else {
        (bounds.top() + LAYER_MARGIN.top as f32, bounds.bottom() - LAYER_MARGIN.bottom as f32)
    };
    let whole = across(bounds);
    if edges.0 {
        consider(&mut best, lo, blo, |_| line(blo, whole));
    }
    if edges.1 {
        consider(&mut best, hi, bhi, |_| line(bhi, whole));
    }
    best
}

/// Sticks a dragged card to the edges of the others and of the layer: next to a
/// card (edge to edge, [`SNAP_GAP`]), or aligned with its edge.
pub fn snap_move(rect: egui::Rect, others: &[egui::Rect], bounds: egui::Rect) -> Snapped {
    let dx = axis(rect, others, bounds, true, (true, true));
    let dy = axis(rect, others, bounds, false, (true, true));
    let shift = vec2(dx.map_or(0.0, |d| d.0), dy.map_or(0.0, |d| d.0));
    // Guides were computed before the shift across the other axis; move them along.
    let guides = dx
        .map(|(_, g)| g.map(|p| p + vec2(0.0, shift.y)))
        .into_iter()
        .chain(dy.map(|(_, g)| g.map(|p| p + vec2(shift.x, 0.0))))
        .collect();
    Snapped { rect: rect.translate(shift), guides, x: dx.is_some(), y: dy.is_some() }
}

/// Which edges of a card a resize handle moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Sides {
    /// `start` with these edges moved by `delta`; the opposite edges stay, the
    /// size never drops below [`MIN_SIZE`] and moved edges stay on the layer.
    pub fn resize(self, start: egui::Rect, delta: Vec2, bounds: egui::Rect) -> egui::Rect {
        let mut r = start;
        if self.left {
            r.min.x = (r.min.x + delta.x).max(bounds.left()).min(r.max.x - MIN_SIZE.x);
        }
        if self.right {
            r.max.x = (r.max.x + delta.x).max(r.min.x + MIN_SIZE.x);
        }
        if self.top {
            r.min.y = (r.min.y + delta.y).max(bounds.top()).min(r.max.y - MIN_SIZE.y);
        }
        if self.bottom {
            r.max.y = (r.max.y + delta.y).max(r.min.y + MIN_SIZE.y);
        }
        r
    }

    pub fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon::*;
        match (self.left || self.right, self.top || self.bottom) {
            (true, false) => ResizeHorizontal,
            (false, true) => ResizeVertical,
            _ if (self.left && self.top) || (self.right && self.bottom) => ResizeNwSe,
            _ => ResizeNeSw,
        }
    }
}

/// Sticks the edges of a card being resized that the handle moves.
pub fn snap_resize(rect: egui::Rect, others: &[egui::Rect], bounds: egui::Rect, sides: Sides) -> Snapped {
    // Moving the low edge by d shrinks the rect by d; the high edge grows it.
    let fits = |c: Candidate, low: bool, len: f32, min: f32| c.filter(|(d, _)| if low { len - d } else { len + d } >= min);
    let dx = fits(axis(rect, others, bounds, true, (sides.left, sides.right)), sides.left, rect.width(), MIN_SIZE.x);
    let dy = fits(axis(rect, others, bounds, false, (sides.top, sides.bottom)), sides.top, rect.height(), MIN_SIZE.y);
    let mut out = rect;
    if let Some((d, _)) = dx {
        if sides.left { out.min.x += d } else { out.max.x += d }
    }
    if let Some((d, _)) = dy {
        if sides.top { out.min.y += d } else { out.max.y += d }
    }
    let guides = dx.map(|(_, g)| g).into_iter().chain(dy.map(|(_, g)| g)).collect();
    Snapped { rect: out, guides, x: dx.is_some(), y: dy.is_some() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    const BOUNDS: egui::Rect = egui::Rect { min: pos2(0.0, 0.0), max: pos2(2000.0, 1200.0) };

    #[test]
    fn snaps_beside_a_card_with_a_gap() {
        let other = r(400.0, 300.0, 280.0, 150.0);
        // Right of it, 6 pt too far.
        let s = snap_move(r(686.0 + SNAP_GAP, 330.0, 280.0, 150.0), &[other], BOUNDS);
        assert_eq!(s.rect.left(), 680.0 + SNAP_GAP);
        assert!(s.x);
        assert_eq!(s.guides.len(), 1);
    }

    #[test]
    fn aligns_edges_when_stacked() {
        let other = r(400.0, 300.0, 280.0, 150.0);
        // Below it, left edges 7 pt apart, top 5 pt from the gap.
        let s = snap_move(r(407.0, 455.0 + SNAP_GAP, 200.0, 100.0), &[other], BOUNDS);
        assert_eq!(s.rect.min, pos2(400.0, 450.0 + SNAP_GAP));
        assert!(s.x && s.y);
    }

    #[test]
    fn far_cards_and_free_space_do_not_stick() {
        let other = r(400.0, 300.0, 280.0, 150.0);
        let rect = r(900.0, 700.0, 280.0, 150.0);
        let s = snap_move(rect, &[other], BOUNDS);
        assert_eq!(s.rect, rect);
        assert!(s.guides.is_empty());
    }

    #[test]
    fn sticks_to_layer_margins() {
        let s = snap_move(r(36.0, 500.0, 280.0, 150.0), &[], BOUNDS);
        assert_eq!(s.rect.left(), 32.0);
    }

    #[test]
    fn resize_snaps_only_the_moving_edges_and_keeps_min_size() {
        let br = Sides { right: true, bottom: true, ..Default::default() };
        let other = r(700.0, 300.0, 280.0, 150.0);
        let s = snap_resize(r(400.0, 320.0, 290.0, 120.0), &[other], BOUNDS, br);
        assert_eq!(s.rect.min, pos2(400.0, 320.0));
        assert_eq!(s.rect.right(), 700.0 - SNAP_GAP);
        // Bottom aligns with the neighbour's.
        assert_eq!(s.rect.bottom(), 450.0);

        let tiny = r(400.0, 320.0, MIN_SIZE.x + 2.0, 120.0);
        let s = snap_resize(tiny, &[r(400.0 + MIN_SIZE.x + 2.0 + SNAP_GAP - 8.0, 300.0, 100.0, 150.0)], BOUNDS, br);
        assert!(s.rect.width() >= MIN_SIZE.x);
    }

    fn card_at(id: i64, x: f32, y: f32) -> Card {
        Card {
            id,
            kind: Kind::Note,
            title: String::new(),
            body: String::new(),
            tags: Vec::new(),
            pinned: false,
            archived: false,
            pos: pos2(x, y),
            size: DEFAULT_SIZE,
            created_at: 0,
        }
    }

    #[test]
    fn duplicate_goes_right_against_the_original_or_below() {
        let area = vec2(1920.0, 1080.0);
        let a = card_at(1, 400.0, 300.0);
        assert_eq!(beside(&[a.clone()], &a, area), pos2(400.0 + DEFAULT_SIZE.x, 300.0));
        // Right is taken: below.
        let b = card_at(2, 400.0 + DEFAULT_SIZE.x, 300.0);
        assert_eq!(beside(&[a.clone(), b], &a, area), pos2(400.0, 300.0 + DEFAULT_SIZE.y));
        // At the right edge of the layer: not off-screen.
        let edge = card_at(3, area.x - DEFAULT_SIZE.x, 300.0);
        assert_eq!(beside(&[edge.clone()], &edge, area), pos2(edge.pos.x, 300.0 + DEFAULT_SIZE.y));
    }

    #[test]
    fn resize_from_the_left_and_top_moves_those_edges() {
        let tl = Sides { left: true, top: true, ..Default::default() };
        let start = r(400.0, 400.0, 280.0, 150.0);
        let grown = tl.resize(start, vec2(-50.0, -30.0), BOUNDS);
        assert_eq!((grown.min, grown.max), (pos2(350.0, 370.0), start.max));
        // Can't shrink past the minimum: the far edges stay where they were.
        let shrunk = tl.resize(start, vec2(500.0, 500.0), BOUNDS);
        assert_eq!(shrunk.size(), MIN_SIZE);
        assert_eq!(shrunk.max, start.max);

        // Left edge 6 pt from a neighbour's right edge sticks to it.
        let left = Sides { left: true, ..Default::default() };
        let s = snap_resize(r(286.0, 400.0, 300.0, 150.0), &[r(0.0, 380.0, 280.0, 150.0)], BOUNDS, left);
        assert_eq!(s.rect.left(), 280.0);
        assert_eq!(s.rect.right(), 586.0);
    }

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
