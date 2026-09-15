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

use crate::card::{Kind, looks_secret, parse_capture};

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
            .prepare("SELECT Id, Text, IsOpen, CreatedAt, UpdatedAt, WindowPosition FROM Note WHERE DeletedAt IS NULL")
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

fn classify(plain: &str) -> Kind {
    if looks_secret(plain) {
        return Kind::Private;
    }
    // Only the beginning decides: a URL or "ssh" deep inside a long note says little.
    let head: String = plain.chars().take(300).collect();
    let lower = head.to_lowercase();
    const PROMPT_STARTS: &[&str] = &["ты —", "ты -", "ты это", "you are", "act as", "представь, что", "представь что"];
    if PROMPT_STARTS.iter().any(|p| lower.starts_with(p)) {
        return Kind::Prompt;
    }
    match parse_capture(&head).kind {
        // "через 2 недели" in a note from two years ago is not a live reminder.
        Kind::Reminder => Kind::Note,
        Kind::Link if plain.chars().count() > 300 => Kind::Note,
        kind => kind,
    }
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
        });
    }

    stats.importable = planned.len();
    stats.on_layer = planned.iter().filter(|p| p.on_layer).count();
    stats.archived = planned.len() - stats.on_layer;
    stats.old = planned.iter().filter(|p| p.old).count();
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
        }
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
