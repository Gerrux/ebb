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

use crate::card::{Kind, parse_capture};

/// .NET ticks (100 ns since 0001-01-01) at the Unix epoch.
const TICKS_AT_UNIX_EPOCH: i64 = 621_355_968_000_000_000;
/// Notes not edited for this long are imported to the archive.
pub const OLD_AFTER_DAYS: i64 = 365;
/// At most this many imported notes go onto the layer...
pub const MAX_ON_LAYER: usize = 8;
/// ...and the layer is not filled beyond this many cards in total.
pub const LAYER_TARGET: usize = 12;

pub fn plum_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let path = Path::new(&base)
        .join(r"Packages\Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe\LocalState\plum.sqlite");
    path.is_file().then_some(path)
}

pub fn ticks_to_unix(ticks: i64) -> i64 {
    (ticks - TICKS_AT_UNIX_EPOCH) / 10_000_000
}

#[derive(Clone, Debug)]
pub struct SourceNote {
    pub id: String,
    pub text: String,
    pub is_open: bool,
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
    let dir = std::env::temp_dir().join(format!("ambient-sticky-{}", std::process::id()));
    let result = (|| {
        let copy = copy_consistent(src, &dir).map_err(|e| format!("copy: {e}"))?;
        // Read-write on the copy so SQLite can replay the WAL into it.
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| format!("open copy: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT Id, Text, IsOpen, CreatedAt, UpdatedAt FROM Note WHERE DeletedAt IS NULL")
            .map_err(|e| format!("query: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceNote {
                    id: r.get(0)?,
                    text: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    is_open: r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
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

/// Looks like a credential: passwords, tokens, keys.
pub fn looks_secret(text: &str) -> bool {
    let lower = text.to_lowercase();
    const MARKERS: &[&str] = &[
        "пароль", "password", "passwd", "pass:", "pwd:", "логин:", "login:", "token:", "token=", "токен:",
        "api key", "api_key", "apikey", "secret:", "секрет:", "private key", "ssh-rsa", "pin:", "пин:",
    ];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // OpenAI-style keys: sk- followed by a long run of key characters.
    lower.match_indices("sk-").any(|(i, _)| {
        lower[i + 3..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').count() >= 20
    })
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

fn split_title(plain: &str) -> (String, String) {
    match plain.split_once('\n') {
        Some((first, rest)) if first.chars().count() <= 80 && !rest.trim().is_empty() => {
            (first.trim().to_owned(), rest.trim().to_owned())
        }
        _ => (String::new(), plain.to_owned()),
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
    pub on_layer: bool,
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

/// Decides what to import and where. `already` holds source ids imported earlier;
/// `layer_free` is how many more cards the layer can take.
pub fn plan(notes: &[SourceNote], already: &std::collections::HashSet<String>, layer_free: usize, now: i64) -> Plan {
    let mut stats = Stats { total: notes.len(), ..Default::default() };

    // Newest first, so the kept copy of a duplicate is the most recently edited one.
    let mut sorted: Vec<&SourceNote> = notes.iter().collect();
    sorted.sort_by_key(|n| std::cmp::Reverse(n.updated_at));

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
        let (title, body) = split_title(&plain);
        let old = now - note.updated_at > OLD_AFTER_DAYS * 86_400;
        planned.push(Planned {
            source_id: note.id.clone(),
            kind,
            title,
            body,
            tags: tags(&plain),
            created_at: note.created_at,
            updated_at: note.updated_at,
            // Candidate for the layer; trimmed to the budget below.
            on_layer: note.is_open && !old,
            old,
        });
    }

    // Open, recent notes go onto the layer, newest first, within the budget.
    let budget = layer_free.min(MAX_ON_LAYER);
    let mut placed = 0;
    for p in &mut planned {
        if p.on_layer {
            if placed < budget {
                placed += 1;
            } else {
                p.on_layer = false;
            }
        }
    }

    stats.importable = planned.len();
    stats.on_layer = placed;
    stats.archived = planned.len() - placed;
    stats.old = planned.iter().filter(|p| p.old).count();
    stats.by_kind = Kind::ALL
        .iter()
        .map(|k| (*k, planned.iter().filter(|p| p.kind == *k).count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    Plan { notes: planned, stats }
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
            created_at: now - updated_days_ago * 86_400,
            updated_at: now - updated_days_ago * 86_400,
        }
    }

    #[test]
    fn plans_layer_archive_duplicates_and_reimport() {
        let notes = vec![
            note("a", "идея: weekly recap", true, 3),
            note("b", "идея:  Weekly   recap", false, 10), // duplicate of a (older)
            note("c", "", true, 1),                        // empty
            note("d", "старая заметка", true, 800),        // open but old -> archive
            note("e", "https://example.com", false, 5),
            note("f", "wifi пароль 123", true, 2),
        ];
        let now = 2_000_000_000;
        let p = plan(&notes, &Default::default(), 10, now);
        assert_eq!(p.stats.duplicates, 1);
        assert_eq!(p.stats.empty, 1);
        assert_eq!(p.stats.importable, 4);
        assert_eq!(p.stats.old, 1);
        let on_layer: Vec<_> = p.notes.iter().filter(|n| n.on_layer).map(|n| n.source_id.as_str()).collect();
        assert_eq!(on_layer, ["f", "a"]);
        assert_eq!(p.notes.iter().find(|n| n.source_id == "f").unwrap().kind, Kind::Private);
        assert_eq!(p.notes.iter().find(|n| n.source_id == "e").unwrap().kind, Kind::Link);

        let limited = plan(&notes, &Default::default(), 1, now);
        assert_eq!(limited.stats.on_layer, 1);

        let already = ["a".to_owned(), "f".to_owned()].into_iter().collect();
        let again = plan(&notes, &already, 10, now);
        assert_eq!(again.stats.already_imported, 2);
        assert!(again.notes.iter().all(|n| n.source_id != "a" && n.source_id != "f"));
    }
}
