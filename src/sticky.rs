//! Import from Windows Sticky Notes (`plum.sqlite`).
//!
//! The original database is never opened: Sticky Notes may be running and writing
//! to it. The main file, `-wal` and `-shm` are copied into a temp directory
//! (retrying if they change mid-copy) and only the copy is opened, which lets
//! SQLite replay the WAL into the copy — notes that exist only in the WAL are
//! imported too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{Connection, OpenFlags};

use crate::card::{Kind, Tint, looks_secret, parse_capture};

/// .NET ticks (100 ns since 0001-01-01) at the Unix epoch.
const TICKS_AT_UNIX_EPOCH: i64 = 621_355_968_000_000_000;
/// Closed notes not edited for this long are counted as old in the summary.
pub const OLD_AFTER_DAYS: i64 = 365;

pub fn plum_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let path = Path::new(&base)
        .join(r"Packages\Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe\LocalState\plum.sqlite");
    path.is_file().then_some(path)
}

pub fn ticks_to_unix(ticks: i64) -> i64 {
    (ticks - TICKS_AT_UNIX_EPOCH) / 10_000_000
}

/// Where a Sticky Notes window was: `ManagedPosition=DeviceId:<path>;Position=x,y;Size=w,h`.
/// Coordinates are pixels relative to that monitor's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowPos {
    pub device: String,
    pub pos: (i32, i32),
    pub size: (i32, i32),
}

pub fn parse_window_position(s: &str) -> Option<WindowPos> {
    let mut device = None;
    let mut pos = None;
    let mut size = None;
    let pair = |v: &str| {
        let (a, b) = v.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    };
    for part in s.split(';') {
        let (key, value) = part.split_once('=')?;
        match key.trim() {
            "ManagedPosition" => device = value.strip_prefix("DeviceId:").map(str::to_owned),
            "Position" => pos = pair(value),
            "Size" => size = pair(value),
            _ => {}
        }
    }
    Some(WindowPos { device: device?, pos: pos?, size: size? })
}

#[derive(Clone, Debug)]
pub struct SourceNote {
    pub id: String,
    pub text: String,
    pub is_open: bool,
    pub window: Option<WindowPos>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The note's color in Sticky Notes: Yellow, Green, Pink, Purple, Blue, Gray, Charcoal.
    pub theme: Option<String>,
}

/// Sticky Notes' default color; a set of notes all in it never picked a color.
const DEFAULT_THEME: &str = "Yellow";

/// The card color for a Sticky Notes color. Charcoal (the dark note) is the
/// neutral one here: on the dark glass it reads as "no color".
pub fn tint_from_theme(theme: &str) -> Option<Tint> {
    match theme.trim() {
        "Yellow" => Some(Tint::Yellow),
        "Green" => Some(Tint::Green),
        "Pink" => Some(Tint::Pink),
        "Purple" => Some(Tint::Purple),
        "Blue" => Some(Tint::Blue),
        "Gray" | "Charcoal" => Some(Tint::Gray),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Reading a copy
// ---------------------------------------------------------------------------

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Copies `plum.sqlite` with its `-wal`/`-shm` into `dir`. Retries when any of the
/// files changed while copying (Sticky Notes writing), so the set is consistent.
fn copy_consistent(src: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let dst = dir.join("plum.sqlite");
    let files = ["", "-wal", "-shm"];
    let mut last_err = None;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
        }
        let before: Vec<_> = files.iter().map(|s| stamp(&sidecar(src, s))).collect();
        let mut ok = true;
        for s in files {
            let from = sidecar(src, s);
            let to = sidecar(&dst, s);
            let _ = std::fs::remove_file(&to);
            if !from.exists() {
                continue; // no WAL: the main file is already complete
            }
            // Not fs::copy: CopyFileEx doesn't share write access, and Sticky Notes
            // keeps the database open for writing. File::open shares read/write/delete.
            let copied = std::fs::File::open(&from)
                .and_then(|mut src| std::io::copy(&mut src, &mut std::fs::File::create(&to)?));
            if let Err(e) = copied {
                last_err = Some(e);
                ok = false;
                break;
            }
        }
        let after: Vec<_> = files.iter().map(|s| stamp(&sidecar(src, s))).collect();
        if ok && before == after {
            return Ok(dst);
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("plum.sqlite kept changing while copying")))
}

/// Reads all non-deleted notes from a private copy of `src`.
pub fn read_notes(src: &Path) -> Result<Vec<SourceNote>, String> {
    let dir = std::env::temp_dir().join(format!("ebb-sticky-{}", std::process::id()));
    let result = (|| {
        let copy = copy_consistent(src, &dir).map_err(|e| format!("copy: {e}"))?;
        // Read-write on the copy so SQLite can replay the WAL into it.
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| format!("open copy: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT Id, Text, IsOpen, CreatedAt, UpdatedAt, WindowPosition, Theme FROM Note WHERE DeletedAt IS NULL")
            .map_err(|e| format!("query: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceNote {
                    id: r.get(0)?,
                    text: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    is_open: r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                    window: r.get::<_, Option<String>>(5)?.as_deref().and_then(parse_window_position),
                    created_at: ticks_to_unix(r.get::<_, Option<i64>>(3)?.unwrap_or(TICKS_AT_UNIX_EPOCH)),
                    updated_at: ticks_to_unix(r.get::<_, Option<i64>>(4)?.unwrap_or(TICKS_AT_UNIX_EPOCH)),
                    theme: r.get::<_, Option<String>>(6)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("row: {e}"))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Converts Sticky Notes' RTF-like markup to plain text. Each paragraph starts
/// with `\id=<guid> `; formatting is `\b`, `\b0`, `\strike`, `\ul`, ... followed by
/// one optional space; a literal backslash is written as `\\`.
pub fn plain_text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut chars = markup.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            Some(p) if p.is_ascii_alphabetic() => {
                let mut word = String::new();
                while let Some(&p) = chars.peek().filter(|p| p.is_ascii_alphabetic()) {
                    word.push(p);
                    chars.next();
                }
                if word == "id" && chars.peek() == Some(&'=') {
                    // Paragraph id: skip up to and including the following space.
                    while let Some(p) = chars.next() {
                        if p == ' ' || p == '\n' {
                            if p == '\n' {
                                out.push('\n');
                            }
                            break;
                        }
                    }
                    continue;
                }
                while chars.peek().is_some_and(|p| p.is_ascii_digit() || *p == '-') {
                    chars.next();
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            _ => out.push('\\'),
        }
    }
    let lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    lines.join("\n").trim().to_owned()
}

/// A note's kind from its text alone, locally (spec 08). Checked in order:
/// credentials, an explicit prefix, prompt, link, reference, idea; else a note.
/// Tuned on `tests/fixtures/sticky-classify.txt`.
pub fn classify(plain: &str) -> Kind {
    if looks_secret(plain) || env_secret(plain) || pin_code(plain) {
        return Kind::Private;
    }
    // An explicit "идея:", "prompt:"… wins (parse_capture checks prefixes first).
    let first_line = plain.lines().next().unwrap_or("");
    match parse_capture(first_line).kind {
        kind @ (Kind::Idea | Kind::Prompt | Kind::Goal | Kind::Reference | Kind::Private) if has_prefix(first_line) => return kind,
        _ => {}
    }
    if looks_prompt(plain) {
        return Kind::Prompt;
    }
    if looks_link(plain) {
        return Kind::Link;
    }
    if looks_reference(plain) {
        return Kind::Reference;
    }
    if looks_idea(plain) {
        return Kind::Idea;
    }
    Kind::Note
}

/// Groups of the import review (spec 08), in the order the screen lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportGroup {
    Secrets,
    Links,
    Prompts,
    Ideas,
    Reference,
    /// Plain notes not changed for over a year.
    Old,
    Other,
}

impl ImportGroup {
    pub const ALL: [Self; 7] = [Self::Secrets, Self::Links, Self::Prompts, Self::Ideas, Self::Reference, Self::Old, Self::Other];

    pub fn label(self) -> &'static str {
        match self {
            Self::Secrets => "Похоже на пароли и доступы",
            Self::Links => "Ссылки",
            Self::Prompts => "Промпты",
            Self::Ideas => "Идеи",
            Self::Reference => "Справка: команды, хосты, пути",
            Self::Old => "Давно не менялись",
            Self::Other => "Остальное",
        }
    }

    /// The kind "accept" gives the group's cards; none for groups that aren't a kind.
    pub fn kind(self) -> Option<Kind> {
        match self {
            Self::Secrets => Some(Kind::Private),
            Self::Links => Some(Kind::Link),
            Self::Prompts => Some(Kind::Prompt),
            Self::Ideas => Some(Kind::Idea),
            Self::Reference => Some(Kind::Reference),
            Self::Old | Self::Other => None,
        }
    }
}

/// Which review group an imported card falls in, from its text as it is now:
/// the suggestion follows edits and never goes stale. A card already Private
/// has no text to read and stays with the secrets.
pub fn import_group(kind: Kind, text: &str, updated_at: i64, now: i64) -> ImportGroup {
    let suggested = if kind == Kind::Private { Kind::Private } else { classify(text) };
    match suggested {
        Kind::Private => ImportGroup::Secrets,
        Kind::Link => ImportGroup::Links,
        Kind::Prompt => ImportGroup::Prompts,
        Kind::Idea => ImportGroup::Ideas,
        Kind::Reference => ImportGroup::Reference,
        _ if now - updated_at > OLD_AFTER_DAYS * 86_400 => ImportGroup::Old,
        _ => ImportGroup::Other,
    }
}

fn has_prefix(line: &str) -> bool {
    let lower = line.trim_start().to_lowercase();
    ["идея:", "idea:", "промпт:", "prompt:", "цель:", "goal:", "ref:", "private:", "секрет:"].iter().any(|p| lower.starts_with(p))
}

/// `AWS_SECRET_ACCESS_KEY=…`, `DB_PASSWORD=…`: an env line naming a credential.
fn env_secret(plain: &str) -> bool {
    plain.lines().any(|line| {
        let Some((key, value)) = line.trim().split_once('=') else { return false };
        let key = key.trim().to_ascii_uppercase();
        key.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            && ["SECRET", "PASSWORD", "PASSWD", "TOKEN", "API_KEY", "PRIVATE_KEY"].iter().any(|w| key.contains(w))
            && value.trim().chars().count() >= 6
    })
}

/// "PIN карты 4829": a PIN with its digits a word or two later.
fn pin_code(plain: &str) -> bool {
    let words: Vec<String> = plain.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).collect();
    words.iter().enumerate().any(|(i, w)| {
        matches!(w.as_str(), "pin" | "пин" | "пин-код" | "пинкод")
            && words[i + 1..].iter().take(2).any(|v| (4..=8).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn looks_prompt(plain: &str) -> bool {
    let lower = plain.to_lowercase();
    const STARTS: &[&str] = &[
        "ты —", "ты -", "ты это", "you are", "act as", "представь, что", "представь что", "выступи в роли", "действуй как",
        "объясни", "переведи", "перепиши", "сократи", "write a", "rewrite", "summarize", "explain",
    ];
    // Addressing a model, anywhere in a longer text.
    const PHRASES: &[&str] = &[
        "твоя задача", "формат ответа", "ответь ", "ответь:", "your task", "respond with", "answer in", "act as", "you are a",
    ];
    let starts = STARTS.iter().any(|p| lower.starts_with(p));
    let long_and_addressed = plain.chars().count() > 300 && PHRASES.iter().filter(|p| lower.contains(*p)).count() >= 1;
    // Template markers: {{variable}}, "### Section", "Role:"/"Task:" lines.
    let placeholders = !crate::card::prompt_placeholders(plain).is_empty();
    let sections = plain.lines().filter(|l| l.trim_start().starts_with("###")).count() >= 2
        || plain.lines().any(|l| {
            let l = l.trim_start().to_lowercase();
            ["role:", "роль:", "system:"].iter().any(|p| l.starts_with(p))
        });
    starts || long_and_addressed || placeholders || sections
}

/// At least half of the text is URLs.
fn looks_link(plain: &str) -> bool {
    let total: usize = plain.split_whitespace().map(|w| w.chars().count()).sum();
    let urls: usize = plain
        .split_whitespace()
        // Not `REDIS_URL=redis://…`: that's a setting.
        .filter(|w| (w.contains("://") || w.starts_with("www.")) && !w.split("://").next().unwrap_or("").contains('='))
        .map(|w| w.chars().count())
        .sum();
    total > 0 && urls * 2 >= total
}

/// Commands, hosts, paths, `KEY=value`: two such lines, or a short note that is one.
fn looks_reference(plain: &str) -> bool {
    let lines: Vec<&str> = plain.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let env = |l: &str| {
        l.split_once('=').is_some_and(|(k, v)| {
            !k.is_empty() && !v.is_empty() && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
    };
    // "Прокси: proxy.corp.example.com:3128": the label doesn't hide the host.
    let technical = |l: &str| {
        let value = l.split_once(": ").map_or(l, |(_, v)| v);
        crate::card::looks_technical(l) || crate::card::looks_technical(value) || env(l)
    };
    let count = lines.iter().filter(|l| technical(l)).count();
    count >= 2 || (count >= 1 && lines.len() <= 2 && plain.chars().count() <= 120)
}

fn looks_idea(plain: &str) -> bool {
    let first = plain.lines().next().unwrap_or("").trim();
    let lower = first.to_lowercase();
    const STARTS: &[&str] = &["а что если", "что если", "а если", "попробовать", "можно сделать", "what if", "try "];
    let named = lower.split(|c: char| !c.is_alphanumeric()).any(|w| matches!(w, "идея" | "идеи" | "idea"));
    // A short question to oneself, not a question about something that happened.
    let question = first.ends_with('?') && first.chars().count() <= 100 && plain.lines().count() == 1
        && ["что если", "а если", "может", "what if", "should"].iter().any(|p| lower.contains(p));
    named || question || STARTS.iter().any(|p| lower.starts_with(p))
}

fn tags(plain: &str) -> Vec<String> {
    let mut tags: Vec<String> = plain
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('#'))
        .map(|t| t.trim_end_matches([',', '.', ';', ':', ')']).to_lowercase())
        .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'))
        .collect();
    tags.push("sticky".into());
    tags.dedup();
    tags
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Planned {
    pub source_id: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Open in Sticky Notes: goes onto the layer, placed by [`layout`].
    pub on_layer: bool,
    pub window: Option<WindowPos>,
    pub old: bool,
    /// The note's Sticky Notes color, when colors were used at all (see [`plan`]).
    pub tint: Option<Tint>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub total: usize,
    pub importable: usize,
    pub on_layer: usize,
    pub archived: usize,
    pub old: usize,
    pub empty: usize,
    pub duplicates: usize,
    pub already_imported: usize,
    pub by_kind: Vec<(Kind, usize)>,
    /// Notes that keep their Sticky Notes color.
    pub colored: usize,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub notes: Vec<Planned>,
    pub stats: Stats,
}

/// Decides what to import. Notes open in Sticky Notes go onto the layer (all of
/// them, as on screen); the rest go to the archive. `already` holds source ids
/// that must not be imported again (kept from an earlier import).
pub fn plan(notes: &[SourceNote], already: &std::collections::HashSet<String>, now: i64) -> Plan {
    let mut stats = Stats { total: notes.len(), ..Default::default() };

    // Open notes first, then newest: the kept copy of a duplicate is the one on
    // screen, or else the most recently edited.
    let mut sorted: Vec<&SourceNote> = notes.iter().collect();
    sorted.sort_by_key(|n| (std::cmp::Reverse(n.is_open), std::cmp::Reverse(n.updated_at)));

    // Colors carry over only when the person used them: with every note in the
    // default yellow, the color says nothing, and yellow on every card would
    // hide the kinds' own colors.
    let uses_colors = notes.iter().any(|n| n.theme.as_deref().is_some_and(|t| t.trim() != DEFAULT_THEME));

    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut planned = Vec::new();
    for note in sorted {
        if already.contains(&note.id) {
            stats.already_imported += 1;
            continue;
        }
        let plain = plain_text(&note.text);
        if plain.is_empty() {
            stats.empty += 1;
            continue;
        }
        let key = plain.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if seen.insert(key, ()).is_some() {
            stats.duplicates += 1;
            continue;
        }
        let kind = classify(&plain);
        planned.push(Planned {
            source_id: note.id.clone(),
            kind,
            // Text as written, no title split (see card::parse_capture).
            title: String::new(),
            body: plain.clone(),
            tags: tags(&plain),
            created_at: note.created_at,
            updated_at: note.updated_at,
            on_layer: note.is_open,
            window: note.window.clone(),
            old: !note.is_open && now - note.updated_at > OLD_AFTER_DAYS * 86_400,
            tint: if uses_colors { note.theme.as_deref().and_then(tint_from_theme) } else { None },
        });
    }

    stats.importable = planned.len();
    stats.on_layer = planned.iter().filter(|p| p.on_layer).count();
    stats.archived = planned.len() - stats.on_layer;
    stats.old = planned.iter().filter(|p| p.old).count();
    stats.colored = planned.iter().filter(|p| p.tint.is_some()).count();
    stats.by_kind = Kind::ALL
        .iter()
        .map(|k| (*k, planned.iter().filter(|p| p.kind == *k).count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    Plan { notes: planned, stats }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Screen geometry for placing imported notes on the layer.
pub struct Screen<'a> {
    /// The monitor the layer covers.
    pub layer: Option<&'a crate::win::Monitor>,
    /// Physical pixels per layer point.
    pub pixels_per_point: f32,
    /// Layer size in points.
    pub area: egui::Vec2,
}

const HEADER_CLEAR: f32 = 64.0;
const GAP: f32 = 8.0;

/// First free spot (row by row, on the 8 pt grid) for a card of `size`, keeping a
/// gap to `taken`.
pub fn find_free(taken: &[egui::Rect], size: egui::Vec2, area: egui::Vec2) -> Option<egui::Pos2> {
    let step = 8.0;
    let mut y = HEADER_CLEAR;
    while y + size.y <= area.y {
        let mut x = 24.0;
        while x + size.x <= area.x {
            let r = egui::Rect::from_min_size(egui::pos2(x, y), size);
            if !taken.iter().any(|t| t.expand(GAP).intersects(r)) {
                return Some(r.min);
            }
            x += step;
        }
        y += step;
    }
    None
}

/// A spot for a card that would like `size`: at that size if it fits anywhere,
/// else smaller (default, then minimum card size), else cascaded as a last resort.
pub fn place(taken: &[egui::Rect], size: egui::Vec2, area: egui::Vec2) -> egui::Rect {
    use crate::card::{DEFAULT_SIZE, MIN_SIZE};
    for candidate in [size, DEFAULT_SIZE.min(size), MIN_SIZE] {
        if let Some(pos) = find_free(taken, candidate, area) {
            return egui::Rect::from_min_size(pos, candidate);
        }
    }
    let n = taken.len() as f32 % 12.0;
    egui::Rect::from_min_size(egui::pos2(40.0 + n * 24.0, HEADER_CLEAR + 16.0 + n * 24.0), size)
}

/// Rectangles (layer points) for the notes that go onto the layer, `None` for the
/// rest. Notes that were on the layer's monitor keep their position and size;
/// notes from other monitors keep their size and take the first free spot.
pub fn layout(notes: &[Planned], screen: &Screen, occupied: &[egui::Rect]) -> Vec<Option<egui::Rect>> {
    use crate::card::{DEFAULT_SIZE, MIN_SIZE};

    let ppp = screen.pixels_per_point.max(0.5);
    let fit = |size: egui::Vec2| size.max(MIN_SIZE).min(screen.area);
    let mut out: Vec<Option<egui::Rect>> = vec![None; notes.len()];
    let mut taken: Vec<egui::Rect> = occupied.to_vec();

    // Exact placements first, so free-spot search avoids them.
    for (i, n) in notes.iter().enumerate().filter(|(_, n)| n.on_layer) {
        let (Some(w), Some(layer)) = (&n.window, screen.layer) else { continue };
        if !layer.matches_device(&w.device) {
            continue;
        }
        let size = fit(egui::vec2(w.size.0 as f32, w.size.1 as f32) / ppp);
        // Position is relative to the monitor; the layer starts at its work area.
        let x = (w.pos.0 + layer.rect.left - layer.work.left) as f32 / ppp;
        let y = (w.pos.1 + layer.rect.top - layer.work.top) as f32 / ppp;
        let min = egui::pos2(x.clamp(0.0, (screen.area.x - size.x).max(0.0)), y.clamp(0.0, (screen.area.y - size.y).max(0.0)));
        let r = egui::Rect::from_min_size(min, size);
        out[i] = Some(r);
        taken.push(r);
    }
    for (i, n) in notes.iter().enumerate().filter(|(_, n)| n.on_layer) {
        if out[i].is_some() {
            continue;
        }
        let size = match &n.window {
            Some(w) => {
                // Scale by the source monitor's DPI? Positions are stored in that
                // monitor's pixels; without its DPI, the layer's is the best guess.
                fit(egui::vec2(w.size.0 as f32, w.size.1 as f32) / ppp)
            }
            None => DEFAULT_SIZE,
        };
        let r = place(&taken, size, screen.area);
        out[i] = Some(r);
        taken.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b1e2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    #[test]
    fn strips_paragraph_ids_and_formatting() {
        let src = format!("\\id={ID} \\b Заголовок\\b0 \n\\id={ID} путь C:\\\\Users\\\\me\n\\id={ID} \\strike готово\\strike0 ");
        assert_eq!(plain_text(&src), "Заголовок\nпуть C:\\Users\\me\nготово");
    }

    #[test]
    fn keeps_empty_paragraphs_as_blank_lines() {
        let src = format!("\\id={ID} a\n\\id={ID} \n\\id={ID} b");
        assert_eq!(plain_text(&src), "a\n\nb");
    }

    #[test]
    fn converts_ticks() {
        // 2024-03-01T16:44:58Z
        assert_eq!(ticks_to_unix(638_449_082_980_000_000), 1_709_311_498);
    }

    /// Labeled synthetic notes in the `tests/fixtures/sticky-classify*.txt` format.
    fn fixture(source: &str) -> Vec<(Kind, String)> {
        let mut notes: Vec<(Kind, String)> = Vec::new();
        for line in source.lines() {
            if let Some(kind) = line.strip_prefix("=== ") {
                notes.push((Kind::parse(kind.trim()), String::new()));
            } else if let Some((_, text)) = notes.last_mut() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(line);
            }
        }
        notes
    }

    /// `cargo test --release classifies_the_fixture -- --nocapture` prints the misses.
    #[test]
    fn classifies_the_fixture() {
        let notes = fixture(include_str!("../tests/fixtures/sticky-classify.txt"));
        assert_eq!(notes.len(), 100);
        let (mut misses, mut below) = (0, Vec::new());
        for kind in [Kind::Note, Kind::Idea, Kind::Link, Kind::Prompt, Kind::Reference, Kind::Private] {
            let labeled: Vec<&String> = notes.iter().filter(|(k, _)| *k == kind).map(|(_, t)| t).collect();
            let found = labeled.iter().filter(|t| classify(t) == kind).count();
            let predicted = notes.iter().filter(|(_, t)| classify(t) == kind).count();
            let recall = found as f64 / labeled.len() as f64;
            let precision = if predicted == 0 { 0.0 } else { found as f64 / predicted as f64 };
            println!("{:>10}: recall {:.0}% ({found}/{}), precision {:.0}%", kind.as_str(), recall * 100.0, labeled.len(), precision * 100.0);
            for (k, text) in notes.iter().filter(|(k, t)| (*k == kind) != (classify(t) == kind)) {
                if *k == kind {
                    misses += 1;
                    println!("            miss as {}: {}", classify(text).as_str(), text.replace('\n', " / "));
                }
            }
            if recall < 0.8 || precision < 0.8 {
                below.push(kind.as_str());
            }
        }
        println!("misses: {misses}/100");
        assert!(below.is_empty(), "below 80%: {below:?}");
    }

    /// Notes the rules were not tuned on: overall accuracy only, groups are too small.
    #[test]
    fn classifies_held_out_notes() {
        let notes = fixture(include_str!("../tests/fixtures/sticky-classify-holdout.txt"));
        let misses: Vec<String> = notes
            .iter()
            .filter(|(k, t)| classify(t) != *k)
            .map(|(k, t)| format!("{} as {}: {}", k.as_str(), classify(t).as_str(), t.replace('\n', " / ")))
            .collect();
        println!("held-out misses {}/{}:\n{}", misses.len(), notes.len(), misses.join("\n"));
        assert!(misses.len() * 5 <= notes.len(), "held-out accuracy below 80%");
    }

    #[test]
    fn detects_secrets_without_flagging_llm_tokens() {
        assert!(looks_secret("wifi\nпароль qwerty123"));
        assert!(looks_secret("key sk-abcdefghijklmnopqrstuvwxyz123"));
        assert!(!looks_secret("prompt limit 8k tokens, be concise"));
    }

    fn note(id: &str, text: &str, open: bool, updated_days_ago: i64) -> SourceNote {
        let now = 2_000_000_000;
        SourceNote {
            id: id.into(),
            text: format!("\\id={ID} {text}"),
            is_open: open,
            window: None,
            created_at: now - updated_days_ago * 86_400,
            updated_at: now - updated_days_ago * 86_400,
            theme: None,
        }
    }

    #[test]
    fn colors_carry_over_only_when_they_were_used() {
        let mut a = note("a", "жёлтая", true, 1);
        a.theme = Some("Yellow".into());
        let mut b = note("b", "серая", true, 1);
        b.theme = Some("Charcoal".into());
        let mut c = note("c", "без цвета", true, 1);
        c.theme = None;
        let tints = |notes: &[SourceNote]| -> Vec<Option<Tint>> {
            let mut p = plan(notes, &Default::default(), 2_000_000_000).notes;
            p.sort_by(|x, y| x.source_id.cmp(&y.source_id));
            p.iter().map(|n| n.tint).collect()
        };
        // All default yellow: nobody picked a color.
        assert_eq!(tints(&[a.clone(), c.clone()]), [None, None]);
        // One other color: every color counts, yellow included.
        assert_eq!(tints(&[a.clone(), b.clone(), c.clone()]), [Some(Tint::Yellow), Some(Tint::Gray), None]);
        assert_eq!(plan(&[a, b, c], &Default::default(), 2_000_000_000).stats.colored, 2);
        assert_eq!(tint_from_theme("Green"), Some(Tint::Green));
        assert_eq!(tint_from_theme("Magenta"), None);
    }

    #[test]
    fn plans_layer_archive_duplicates_and_reimport() {
        let notes = vec![
            note("a", "идея: weekly recap", false, 3),
            note("b", "идея:  Weekly   recap", true, 10), // duplicate of a, but open: kept
            note("c", "", true, 1),                       // empty
            note("d", "старая заметка", true, 800),       // open, however old: on the layer
            note("e", "https://example.com", false, 900), // closed and old
            note("f", "wifi пароль 123", true, 2),
        ];
        let now = 2_000_000_000;
        let p = plan(&notes, &Default::default(), now);
        assert_eq!(p.stats.duplicates, 1);
        assert_eq!(p.stats.empty, 1);
        assert_eq!(p.stats.importable, 4);
        assert_eq!(p.stats.old, 1);
        let mut on_layer: Vec<_> = p.notes.iter().filter(|n| n.on_layer).map(|n| n.source_id.as_str()).collect();
        on_layer.sort();
        assert_eq!(on_layer, ["b", "d", "f"]);
        assert_eq!(p.notes.iter().find(|n| n.source_id == "f").unwrap().kind, Kind::Private);
        assert_eq!(p.notes.iter().find(|n| n.source_id == "e").unwrap().kind, Kind::Link);

        let already = ["b".to_owned(), "f".to_owned()].into_iter().collect();
        let again = plan(&notes, &already, now);
        assert_eq!(again.stats.already_imported, 2);
        assert!(again.notes.iter().all(|n| n.source_id != "b" && n.source_id != "f"));
    }

    #[test]
    fn parses_window_position() {
        let w = parse_window_position(r"ManagedPosition=DeviceId:\\?\DISPLAY#XMI27B1#5&c579a42&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7};Position=1263,170;Size=317,285").unwrap();
        assert_eq!((w.pos, w.size), ((1263, 170), (317, 285)));
        assert!(w.device.starts_with(r"\\?\DISPLAY#XMI27B1"));
        assert_eq!(parse_window_position("garbage"), None);
    }

    #[test]
    fn lays_out_like_the_screen() {
        use crate::win::Monitor;
        use windows::Win32::Foundation::RECT;
        let second = Monitor {
            handle: 1,
            rect: RECT { left: 2560, top: 165, right: 4480, bottom: 1245 },
            work: RECT { left: 2560, top: 165, right: 4480, bottom: 1197 },
            primary: false,
            device_ids: vec![r"\\?\DISPLAY#AAA#1&2&UID1#{guid}".into()],
        };
        let main = Monitor {
            handle: 2,
            rect: RECT { left: 0, top: 0, right: 2560, bottom: 1440 },
            work: RECT { left: 0, top: 0, right: 2560, bottom: 1392 },
            primary: true,
            device_ids: vec![r"\\?\DISPLAY#BBB#1&2&UID2#{guid}".into()],
        };
        let _ = main;
        let screen = Screen { layer: Some(&second), pixels_per_point: 1.0, area: egui::vec2(1920.0, 1032.0) };
        let planned = |id: &str, device: &str, pos, size| Planned {
            source_id: id.into(),
            kind: Kind::Note,
            title: String::new(),
            body: String::new(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            on_layer: true,
            window: Some(WindowPos { device: device.into(), pos, size }),
            old: false,
            tint: None,
        };
        let notes = [
            planned("here", r"\\?\DISPLAY#AAA#1&2&UID1#{other-guid}", (1263, 170), (317, 285)),
            planned("main", r"\\?\DISPLAY#BBB#1&2&UID2#{guid}", (2207, 102), (353, 405)),
            planned("off-edge", r"\\?\DISPLAY#AAA#1&2&UID1#{guid}", (1800, 900), (320, 320)),
        ];
        let rects = layout(&notes, &screen, &[]);
        assert_eq!(rects[0], Some(egui::Rect::from_min_size(egui::pos2(1263.0, 170.0), egui::vec2(317.0, 285.0))));
        // Kept its size, placed without overlapping the exact ones.
        let main_rect = rects[1].unwrap();
        assert_eq!(main_rect.size(), egui::vec2(353.0, 405.0));
        assert!(!main_rect.intersects(rects[0].unwrap()) && !main_rect.intersects(rects[2].unwrap()));
        // Clamped into the layer.
        assert_eq!(rects[2].unwrap().max, egui::pos2(1920.0, 1032.0));
    }

    #[test]
    fn crowded_layer_shrinks_instead_of_stacking() {
        let area = egui::vec2(1000.0, 600.0);
        // Everything taken except a 170 pt strip at the bottom.
        let taken = [egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 420.0))];
        let r = place(&taken, egui::vec2(350.0, 400.0), area);
        assert_eq!(r.size(), crate::card::DEFAULT_SIZE);
        assert!(!r.intersects(taken[0]) && r.max.y <= area.y);
    }
}
