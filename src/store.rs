//! SQLite persistence.

use std::path::{Path, PathBuf};

use egui::{pos2, vec2};
use rusqlite::{Connection, OptionalExtension, params};

use crate::card::{Card, DEFAULT_SIZE, IdeaStatus, Kind, Parsed, Placement, Tint, private_parts, private_text};
use crate::resurface::DAY;

/// Local day number of the last Rediscover pick.
const SET_REDISCOVER_DAY: &str = "rediscover_day";

fn vault_error(error: windows::core::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error.to_string())))
}

/// Encrypts Private cards still stored in the open, and hides labels that look
/// like the secret itself (an early build kept any first line as the label).
fn migrate_private_cards(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT id, title, body, secret FROM cards WHERE kind='private'")?;
    let rows: Vec<(i64, String, String, Option<Vec<u8>>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|(_, title, _, secret)| {
            secret.is_none() || (title != crate::card::PRIVATE_PLACEHOLDER && !crate::card::fits_label(title))
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    // The old value may already be present in the WAL and the FTS shadow tables.
    // Secure deletion plus a checkpoint/VACUUM makes this one-way migration
    // remove those plaintext copies before the database is used normally again.
    conn.execute_batch("PRAGMA secure_delete=ON;")?;
    for (id, title, body, secret) in rows {
        let body = match secret {
            // Not this Windows user's: leave it be rather than fail to open.
            Some(bytes) => match crate::vault::unprotect(id, &bytes) {
                Ok(text) => text,
                Err(_) => continue,
            },
            None => body,
        };
        let (label, secret) = private_parts(&title, &body);
        let encrypted = crate::vault::protect(id, &secret).map_err(vault_error)?;
        conn.execute("UPDATE cards SET title=?2, body='', secret=?3 WHERE id=?1", params![id, label, encrypted])?;
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA secure_delete=FAST;")?;
    Ok(())
}

pub struct Store {
    conn: Connection,
}

/// Deleted cards stay restorable for this long, then are purged on startup.
pub const TRASH_DAYS: i64 = 30;

/// Which cards a search covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Everything not in the trash (layer and archive).
    #[default]
    Live,
    Archive,
    Trash,
}

impl Scope {
    fn sql(self) -> &'static str {
        match self {
            Scope::Live => " AND c.deleted_at IS NULL",
            Scope::Archive => " AND c.deleted_at IS NULL AND c.archived = 1",
            Scope::Trash => " AND c.deleted_at IS NOT NULL",
        }
    }
}

/// One search result.
#[derive(Clone, Debug)]
pub struct Hit {
    pub id: i64,
    pub kind: Kind,
    pub title: String,
    /// Body excerpt; matched terms are wrapped in \u{1} … \u{2}. Empty for Private.
    pub snippet: String,
    pub archived: bool,
    pub pinned: bool,
    pub updated_at: i64,
    /// Set for cards in the trash.
    pub deleted_at: Option<i64>,
}

/// What the weekly review does with a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewAction {
    Keep,
    Archive,
    Snooze,
    Pin,
    Trash,
    Goal,
}

impl ReviewAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Archive => "archive",
            Self::Snooze => "snooze",
            Self::Pin => "pin",
            Self::Trash => "trash",
            Self::Goal => "goal",
        }
    }
}

/// A card's state before a review action or a snooze, for undo.
#[derive(Clone, Copy, Debug)]
pub struct ReviewSnapshot {
    pub id: i64,
    pub kind: Kind,
    pub archived: bool,
    pub pinned: bool,
    pub placement: Placement,
    pub review_at: Option<i64>,
    pub ignored_count: i64,
    pub deleted_at: Option<i64>,
    pub last_viewed_at: i64,
}

/// Row of the live card projection used by the layer.
fn card_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let tags: String = r.get(4)?;
    Ok(Card {
        id: r.get(0)?,
        kind: Kind::parse(&r.get::<_, String>(1)?),
        title: r.get(2)?,
        body: r.get(3)?,
        tags: tags.split(',').filter(|t| !t.is_empty()).map(str::to_owned).collect(),
        pinned: r.get(5)?,
        archived: r.get(6)?,
        pos: pos2(r.get(7)?, r.get(8)?),
        size: vec2(r.get(9)?, r.get(10)?),
        created_at: r.get(11)?,
        tint: r.get::<_, Option<String>>(12)?.as_deref().and_then(Tint::parse),
        placement: Placement::parse(&r.get::<_, String>(13)?),
        review_at: r.get(14)?,
        collapsed: r.get(15)?,
        idea_status: r.get::<_, Option<String>>(16)?.as_deref().and_then(IdeaStatus::parse),
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Shown when a window can't open its own connection to the database.
pub const STORE_FAILED: &str = "База заметок недоступна";

/// The database every connection opens: `%LOCALAPPDATA%\Ebb\ebb.db`, or the
/// former name's database while it couldn't be moved yet (see
/// `migrate_legacy_data`), so an empty new one never shadows the user's notes.
pub fn db_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let (new_db, old_db) = (base.join("Ebb").join("ebb.db"), legacy_db(&base));
    if !new_db.exists() && old_db.exists() { old_db } else { new_db }
}

fn legacy_db(base: &Path) -> PathBuf {
    base.join("Ambient").join("ambient.db")
}

fn wal_of(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    name.into()
}

/// Moves data left by the app's former name (`%LOCALAPPDATA%\Ambient\ambient.db`)
/// to `%LOCALAPPDATA%\Ebb\ebb.db`. Runs before the first `Store::open`. A locked
/// old database (an old build still running) stays where it is and in use
/// (`db_path`) until a later launch can move it.
pub fn migrate_legacy_data() {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    migrate_legacy_at(&legacy_db(&base), &base.join("Ebb").join("ebb.db"));
}

fn migrate_legacy_at(old_db: &Path, new_db: &Path) {
    let (Some(old_dir), Some(new_dir)) = (old_db.parent(), new_db.parent()) else { return };
    if new_db.exists() {
        // A move cut short right after the database itself: its log still holds
        // the last commits and must be next to it before anything opens it.
        if !old_db.exists() && wal_of(old_db).exists() && !wal_of(new_db).exists() {
            let _ = std::fs::rename(wal_of(old_db), wal_of(new_db));
        }
        return;
    }
    if !old_db.exists() || std::fs::create_dir_all(new_dir).is_err() {
        return;
    }
    if std::fs::rename(old_db, new_db).is_err() {
        return;
    }
    if wal_of(old_db).exists() && std::fs::rename(wal_of(old_db), wal_of(new_db)).is_err() {
        // Opened without its log the database would lose commits: put it back.
        let _ = std::fs::rename(new_db, old_db);
        return;
    }
    let Ok(entries) = std::fs::read_dir(old_dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().replacen("ambient.db", "ebb.db", 1);
        let _ = std::fs::rename(entry.path(), new_dir.join(name));
    }
    let _ = std::fs::remove_dir(old_dir);
}

impl Store {
    pub fn open() -> rusqlite::Result<Self> {
        Self::open_at(db_path())
    }

    pub fn open_at(path: PathBuf) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             -- Layer, bar and library each hold a connection: a write that meets
             -- another one waits for it instead of failing with SQLITE_BUSY.
             PRAGMA busy_timeout = 1000;
             -- Text a card turned Private leaves behind is zeroed where that's free.
             PRAGMA secure_delete = FAST;
             CREATE TABLE IF NOT EXISTS cards (
                 id INTEGER PRIMARY KEY,
                 kind TEXT NOT NULL,
                 title TEXT NOT NULL DEFAULT '',
                 body TEXT NOT NULL DEFAULT '',
                 tags TEXT NOT NULL DEFAULT '',
                 pinned INTEGER NOT NULL DEFAULT 0,
                 archived INTEGER NOT NULL DEFAULT 0,
                 x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 last_viewed_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             -- One row per imported source note, so re-imports skip it and a batch can be undone.
             CREATE TABLE IF NOT EXISTS imported (
                 source_id TEXT PRIMARY KEY,
                 card_id INTEGER NOT NULL,
                 batch INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS reviews (
                 id INTEGER PRIMARY KEY,
                 started_at INTEGER NOT NULL,
                 finished_at INTEGER,
                 kept INTEGER NOT NULL DEFAULT 0,
                 archived INTEGER NOT NULL DEFAULT 0,
                 snoozed INTEGER NOT NULL DEFAULT 0,
                 trashed INTEGER NOT NULL DEFAULT 0,
                 pinned INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS review_items (
                 review_id INTEGER NOT NULL,
                 card_id INTEGER NOT NULL,
                 action TEXT NOT NULL,
                 at INTEGER NOT NULL,
                 PRIMARY KEY (review_id, card_id)
             );",
        )?;
        let has_fts: bool = conn.query_row("SELECT count(*) FROM sqlite_master WHERE name='cards_fts'", [], |r| r.get(0))?;
        conn.execute_batch(
            "-- External-content index over cards. unicode61 folds case for Cyrillic too;
             -- remove_diacritics folds ё→е. No `prefix=` option: on 10k notes it made
             -- the file 20% larger (38 -> 46 MiB) without making any query faster.
             CREATE VIRTUAL TABLE IF NOT EXISTS cards_fts USING fts5(
                 title, body, tags,
                 content='cards', content_rowid='id',
                 tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER IF NOT EXISTS cards_fts_insert AFTER INSERT ON cards BEGIN
                 INSERT INTO cards_fts(rowid, title, body, tags) VALUES (new.id, new.title, new.body, new.tags);
             END;
             CREATE TRIGGER IF NOT EXISTS cards_fts_delete AFTER DELETE ON cards BEGIN
                 INSERT INTO cards_fts(cards_fts, rowid, title, body, tags)
                 VALUES ('delete', old.id, old.title, old.body, old.tags);
             END;
             -- save() rewrites every column on each drag; reindex only when text changed.
             CREATE TRIGGER IF NOT EXISTS cards_fts_update AFTER UPDATE ON cards
             WHEN old.title IS NOT new.title OR old.body IS NOT new.body OR old.tags IS NOT new.tags BEGIN
                 INSERT INTO cards_fts(cards_fts, rowid, title, body, tags)
                 VALUES ('delete', old.id, old.title, old.body, old.tags);
                 INSERT INTO cards_fts(rowid, title, body, tags) VALUES (new.id, new.title, new.body, new.tags);
             END;",
        )?;
        if !has_fts {
            // Column weights for `ORDER BY rank`: title, body, tags. Stored in the index.
            conn.execute("INSERT INTO cards_fts(cards_fts, rank) VALUES ('rank', 'bm25(8.0, 1.0, 4.0)')", [])?;
            conn.execute("INSERT INTO cards_fts(cards_fts) VALUES ('rebuild')", [])?;
        }
        // Trash: deleted cards keep their row for TRASH_DAYS.
        let has_deleted_at: bool =
            conn.query_row("SELECT count(*) FROM pragma_table_info('cards') WHERE name='deleted_at'", [], |r| r.get(0))?;
        if !has_deleted_at {
            conn.execute("ALTER TABLE cards ADD COLUMN deleted_at INTEGER", [])?;
        }
        // A color picked by hand (card::Tint); NULL takes the kind's.
        let has_tint: bool =
            conn.query_row("SELECT count(*) FROM pragma_table_info('cards') WHERE name='tint'", [], |r| r.get(0))?;
        if !has_tint {
            conn.execute("ALTER TABLE cards ADD COLUMN tint TEXT", [])?;
        }
        let has_secret: bool =
            conn.query_row("SELECT count(*) FROM pragma_table_info('cards') WHERE name='secret'", [], |r| r.get(0))?;
        if !has_secret {
            conn.execute("ALTER TABLE cards ADD COLUMN secret BLOB", [])?;
        }
        migrate_private_cards(&conn)?;
        for (name, definition) in [
            ("placement", "TEXT NOT NULL DEFAULT 'manual'"),
            ("review_at", "INTEGER"),
            ("last_resurfaced_at", "INTEGER"),
            ("resurface_count", "INTEGER NOT NULL DEFAULT 0"),
            ("ignored_count", "INTEGER NOT NULL DEFAULT 0"),
            ("priority", "INTEGER NOT NULL DEFAULT 0"),
            // Stacking order on the layer: higher is drawn above.
            ("z", "INTEGER NOT NULL DEFAULT 0"),
            // Folded to one line on the layer (card::Card::collapsed).
            ("collapsed", "INTEGER NOT NULL DEFAULT 0"),
            // Fields of a card's kind as JSON, read and written with SQLite's
            // json functions: {"idea_status": "explore"}. Kept across kind changes.
            ("meta", "TEXT NOT NULL DEFAULT '{}'"),
        ] {
            let exists: bool = conn.query_row(
                "SELECT count(*) FROM pragma_table_info('cards') WHERE name=?1",
                [name],
                |r| r.get(0),
            )?;
            if !exists {
                conn.execute(&format!("ALTER TABLE cards ADD COLUMN {name} {definition}"), [])?;
            }
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS cards_kind_created ON cards(kind, created_at);
             CREATE INDEX IF NOT EXISTS cards_created ON cards(created_at);
             CREATE INDEX IF NOT EXISTS cards_updated ON cards(updated_at);
             CREATE INDEX IF NOT EXISTS cards_deleted ON cards(deleted_at) WHERE deleted_at IS NOT NULL;
             CREATE INDEX IF NOT EXISTS cards_review ON cards(review_at) WHERE review_at IS NOT NULL;",
        )?;
        conn.execute(
            "DELETE FROM cards WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
            [now() - TRASH_DAYS * 86_400],
        )?;
        Ok(Self { conn })
    }

    /// Full-text search within `scope`. Private bodies are never returned as snippets.
    pub fn search(&self, q: &crate::search::Query, scope: Scope, limit: usize) -> rusqlite::Result<Vec<Hit>> {
        use rusqlite::types::Value;

        // Filters as a condition on `cards` (alias c). The scope condition is kept
        // apart: Live excludes only the few trashed cards, so it doesn't need the
        // exhaustive-join path below.
        let scope_sql = scope.sql();
        let mut filters = String::new();
        let mut args: Vec<Value> = Vec::new();
        if let Some(kind) = q.kind {
            filters.push_str(" AND c.kind = ?");
            args.push(Value::Text(kind.as_str().into()));
        }
        if let Some((from, to, _)) = &q.range {
            filters.push_str(" AND c.created_at >= ? AND c.created_at < ?");
            args.push(Value::Integer(*from));
            args.push(Value::Integer(*to));
        }
        let columns = "c.id, c.kind, c.title, c.deleted_at, c.archived, c.pinned, c.updated_at";
        let row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Hit> {
            let kind = Kind::parse(&r.get::<_, String>(1)?);
            let body: String = r.get(7)?;
            let title: String = r.get(2)?;
            Ok(Hit {
                id: r.get(0)?,
                kind,
                title: if kind == Kind::Private {
                    crate::card::private_label(&title, &body).unwrap_or_default()
                } else {
                    title
                },
                deleted_at: r.get(3)?,
                archived: r.get(4)?,
                pinned: r.get(5)?,
                updated_at: r.get(6)?,
                snippet: if kind == Kind::Private { String::new() } else { crate::search::snippet(&body, q, 160) },
            })
        };
        let run = |sql: String, args: Vec<Value>| -> rusqlite::Result<Vec<Hit>> {
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(args), row)?;
            rows.collect()
        };

        let Some(expr) = q.fts_expression() else {
            // Filters only (or nothing): newest first.
            let order = if scope == Scope::Trash { "c.deleted_at DESC" } else { "c.updated_at DESC" };
            let sql = format!(
                "SELECT {columns}, substr(c.body, 1, 400) FROM cards c WHERE 1{scope_sql}{filters}
                 ORDER BY {order} LIMIT {limit}"
            );
            return run(sql, args);
        };

        // Ranking is bm25 (title > tags > body, stored as the index's `rank`), with
        // cards on the layer and pinned ones nudged up; bm25 is negative. Without
        // filters FTS5 can sort by rank itself and stop early, so only a bounded
        // candidate set gets the boost; with filters every match is joined instead,
        // so a rare kind isn't cut off by the candidate limit. Snippets are built in
        // Rust for the final rows only (cheaper than snippet() on every candidate).
        let candidates = if filters.is_empty() && scope == Scope::Live {
            format!("SELECT rowid, rank FROM cards_fts WHERE cards_fts MATCH ? ORDER BY rank LIMIT {}", (limit * 4).max(100))
        } else {
            "SELECT rowid, rank FROM cards_fts WHERE cards_fts MATCH ?".to_owned()
        };
        let sql = format!(
            "SELECT {columns}, c.body
             FROM ({candidates}) f JOIN cards c ON c.id = f.rowid
             WHERE 1{scope_sql}{filters}
             ORDER BY f.rank - 1.5 * (c.archived = 0) - 1.5 * c.pinned
             LIMIT {limit}"
        );
        let mut fts_args = vec![Value::Text(expr)];
        fts_args.extend(args.iter().cloned());
        let hits = run(sql, fts_args)?;
        if !hits.is_empty() || q.words.is_empty() {
            return Ok(hits);
        }

        // Nothing token-wise: fall back to a substring scan (mid-word matches such as
        // "taging" in "vpn.staging"). LIKE folds ASCII case only. Walks the updated_at
        // index newest-first and stops at the limit; a miss still scans everything.
        let mut like_filters = String::new();
        let mut like_args = Vec::new();
        for w in &q.words {
            like_filters.push_str(" AND (c.title || ' ' || c.body) LIKE ? ESCAPE '\\'");
            let escaped = w.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            like_args.push(Value::Text(format!("%{escaped}%")));
        }
        like_args.extend(args);
        let sql = format!(
            "SELECT {columns}, c.body FROM cards c WHERE 1{scope_sql}{like_filters}{filters}
             ORDER BY c.updated_at DESC LIMIT {limit}"
        );
        run(sql, like_args)
    }

    pub fn card(&self, id: i64) -> rusqlite::Result<Option<Card>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, kind, title, body, tags, pinned, archived, x, y, w, h, created_at, tint, placement, review_at, collapsed,
                    json_extract(meta, '$.idea_status')
             FROM cards WHERE id=?1",
        )?;
        let mut rows = stmt.query_map([id], card_row)?;
        rows.next().transpose()
    }

    /// Decrypts a Private value only for an explicit reveal/copy action.
    pub fn secret(&self, id: i64) -> rusqlite::Result<Option<String>> {
        let row: Option<Option<Vec<u8>>> = self
            .conn
            .query_row("SELECT secret FROM cards WHERE id=?1 AND kind='private'", [id], |r| r.get(0))
            .optional()?;
        // A Private card whose text was cleared has no value at all.
        row.flatten().map(|bytes| crate::vault::unprotect(id, &bytes).map_err(vault_error)).transpose()
    }

    /// Puts a card above every other on the layer, for the next launch too.
    pub fn raise(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET z=(SELECT coalesce(max(z), 0) + 1 FROM cards) WHERE id=?1", [id])?;
        Ok(())
    }

    /// Records that the user looked at a card (input for resurfacing).
    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET last_viewed_at=?2 WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    /// Once a local day (`utc_offset` in seconds), takes yesterday's Rediscover
    /// cards that got no reaction back to the archive and brings up to `limit`
    /// forgotten ones onto the layer. A restart the same day changes nothing.
    /// Only placement metadata changes; note text is never rewritten.
    pub fn refresh_resurfacing(&self, at: i64, utc_offset: i64, limit: usize) -> rusqlite::Result<Vec<crate::resurface::Pick>> {
        if self.resurfaced_today(at, utc_offset) {
            return Ok(Vec::new());
        }
        let day = crate::resurface::local_day(at, utc_offset);
        let tx = self.conn.unchecked_transaction()?;
        // Pinned since it came back (older builds pinned from search without
        // touching placement): it's the user's now, never taken back.
        tx.execute("UPDATE cards SET placement='pinned' WHERE placement='rediscover' AND pinned=1", [])?;
        // Opened since it came back: it stays, as if placed by hand. Either way a
        // reminder that brought it has been shown and doesn't bring it again.
        tx.execute(
            "UPDATE cards SET placement='manual', review_at=CASE WHEN review_at <= ?1 THEN NULL ELSE review_at END
             WHERE placement='rediscover' AND archived=0 AND last_viewed_at >= last_resurfaced_at",
            [at],
        )?;
        tx.execute(
            "UPDATE cards SET archived=1, placement='archive', ignored_count=ignored_count+1,
                 review_at=CASE WHEN review_at <= ?1 THEN NULL ELSE review_at END
             WHERE placement='rediscover' AND archived=0 AND deleted_at IS NULL",
            [at],
        )?;
        let mut stmt = tx.prepare(
            "SELECT id, kind, created_at, last_viewed_at, review_at, last_resurfaced_at,
                    ignored_count,
                    -- An idea marked important counts as high priority.
                    priority + coalesce(kind = 'idea' AND json_extract(meta, '$.idea_status') = 'important', 0),
                    pinned, archived, deleted_at
             FROM cards WHERE archived=1 AND deleted_at IS NULL",
        )?;
        let candidates = stmt
            .query_map([], |r| {
                Ok(crate::resurface::Candidate {
                    id: r.get(0)?,
                    kind: Kind::parse(&r.get::<_, String>(1)?),
                    created_at: r.get(2)?,
                    last_viewed_at: r.get(3)?,
                    review_at: r.get(4)?,
                    last_resurfaced_at: r.get(5)?,
                    ignored_count: r.get(6)?,
                    priority: r.get(7)?,
                    pinned: r.get(8)?,
                    archived: r.get(9)?,
                    deleted: r.get::<_, Option<i64>>(10)?.is_some(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        let picks = crate::resurface::candidates(at, &candidates, limit);
        for pick in &picks {
            tx.execute(
                "UPDATE cards SET archived=0, placement='rediscover', last_resurfaced_at=?2,
                 resurface_count=resurface_count+1 WHERE id=?1",
                params![pick.id, at],
            )?;
        }
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![SET_REDISCOVER_DAY, day.to_string()],
        )?;
        tx.commit()?;
        Ok(picks)
    }

    /// Whether the Rediscover pick for the day `at` falls in has been made.
    pub fn resurfaced_today(&self, at: i64, utc_offset: i64) -> bool {
        let day = crate::resurface::local_day(at, utc_offset);
        self.setting(SET_REDISCOVER_DAY).and_then(|v| v.parse::<i64>().ok()) == Some(day)
    }

    pub fn review_queue(&self, at: i64, limit: usize) -> rusqlite::Result<Vec<Card>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, body, tags, pinned, archived, x, y, w, h, created_at, tint, placement, review_at, collapsed,
                    json_extract(meta, '$.idea_status')
             FROM cards
             WHERE deleted_at IS NULL AND kind != 'private' AND pinned=0
               AND (review_at IS NULL OR review_at <= ?1)
               AND (archived=1 OR last_viewed_at <= ?1 - ?2 OR (review_at IS NOT NULL AND review_at <= ?1))
             ORDER BY archived DESC, (review_at IS NOT NULL AND review_at <= ?1) DESC, ignored_count DESC, last_viewed_at ASC
             LIMIT ?3",
        )?;
        stmt.query_map(params![at, 30 * DAY, limit as i64], card_row)?.collect()
    }

    pub fn begin_review(&self, at: i64) -> rusqlite::Result<i64> {
        self.conn.execute("INSERT INTO reviews (started_at) VALUES (?1)", [at])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn record_review_item(&self, review_id: i64, card_id: i64, action: ReviewAction, at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO review_items (review_id, card_id, action, at) VALUES (?1, ?2, ?3, ?4)",
            params![review_id, card_id, action.as_str(), at],
        )?;
        Ok(())
    }

    /// An undone review action no longer counts.
    pub fn forget_review_item(&self, review_id: i64, card_id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM review_items WHERE review_id=?1 AND card_id=?2", params![review_id, card_id])?;
        Ok(())
    }

    pub fn finish_review(&self, review_id: i64, at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE reviews SET finished_at=?2,
                 kept=(SELECT count(*) FROM review_items WHERE review_id=?1 AND action='keep'),
                 archived=(SELECT count(*) FROM review_items WHERE review_id=?1 AND action='archive'),
                 snoozed=(SELECT count(*) FROM review_items WHERE review_id=?1 AND action='snooze'),
                 trashed=(SELECT count(*) FROM review_items WHERE review_id=?1 AND action='trash'),
                 pinned=(SELECT count(*) FROM review_items WHERE review_id=?1 AND action='pin')
             WHERE id=?1",
            params![review_id, at],
        )?;
        Ok(())
    }

    /// Puts a card away until `until` (see `resurface::snooze_until`): off the
    /// layer into the archive, from where Rediscover brings it back once it's due.
    /// Unlike a snooze in the weekly review, it doesn't count as ignored.
    pub fn snooze(&self, id: i64, until: i64) -> rusqlite::Result<ReviewSnapshot> {
        let snapshot = self.snapshot(id)?;
        self.conn.execute(
            "UPDATE cards SET archived=1, pinned=0, placement='archive', review_at=?2, last_viewed_at=?3 WHERE id=?1",
            params![id, until, now()],
        )?;
        Ok(snapshot)
    }

    fn snapshot(&self, id: i64) -> rusqlite::Result<ReviewSnapshot> {
        self.conn.query_row(
            "SELECT id, kind, archived, pinned, placement, review_at, ignored_count, deleted_at, last_viewed_at
             FROM cards WHERE id=?1",
            [id],
            |r| {
                Ok(ReviewSnapshot {
                    id: r.get(0)?,
                    kind: Kind::parse(&r.get::<_, String>(1)?),
                    archived: r.get(2)?,
                    pinned: r.get(3)?,
                    placement: Placement::parse(&r.get::<_, String>(4)?),
                    review_at: r.get(5)?,
                    ignored_count: r.get(6)?,
                    deleted_at: r.get(7)?,
                    last_viewed_at: r.get(8)?,
                })
            },
        )
    }

    pub fn review_action(&self, id: i64, action: ReviewAction) -> rusqlite::Result<ReviewSnapshot> {
        let snapshot = self.snapshot(id)?;
        let t = now();
        match action {
            ReviewAction::Keep => {
                let placement = if snapshot.archived { Placement::Archive } else { Placement::Manual };
                self.conn.execute(
                    "UPDATE cards SET review_at=?2, ignored_count=0, placement=?3, last_viewed_at=?4 WHERE id=?1",
                    params![id, t + 56 * DAY, placement.as_str(), t],
                )?;
            }
            ReviewAction::Archive => {
                self.conn.execute(
                    "UPDATE cards SET archived=1, placement='archive', review_at=?2, last_viewed_at=?3 WHERE id=?1",
                    params![id, t + 56 * DAY, t],
                )?;
            }
            ReviewAction::Snooze => {
                self.conn.execute(
                    "UPDATE cards SET review_at=?2, ignored_count=ignored_count+1, last_viewed_at=?3 WHERE id=?1",
                    params![id, t + 14 * DAY, t],
                )?;
            }
            ReviewAction::Pin => {
                self.conn.execute(
                    "UPDATE cards SET pinned=1, archived=0, placement='pinned', last_viewed_at=?2 WHERE id=?1",
                    params![id, t],
                )?;
            }
            ReviewAction::Trash => self.delete(id)?,
            ReviewAction::Goal => {
                self.conn.execute(
                    "UPDATE cards SET kind='goal', archived=0, pinned=0, placement='manual', review_at=NULL, last_viewed_at=?2
                     WHERE id=?1",
                    params![id, t],
                )?;
            }
        }
        Ok(snapshot)
    }

    pub fn undo_review(&self, snapshot: &ReviewSnapshot) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE cards SET kind=?2, archived=?3, pinned=?4, placement=?5, review_at=?6,
             ignored_count=?7, deleted_at=?8, last_viewed_at=?9 WHERE id=?1",
            params![
                snapshot.id,
                snapshot.kind.as_str(),
                snapshot.archived,
                snapshot.pinned,
                snapshot.placement.as_str(),
                snapshot.review_at,
                snapshot.ignored_count,
                snapshot.deleted_at,
                snapshot.last_viewed_at,
            ],
        )?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0))
            .ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// State of earlier imports from `prefix`, for importing again:
    /// - source ids to leave alone: their card was changed in Ebb (moved, edited,
    ///   trashed) or is gone, so a re-import must not duplicate or resurrect it;
    /// - card ids that are still exactly as imported and can be replaced.
    ///
    /// "Changed" is `updated_at > last_viewed_at`: an import sets last_viewed_at to
    /// the import time (layer) or the note's own updated_at (archive), and every
    /// save() moves updated_at to now.
    pub fn import_state(&self, prefix: &str) -> rusqlite::Result<(std::collections::HashSet<String>, Vec<i64>)> {
        let mut stmt = self.conn.prepare(
            "SELECT i.source_id, c.id,
                    c.id IS NOT NULL AND c.deleted_at IS NULL AND c.updated_at <= c.last_viewed_at
             FROM imported i LEFT JOIN cards c ON c.id = i.card_id
             WHERE i.source_id LIKE ?1 || '%'",
        )?;
        let mut keep = std::collections::HashSet::new();
        let mut replace = Vec::new();
        let rows = stmt.query_map([prefix], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, bool>(2)?)))?;
        for row in rows {
            let (source, card, replaceable) = row?;
            match (replaceable, card) {
                (true, Some(id)) => replace.push(id),
                _ => {
                    keep.insert(source[prefix.len()..].to_owned());
                }
            }
        }
        Ok((keep, replace))
    }

    /// In one transaction: removes the `replace` cards of an earlier import, then
    /// inserts `notes`. `rects[i]` is `Some` for notes that go onto the layer.
    /// Returns the batch id.
    pub fn import(
        &mut self,
        prefix: &str,
        notes: &[crate::sticky::Planned],
        rects: &[Option<egui::Rect>],
        replace: &[i64],
    ) -> rusqlite::Result<i64> {
        let viewed = now();
        let tx = self.conn.transaction()?;
        {
            let mut drop_card = tx.prepare("DELETE FROM cards WHERE id=?1")?;
            let mut drop_link = tx.prepare("DELETE FROM imported WHERE card_id=?1")?;
            for id in replace {
                drop_card.execute([id])?;
                drop_link.execute([id])?;
            }
        }
        let batch: i64 = tx.query_row("SELECT COALESCE(MAX(batch), 0) + 1 FROM imported", [], |r| r.get(0))?;
        {
            let mut card = tx.prepare(
                "INSERT INTO cards (kind, title, body, tags, archived, x, y, w, h, created_at, updated_at, last_viewed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            let mut protect = tx.prepare("UPDATE cards SET title=?2, body='', secret=?3 WHERE id=?1")?;
            let mut link = tx.prepare("INSERT INTO imported (source_id, card_id, batch) VALUES (?1, ?2, ?3)")?;
            for (n, rect) in notes.iter().zip(rects) {
                let r = rect.unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, DEFAULT_SIZE));
                let (title, body) = if n.kind == Kind::Private {
                    let (label, _) = private_parts(&n.title, &n.body);
                    (label, String::new())
                } else {
                    (n.title.clone(), n.body.clone())
                };
                card.execute(params![
                    n.kind.as_str(),
                    title,
                    body,
                    n.tags.join(","),
                    rect.is_none(),
                    r.min.x,
                    r.min.y,
                    r.width(),
                    r.height(),
                    n.created_at,
                    n.updated_at,
                    // Not "viewed" in Ebb yet, except what lands on the layer now.
                    // One second back: timestamps are in seconds, and a move right
                    // after the import must still count as a change (import_state).
                    if rect.is_some() { (viewed - 1).max(n.updated_at) } else { n.updated_at },
                ])?;
                let id = tx.last_insert_rowid();
                if n.kind == Kind::Private {
                    let (label, secret) = private_parts(&n.title, &n.body);
                    let encrypted = crate::vault::protect(id, &secret).map_err(vault_error)?;
                    protect.execute(params![id, label, encrypted])?;
                }
                link.execute(params![format!("{prefix}{}", n.source_id), id, batch])?;
            }
        }
        tx.commit()?;
        Ok(batch)
    }

    /// Deletes the cards of an import batch and forgets it, so it can be imported again.
    pub fn undo_import(&mut self, batch: i64) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        let n = tx.execute("DELETE FROM cards WHERE id IN (SELECT card_id FROM imported WHERE batch=?1)", [batch])?;
        tx.execute("DELETE FROM imported WHERE batch=?1", [batch])?;
        tx.commit()?;
        Ok(n)
    }

    pub fn load(&self) -> rusqlite::Result<Vec<Card>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, body, tags, pinned, archived, x, y, w, h, created_at, tint, placement, review_at, collapsed,
                    json_extract(meta, '$.idea_status')
             FROM cards WHERE archived = 0 AND deleted_at IS NULL ORDER BY z, updated_at",
        )?;
        let rows = stmt.query_map([], card_row)?;
        rows.collect()
    }

    pub fn insert(&self, p: &Parsed, pos: egui::Pos2) -> rusqlite::Result<Card> {
        let t = now();
        let (title, body) = if p.kind == Kind::Private {
            let (label, _) = private_parts(&p.title, &p.body);
            (label, String::new())
        } else {
            (p.title.clone(), p.body.clone())
        };
        self.conn.execute(
            "INSERT INTO cards (kind, title, body, tags, x, y, w, h, created_at, updated_at, last_viewed_at, z)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9, (SELECT coalesce(max(z), 0) + 1 FROM cards))",
            params![
                p.kind.as_str(),
                title,
                body,
                p.tags.join(","),
                pos.x,
                pos.y,
                DEFAULT_SIZE.x,
                DEFAULT_SIZE.y,
                t
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        if p.kind == Kind::Private {
            let (_, secret) = private_parts(&p.title, &p.body);
            let encrypted = crate::vault::protect(id, &secret).map_err(vault_error)?;
            self.conn.execute("UPDATE cards SET secret=?2 WHERE id=?1", params![id, encrypted])?;
        }
        let review_at = (p.kind == Kind::Reminder)
            .then(|| crate::resurface::review_at_from_text(&format!("{}
{}", p.title, p.body), t))
            .flatten();
        if let Some(review_at) = review_at {
            self.conn.execute("UPDATE cards SET review_at=?2 WHERE id=?1", params![id, review_at])?;
        }
        Ok(Card {
            id,
            kind: p.kind,
            title,
            body,
            tags: p.tags.clone(),
            pinned: false,
            archived: false,
            pos,
            size: DEFAULT_SIZE,
            created_at: t,
            review_at,
            placement: Placement::Manual,
            tint: None,
            collapsed: false,
            idea_status: None,
        })
    }

    /// Sets or clears an idea's status. Layout-like metadata: the note doesn't
    /// count as changed.
    pub fn set_idea_status(&self, id: i64, status: Option<IdeaStatus>) -> rusqlite::Result<()> {
        match status {
            Some(s) => self.conn.execute("UPDATE cards SET meta=json_set(meta, '$.idea_status', ?2) WHERE id=?1", params![id, s.as_str()])?,
            None => self.conn.execute("UPDATE cards SET meta=json_remove(meta, '$.idea_status') WHERE id=?1", [id])?,
        };
        Ok(())
    }

    /// Folds a card to one line on the layer, or opens it. Layout only: the note
    /// doesn't count as changed.
    pub fn set_collapsed(&self, id: i64, collapsed: bool) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET collapsed=?2 WHERE id=?1", params![id, collapsed])?;
        Ok(())
    }

    pub fn save(&self, c: &Card) -> rusqlite::Result<()> {
        if c.kind == Kind::Private {
            let (title, secret) = private_parts(&c.title, &c.body);
            if !c.body.is_empty() {
                let encrypted = crate::vault::protect(c.id, &secret).map_err(vault_error)?;
                self.conn.execute(
                    "UPDATE cards SET kind=?2, title=?3, body='', tags=?4, pinned=?5, archived=?6,
                         x=?7, y=?8, w=?9, h=?10, updated_at=?11, tint=?12, placement=?13, review_at=?14, secret=?15 WHERE id=?1",
                    params![c.id, c.kind.as_str(), title, c.tags.join(","), c.pinned, c.archived, c.pos.x, c.pos.y, c.size.x, c.size.y, now(), c.tint.map(Tint::as_str), c.placement.as_str(), c.review_at, encrypted],
                )?;
            } else {
                self.conn.execute(
                    "UPDATE cards SET kind=?2, title=?3, body='', tags=?4, pinned=?5, archived=?6,
                         x=?7, y=?8, w=?9, h=?10, updated_at=?11, tint=?12, placement=?13, review_at=?14 WHERE id=?1",
                    params![c.id, c.kind.as_str(), title, c.tags.join(","), c.pinned, c.archived, c.pos.x, c.pos.y, c.size.x, c.size.y, now(), c.tint.map(Tint::as_str), c.placement.as_str(), c.review_at],
                )?;
            }
        } else {
            self.conn.execute(
                "UPDATE cards SET kind=?2, title=?3, body=?4, tags=?5, pinned=?6, archived=?7,
                     x=?8, y=?9, w=?10, h=?11, updated_at=?12, tint=?13, placement=?14, review_at=?15, secret=NULL WHERE id=?1",
                params![c.id, c.kind.as_str(), c.title, c.body, c.tags.join(","), c.pinned, c.archived, c.pos.x, c.pos.y, c.size.x, c.size.y, now(), c.tint.map(Tint::as_str), c.placement.as_str(), c.review_at],
            )?;
        }
        Ok(())
    }

    /// Drops a Private card's encrypted value: `save` can't tell a cleared
    /// editor from a card that never carries its value in memory.
    pub fn clear_secret(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET secret=NULL, updated_at=?2 WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    /// Moves a card to the trash.
    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET deleted_at=?2 WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    /// Takes a card out of the trash, back where it was (layer or archive).
    pub fn restore(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET deleted_at=NULL WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn set_kind(&self, id: i64, kind: Kind) -> rusqlite::Result<()> {
        let (old_kind, title, body): (String, String, String) = self.conn.query_row(
            "SELECT kind, title, body FROM cards WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let old_kind = Kind::parse(&old_kind);
        match (old_kind, kind) {
            (Kind::Private, new) if new != Kind::Private => {
                // The label comes back as the first line: nothing the note had is lost.
                let text = private_text(&title, &self.secret(id)?.unwrap_or_default());
                self.conn.execute(
                    "UPDATE cards SET kind=?2, title='', body=?3, secret=NULL, updated_at=?4 WHERE id=?1",
                    params![id, new.as_str(), text, now()],
                )?;
            }
            (old, Kind::Private) if old != Kind::Private => {
                let (label, secret) = private_parts(&title, &body);
                let encrypted = crate::vault::protect(id, &secret).map_err(vault_error)?;
                self.conn.execute(
                    "UPDATE cards SET kind='private', title=?2, body='', secret=?3, updated_at=?4 WHERE id=?1",
                    params![id, label, encrypted, now()],
                )?;
            }
            _ => {
                self.conn.execute("UPDATE cards SET kind=?2, updated_at=?3 WHERE id=?1", params![id, kind.as_str(), now()])?;
            }
        }
        Ok(())
    }

    pub fn set_archived(&self, id: i64, archived: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE cards SET archived=?2, placement=?3 WHERE id=?1",
            params![id, archived, if archived { Placement::Archive.as_str() } else { Placement::Manual.as_str() }],
        )?;
        Ok(())
    }

    /// Deletes a trashed card for good.
    pub fn purge(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM cards WHERE id=?1 AND deleted_at IS NOT NULL", [id])?;
        Ok(())
    }

    pub fn empty_trash(&self) -> rusqlite::Result<usize> {
        self.conn.execute("DELETE FROM cards WHERE deleted_at IS NOT NULL", [])
    }

    /// (on the layer, archived, in the trash)
    pub fn counts(&self) -> rusqlite::Result<(i64, i64, i64)> {
        self.conn.query_row(
            "SELECT count(*) FILTER (WHERE deleted_at IS NULL AND archived = 0),
                    count(*) FILTER (WHERE deleted_at IS NULL AND archived = 1),
                    count(*) FILTER (WHERE deleted_at IS NOT NULL)
             FROM cards",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sticky::{Planned, plan};

    fn temp_store(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ebb-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Store::open_at(dir.join("t.db")).unwrap(), dir)
    }

    fn planned(id: &str, on_layer: bool) -> Planned {
        Planned {
            source_id: id.into(),
            kind: Kind::Note,
            title: String::new(),
            body: format!("note {id}"),
            tags: vec!["sticky".into()],
            created_at: 1_600_000_000,
            updated_at: 1_650_000_000,
            on_layer,
            window: None,
            old: false,
        }
    }

    fn add(store: &Store, text: &str) -> Card {
        store.insert(&crate::card::parse_capture(text), egui::pos2(0.0, 0.0)).unwrap()
    }

    fn find(store: &Store, query: &str) -> Vec<String> {
        let q = crate::search::parse(query, now(), 0);
        store.search(&q, Scope::Live, 20).unwrap().into_iter().map(|h| format!("{}|{}", h.title, h.snippet.replace(['\u{1}', '\u{2}'], ""))).collect()
    }

    #[test]
    fn full_text_search() {
        let (store, dir) = temp_store("fts");
        add(&store, "Онбординг\nПопробовать онбординг без регистрации #product");
        add(&store, "идея: weekly recap по пятницам");
        add(&store, "vpn staging vpn.staging.internal #infra");
        let secret = add(&store, "секрет: Wi-Fi\nпароль hunter2");

        // Case-insensitive Cyrillic, inflected form via stemming + prefix.
        assert_eq!(find(&store, "ОНБОРДИНГА").len(), 1);
        assert_eq!(find(&store, "vpn").len(), 1);
        assert_eq!(find(&store, "#infra").len(), 1);
        assert_eq!(find(&store, "идеи").len(), 1, "kind filter without text");
        // Private values are neither indexed nor returned as snippets.
        assert!(find(&store, "hunter2").is_empty());
        assert_eq!(find(&store, "Wi-Fi"), ["Wi-Fi|"]);
        assert_eq!(store.secret(secret.id).unwrap().as_deref(), Some("пароль hunter2"));
        // Substring fallback for mid-word matches.
        assert_eq!(find(&store, "board").len(), 0, "LIKE is on title+body; 'board' isn't there");
        assert_eq!(find(&store, "taging").len(), 1);

        // Ordinary edits still reindex; the trash is out of normal search but can
        // be searched and restored; purging drops the card from the index.
        let mut edited = add(&store, "ordinary changed");
        edited.body = "ordinary changed".into();
        store.save(&edited).unwrap();
        assert_eq!(find(&store, "changed").len(), 1);
        store.delete(edited.id).unwrap();
        assert!(find(&store, "changed").is_empty());
        let q = crate::search::parse("changed", now(), 0);
        let trashed = store.search(&q, Scope::Trash, 20).unwrap();
        assert_eq!(trashed.len(), 1);
        assert!(trashed[0].deleted_at.is_some());
        assert_eq!(store.counts().unwrap().2, 1);
        store.restore(edited.id).unwrap();
        assert_eq!(find(&store, "changed").len(), 1);
        store.set_archived(edited.id, true).unwrap();
        assert_eq!(store.search(&q, Scope::Archive, 20).unwrap().len(), 1);
        store.delete(edited.id).unwrap();
        store.purge(edited.id).unwrap();
        assert!(store.search(&q, Scope::Trash, 20).unwrap().is_empty());
        assert!(store.card(edited.id).unwrap().is_none());

        // Reopening an existing database without the index builds it.
        store.conn.execute_batch("DROP TABLE cards_fts;").unwrap();
        drop(store);
        let store = Store::open_at(dir.join("t.db")).unwrap();
        assert_eq!(find(&store, "онбординг").len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_legacy_private_text_out_of_database() {
        let dir = std::env::temp_dir().join(format!("ebb-test-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE cards (
                    id INTEGER PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                    body TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '', pinned INTEGER NOT NULL DEFAULT 0,
                    archived INTEGER NOT NULL DEFAULT 0, x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, last_viewed_at INTEGER NOT NULL
                );
                INSERT INTO cards (kind, body, x, y, w, h, created_at, updated_at, last_viewed_at)
                VALUES ('private', 'Wi-Fi office\nsecret-legacy-42', 0, 0, 280, 150, 1, 1, 1);",
            )
            .unwrap();
        }
        let store = Store::open_at(path.clone()).unwrap();
        let card = store.card(1).unwrap().unwrap();
        assert_eq!(card.title, "Wi-Fi office");
        assert!(card.body.is_empty());
        assert_eq!(store.secret(1).unwrap().as_deref(), Some("secret-legacy-42"));
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(b"secret-legacy-42".len()).any(|w| w == b"secret-legacy-42"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refresh_puts_an_old_archived_idea_back_on_the_layer_once_a_day() {
        let (store, dir) = temp_store("resurface");
        let ideas: Vec<Card> = (0..4).map(|i| add(&store, &format!("идея: annual pricing {i}"))).collect();
        let private = add(&store, "секрет: Wi-Fi
secret");
        for c in ideas.iter().chain([&private]) {
            store.set_archived(c.id, true).unwrap();
        }
        store.conn.execute("UPDATE cards SET created_at=1, last_viewed_at=1", []).unwrap();

        let day = 100 * DAY;
        let picks = store.refresh_resurfacing(day, 0, 3).unwrap();
        let picked: Vec<i64> = picks.iter().map(|p| p.id).collect();
        assert_eq!(picked, ideas[..3].iter().map(|c| c.id).collect::<Vec<_>>());
        assert_eq!(store.card(ideas[0].id).unwrap().unwrap().placement, Placement::Rediscover);
        assert_eq!(store.card(private.id).unwrap().unwrap().placement, Placement::Archive);

        // A restart the same day neither changes nor grows the set.
        assert!(store.refresh_resurfacing(day + 3_600, 0, 3).unwrap().is_empty());
        assert_eq!(store.load().unwrap().len(), 3);

        // Next day: the one that was opened stays, the ignored ones go back, and
        // the one left over comes up; the rest wait out their cooldown.
        store.conn.execute("UPDATE cards SET last_viewed_at=?2 WHERE id=?1", params![ideas[0].id, day + 60]).unwrap();
        let picks = store.refresh_resurfacing(day + DAY, 0, 3).unwrap();
        assert_eq!(picks.iter().map(|p| p.id).collect::<Vec<_>>(), vec![ideas[3].id]);
        let mut live: Vec<(i64, &str)> = store.load().unwrap().iter().map(|c| (c.id, c.placement.as_str())).collect();
        live.sort();
        assert_eq!(live, [(ideas[0].id, "manual"), (ideas[3].id, "rediscover")]);
        let ignored: i64 = store.conn.query_row("SELECT ignored_count FROM cards WHERE id=?1", [ideas[1].id], |r| r.get(0)).unwrap();
        assert_eq!(ignored, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn snoozed_card_leaves_the_layer_and_comes_back_on_its_morning() {
        let (store, dir) = temp_store("snooze");
        let card = add(&store, "идея: annual pricing");
        let now = 100 * DAY + 20 * 3_600;
        let until = crate::resurface::snooze_until(now, 0, 3);
        let snapshot = store.snooze(card.id, until).unwrap();
        assert!(store.load().unwrap().is_empty());
        let ignored: i64 = store.conn.query_row("SELECT ignored_count FROM cards WHERE id=?1", [card.id], |r| r.get(0)).unwrap();
        assert_eq!(ignored, 0, "putting a card off by hand isn't ignoring it");

        // Undo: back on the layer as it was.
        store.undo_review(&snapshot).unwrap();
        assert_eq!(store.load().unwrap().len(), 1);
        store.snooze(card.id, until).unwrap();
        // Opened just now, so not forgotten; it comes back only because it's due.
        store.conn.execute("UPDATE cards SET last_viewed_at=?2 WHERE id=?1", params![card.id, now]).unwrap();

        for day in 1..3 {
            let morning = crate::resurface::next_day_start(now, 0) + (day - 1) * DAY;
            assert!(store.refresh_resurfacing(morning, 0, 3).unwrap().is_empty(), "not back on day {day}");
        }
        let third = crate::resurface::next_day_start(now, 0) + 2 * DAY;
        assert!(!store.resurfaced_today(third, 0));
        let picks = store.refresh_resurfacing(third, 0, 3).unwrap();
        assert_eq!(picks.iter().map(|p| p.id).collect::<Vec<_>>(), vec![card.id]);
        assert!(store.resurfaced_today(third + 3_600, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_card_pinned_while_rediscovered_stays_on_the_layer() {
        let (store, dir) = temp_store("pinned-rediscover");
        let idea = add(&store, "идея: annual pricing");
        store.set_archived(idea.id, true).unwrap();
        store.conn.execute("UPDATE cards SET created_at=1, last_viewed_at=1", []).unwrap();
        let day = 100 * DAY;
        assert_eq!(store.refresh_resurfacing(day, 0, 3).unwrap().len(), 1);
        // As search used to pin it: the flag without the placement.
        store.conn.execute("UPDATE cards SET pinned=1 WHERE id=?1", [idea.id]).unwrap();
        store.refresh_resurfacing(day + DAY, 0, 3).unwrap();
        let card = store.card(idea.id).unwrap().unwrap();
        assert!(!card.archived);
        assert_eq!(card.placement, Placement::Pinned);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn emptied_private_card_keeps_no_value() {
        let (store, dir) = temp_store("private-clear");
        let card = add(&store, "секрет: Wi-Fi\nhunter2");
        store.clear_secret(card.id).unwrap();
        assert_eq!(store.secret(card.id).unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_database_moves_with_its_log_or_not_at_all() {
        let dir = std::env::temp_dir().join(format!("ebb-test-legacy-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (old_db, new_db) = (dir.join("Ambient").join("ambient.db"), dir.join("Ebb").join("ebb.db"));
        std::fs::create_dir_all(old_db.parent().unwrap()).unwrap();
        std::fs::write(&old_db, "db").unwrap();
        std::fs::write(wal_of(&old_db), "wal").unwrap();

        // Held open by an old build: nothing moves, nothing new is created.
        // As SQLite opens it: shared for reading and writing, not for delete/rename.
        let held = std::os::windows::fs::OpenOptionsExt::share_mode(std::fs::OpenOptions::new().read(true), 0x1 | 0x2)
            .open(&old_db)
            .unwrap();
        migrate_legacy_at(&old_db, &new_db);
        assert!(old_db.exists() && !new_db.exists());
        drop(held);

        migrate_legacy_at(&old_db, &new_db);
        assert_eq!(std::fs::read_to_string(&new_db).unwrap(), "db");
        assert_eq!(std::fs::read_to_string(wal_of(&new_db)).unwrap(), "wal");
        assert!(!old_db.parent().unwrap().exists());

        // Cut short after the database: the log follows on the next launch.
        std::fs::create_dir_all(old_db.parent().unwrap()).unwrap();
        std::fs::rename(wal_of(&new_db), wal_of(&old_db)).unwrap();
        migrate_legacy_at(&old_db, &new_db);
        assert!(wal_of(&new_db).exists() && !wal_of(&old_db).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn collapsed_card_stays_collapsed_after_reopening() {
        let (store, dir) = temp_store("collapsed");
        let card = add(&store, "длинная заметка\nв несколько строк");
        store.set_collapsed(card.id, true).unwrap();
        let path = store.conn.path().unwrap().to_owned();
        drop(store);
        let store = Store::open_at(PathBuf::from(path)).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded[0].collapsed);
        assert_eq!(loaded[0].size, DEFAULT_SIZE, "opens to its size");
        // A later save of the card (a drag) leaves it collapsed.
        store.save(&loaded[0]).unwrap();
        assert!(store.card(card.id).unwrap().unwrap().collapsed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn raised_cards_load_on_top() {
        let (store, dir) = temp_store("z-order");
        let ids: Vec<i64> = ["a", "b", "c"].iter().map(|t| add(&store, t).id).collect();
        assert_eq!(store.load().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), ids);
        store.raise(ids[0]).unwrap();
        assert_eq!(store.load().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [ids[1], ids[2], ids[0]]);
        // A new card starts above the raised one.
        let d = add(&store, "d").id;
        assert_eq!(store.load().unwrap().last().map(|c| c.id), Some(d));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn labels_that_look_like_secrets_are_hidden_on_open() {
        let (store, dir) = temp_store("relabel");
        let card = add(&store, "note");
        let path = store.conn.path().unwrap().to_owned();
        // As an early build left it: the credential line as the open label.
        let encrypted = crate::vault::protect(card.id, "rest").unwrap();
        store
            .conn
            .execute(
                "UPDATE cards SET kind='private', title='API_KEY=abc123', body='', secret=?2 WHERE id=?1",
                params![card.id, encrypted],
            )
            .unwrap();
        drop(store);
        let store = Store::open_at(PathBuf::from(path)).unwrap();
        let hidden = store.card(card.id).unwrap().unwrap();
        assert_eq!(hidden.title, crate::card::PRIVATE_PLACEHOLDER);
        assert_eq!(store.secret(card.id).unwrap().as_deref(), Some("API_KEY=abc123\nrest"));
        assert!(find(&store, "abc123").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn private_kind_round_trip_keeps_label_and_value() {
        let (store, dir) = temp_store("private-kind");
        let card = add(&store, "Wi-Fi офис
guest / pass");
        store.set_kind(card.id, Kind::Private).unwrap();
        let hidden = store.card(card.id).unwrap().unwrap();
        assert_eq!((hidden.title.as_str(), hidden.body.as_str()), ("Wi-Fi офис", ""));
        assert_eq!(store.secret(card.id).unwrap().as_deref(), Some("guest / pass"));
        store.set_kind(card.id, Kind::Note).unwrap();
        let open = store.card(card.id).unwrap().unwrap();
        assert_eq!(open.body, "Wi-Fi офис
guest / pass");
        assert!(store.secret(card.id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn idea_status_survives_a_change_of_kind() {
        let (store, dir) = temp_store("idea-status");
        let card = add(&store, "идея: annual pricing");
        store.set_idea_status(card.id, Some(IdeaStatus::Explore)).unwrap();
        store.set_kind(card.id, Kind::Goal).unwrap();
        store.set_kind(card.id, Kind::Idea).unwrap();
        let loaded = store.card(card.id).unwrap().unwrap();
        assert_eq!((loaded.kind, loaded.idea_status), (Kind::Idea, Some(IdeaStatus::Explore)));
        store.set_idea_status(card.id, None).unwrap();
        assert_eq!(store.card(card.id).unwrap().unwrap().idea_status, None);
        let meta: String = store.conn.query_row("SELECT meta FROM cards WHERE id=?1", [card.id], |r| r.get(0)).unwrap();
        assert_eq!(meta, "{}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn important_ideas_come_back_first() {
        let (store, dir) = temp_store("idea-important");
        let plain = add(&store, "идея: first");
        let important = add(&store, "идея: second");
        for c in [&plain, &important] {
            store.set_archived(c.id, true).unwrap();
        }
        store.set_idea_status(important.id, Some(IdeaStatus::Important)).unwrap();
        store.conn.execute("UPDATE cards SET created_at=1, last_viewed_at=1", []).unwrap();
        let picks = store.refresh_resurfacing(100 * DAY, 0, 1).unwrap();
        assert_eq!(picks.iter().map(|p| p.id).collect::<Vec<_>>(), vec![important.id]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn weekly_review_actions_are_reversible_and_logged() {
        let (store, dir) = temp_store("weekly-review");
        let card = add(&store, "идея: review this");
        store.set_archived(card.id, true).unwrap();
        let queue = store.review_queue(now(), 15).unwrap();
        assert_eq!(queue.iter().map(|c| c.id).collect::<Vec<_>>(), vec![card.id]);

        let review_id = store.begin_review(now()).unwrap();
        let snapshot = store.review_action(card.id, ReviewAction::Snooze).unwrap();
        store.record_review_item(review_id, card.id, ReviewAction::Snooze, now()).unwrap();
        let snoozed = store.card(card.id).unwrap().unwrap();
        assert!(snoozed.review_at.is_some());
        assert_eq!(snoozed.placement, Placement::Archive);

        store.undo_review(&snapshot).unwrap();
        store.forget_review_item(review_id, card.id).unwrap();
        let restored = store.card(card.id).unwrap().unwrap();
        assert!(restored.review_at.is_none());
        assert!(restored.archived);
        store.review_action(card.id, ReviewAction::Keep).unwrap();
        store.record_review_item(review_id, card.id, ReviewAction::Keep, now()).unwrap();
        assert!(store.review_queue(now(), 15).unwrap().is_empty());
        store.finish_review(review_id, now()).unwrap();
        let (kept, snoozed): (i64, i64) =
            store.conn.query_row("SELECT kept, snoozed FROM reviews WHERE id=?1", [review_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((kept, snoozed), (1, 0));

        // Trash from the review goes through the trash, and undo brings it back.
        let snapshot = store.review_action(card.id, ReviewAction::Trash).unwrap();
        assert_eq!(store.counts().unwrap().2, 1);
        store.undo_review(&snapshot).unwrap();
        assert_eq!(store.counts().unwrap().2, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `cargo test --release search_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn search_speed() {
        let notes: usize = std::env::var("EBB_SPEED_NOTES").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
        let (store, dir) = temp_store("speed");
        // Zipf-distributed vocabulary like natural text: a few very common words,
        // a long tail of rare ones. Known words sit at chosen ranks.
        let mut vocab: Vec<String> = (0..20_000)
            .map(|i| if i % 2 == 0 { format!("слово{i}") } else { format!("word{i}") })
            .collect();
        for (rank, w) in [(0, "и"), (1, "в"), (5, "проверить"), (40, "pricing"), (300, "онбординг"), (2000, "figma"), (8000, "vpn")] {
            vocab[rank] = w.into();
        }
        let cumulative: Vec<f64> = vocab
            .iter()
            .enumerate()
            .scan(0.0, |acc, (r, _)| {
                *acc += 1.0 / (r as f64 + 1.0);
                Some(*acc)
            })
            .collect();
        let total = *cumulative.last().unwrap();
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let t = std::time::Instant::now();
        store.conn.execute_batch("BEGIN").unwrap();
        for i in 0..notes {
            // Mostly short notes, some long: 5..~400 words.
            let len = 5 + ((next() % 1000) as f64 / 1000.0).powi(3).mul_add(400.0, 0.0) as usize;
            let body: Vec<&str> = (0..len)
                .map(|_| {
                    let x = (next() % 1_000_000) as f64 / 1_000_000.0 * total;
                    &*vocab[cumulative.partition_point(|c| *c < x)]
                })
                .collect();
            let kind = ["идея: ", "", "", "", "prompt: "][i % 5];
            add(&store, &format!("{kind}Заметка {i}\n{}", body.join(" ")));
        }
        store.conn.execute_batch("COMMIT").unwrap();
        println!("inserted {notes} notes in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);

        for query in [
            "pricing", "онбординга", "figma", "vpn", "проверить pricing", "онб", "идеи за месяц", "prompts figma",
            "zzz_nothing", "ord19",
        ] {
            let q = crate::search::parse(query, now(), 0);
            let mut times = Vec::new();
            let mut n = 0;
            for _ in 0..30 {
                let t = std::time::Instant::now();
                n = store.search(&q, Scope::Live, 50).unwrap().len();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            println!("{query:>24}: {n:>2} hits, p50 {:.2} ms, p95 {:.2} ms", times[15], times[28]);
        }
        let size = std::fs::metadata(dir.join("t.db")).map(|m| m.len()).unwrap_or(0)
            + std::fs::metadata(dir.join("t.db-wal")).map(|m| m.len()).unwrap_or(0);
        println!("db+wal size: {:.1} MiB", size as f64 / 1048576.0);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn import_replace_keeps_changed_cards_and_undo() {
        let (mut store, dir) = temp_store("import");
        let notes = [planned("a", true), planned("b", false), planned("c", false)];
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(317.0, 285.0));
        store.import("sticky:", &notes, &[Some(rect), None, None], &[]).unwrap();

        let visible = store.load().unwrap();
        assert_eq!(visible.len(), 1, "only the on-layer note is loaded");
        assert_eq!(visible[0].created_at, 1_600_000_000, "original timestamps are kept");
        assert_eq!((visible[0].pos, visible[0].size), (rect.min, rect.size()), "position and size kept");

        // Untouched: everything can be replaced.
        let (keep, replace) = store.import_state("sticky:").unwrap();
        assert!(keep.is_empty());
        assert_eq!(replace.len(), 3);

        // The user moves "a" and trashes "b": those stay; only "c" is replaceable.
        let mut a = visible[0].clone();
        a.pos = egui::pos2(100.0, 100.0);
        store.save(&a).unwrap();
        let b = store.search(&crate::search::parse("note b", now(), 0), Scope::Archive, 5).unwrap()[0].id;
        store.delete(b).unwrap();
        let (keep, replace) = store.import_state("sticky:").unwrap();
        assert_eq!(keep, ["a".to_owned(), "b".to_owned()].into_iter().collect());
        assert_eq!(replace.len(), 1);

        // Re-import: "c" is replaced, "a" and "b" are not duplicated.
        let source: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|id| crate::sticky::SourceNote {
                id: (*id).into(),
                text: format!("\\id=0b1e2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d note {id}"),
                is_open: false,
                window: None,
                created_at: 0,
                updated_at: 1_650_000_000,
            })
            .collect();
        let again = plan(&source, &keep, 1_700_000_000);
        assert_eq!(again.notes.len(), 1);
        let batch = store.import("sticky:", &again.notes, &[None], &replace).unwrap();
        assert_eq!(store.counts().unwrap(), (1, 1, 1), "a on layer, new c archived, b in trash");

        assert_eq!(store.undo_import(batch).unwrap(), 1);
        assert_eq!(store.counts().unwrap(), (1, 0, 1));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
