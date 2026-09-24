//! Numbered schema migrations.
//!
//! The schema version is SQLite's own `PRAGMA user_version` (0 in a new file).
//! `MIGRATIONS[n]` brings a database from version `n` to `n + 1`; each step
//! runs in its own `BEGIN IMMEDIATE` transaction that also writes the new
//! version, so a failed step leaves the file at the previous version, intact.
//!
//! Step 0 → 1 is the baseline: every build before versioning left its database
//! at version 0, whatever columns it had, so this one step (and only this one)
//! is idempotent — `IF NOT EXISTS` and `pragma_table_info` checks bring an empty
//! file or any older schema to the same result. Later steps are plain SQL for
//! exactly the version before them, with no guards.
//!
//! To change the schema: append a step to `MIGRATIONS`, never edit or reorder
//! the existing ones (they have already run on users' databases).
//!
//! `migrate` runs before the first frame: when the file is current it costs one
//! PRAGMA read and nothing else.

use rusqlite::{Connection, TransactionBehavior, params};

type Migration = fn(&Connection) -> rusqlite::Result<()>;

const MIGRATIONS: &[Migration] = &[baseline];

/// The schema version this build writes and understands.
pub(super) const LATEST: i64 = MIGRATIONS.len() as i64;

/// Setting written by builds before versioning once they had cleared review
/// dates (`clear_review_dates`). Read only by the baseline now.
pub(super) const SET_REVIEW_DATES_CLEARED: &str = "migrated.review_dates";

fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
}

/// The file was written by a newer Ebb: its schema may carry meaning this build
/// would lose, so it isn't opened at all.
fn too_new(version: i64) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(format!(
            "база заметок создана более новой версией Ebb (схема {version}, эта версия знает до {LATEST}). Обнови Ebb."
        )),
    )
}

/// Brings the database to `LATEST`. Refuses a database newer than that.
pub(super) fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let mut version = user_version(conn)?;
    while version != LATEST {
        if version > LATEST {
            return Err(too_new(version));
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Another connection (the bar, the library) may have migrated meanwhile:
        // read the version again under the write lock.
        let current = user_version(&tx)?;
        if current == version {
            MIGRATIONS[version as usize](&tx)?;
            version += 1;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        } else {
            version = current;
        }
    }
    Ok(())
}

/// 0 → 1: the full schema as of the first versioned build, reached from an
/// empty file or from any database an earlier build left behind.
fn baseline(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cards (
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
    let has_fts: bool = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name='cards_fts'",
        [],
        |r| r.get(0),
    )?;
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
        conn.execute(
            "INSERT INTO cards_fts(cards_fts, rank) VALUES ('rank', 'bm25(8.0, 1.0, 4.0)')",
            [],
        )?;
        conn.execute("INSERT INTO cards_fts(cards_fts) VALUES ('rebuild')", [])?;
    }
    for (table, name, definition) in [
        // Trash: deleted cards keep their row for TRASH_DAYS.
        ("cards", "deleted_at", "INTEGER"),
        // A color picked by hand (card::Tint); NULL takes the kind's.
        ("cards", "tint", "TEXT"),
        // A Private card's value, DPAPI-encrypted (vault).
        ("cards", "secret", "BLOB"),
        ("cards", "placement", "TEXT NOT NULL DEFAULT 'manual'"),
        ("cards", "review_at", "INTEGER"),
        ("cards", "last_resurfaced_at", "INTEGER"),
        ("cards", "resurface_count", "INTEGER NOT NULL DEFAULT 0"),
        ("cards", "ignored_count", "INTEGER NOT NULL DEFAULT 0"),
        ("cards", "priority", "INTEGER NOT NULL DEFAULT 0"),
        // Stacking order on the layer: higher is drawn above.
        ("cards", "z", "INTEGER NOT NULL DEFAULT 0"),
        // Folded to one line on the layer (card::Card::collapsed).
        ("cards", "collapsed", "INTEGER NOT NULL DEFAULT 0"),
        // Fields of a card's kind as JSON, read and written with SQLite's
        // json functions: {"idea_status": "explore"}. Kept across kind changes.
        ("cards", "meta", "TEXT NOT NULL DEFAULT '{}'"),
        // Import review (spec 08): done with in the review, or not yet.
        ("imported", "reviewed", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let exists: bool = conn.query_row(
            "SELECT count(*) FROM pragma_table_info(?1) WHERE name=?2",
            [table, name],
            |r| r.get(0),
        )?;
        if !exists {
            conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS cards_kind_created ON cards(kind, created_at);
         CREATE INDEX IF NOT EXISTS cards_created ON cards(created_at);
         CREATE INDEX IF NOT EXISTS cards_updated ON cards(updated_at);
         CREATE INDEX IF NOT EXISTS cards_deleted ON cards(deleted_at) WHERE deleted_at IS NOT NULL;
         CREATE INDEX IF NOT EXISTS cards_review ON cards(review_at) WHERE review_at IS NOT NULL;
         CREATE INDEX IF NOT EXISTS review_items_card ON review_items(card_id, at);
         CREATE INDEX IF NOT EXISTS imported_card ON imported(card_id);",
    )?;
    let cleared: bool = conn.query_row(
        "SELECT count(*) FROM settings WHERE key=?1",
        [SET_REVIEW_DATES_CLEARED],
        |r| r.get(0),
    )?;
    if !cleared {
        clear_review_dates(conn)?;
    }
    Ok(())
}

/// Older builds recorded "don't ask for 8 weeks" (and "later") of the weekly
/// review in `review_at`, which Rediscover reads as a reminder: such a card came
/// back as "Напоминание на сегодня". Clears the dates that match a review
/// decision to the minute; reminders set any other way stay. The periods are
/// the ones those builds used, frozen here: `REVIEW_*_DAYS` may change later.
fn clear_review_dates(conn: &Connection) -> rusqlite::Result<()> {
    const DAY: i64 = 86_400;
    const PAUSE_DAYS: i64 = 56;
    const LATER_DAYS: i64 = 14;
    conn.execute(
        "UPDATE cards SET review_at=NULL WHERE review_at IS NOT NULL AND EXISTS (
             SELECT 1 FROM review_items ri WHERE ri.card_id = cards.id AND abs(cards.review_at - ri.at
                 - CASE ri.action WHEN 'snooze' THEN ?1 ELSE ?2 END) <= 60
               AND ri.action IN ('keep', 'archive', 'snooze'))",
        params![LATER_DAYS * DAY, PAUSE_DAYS * DAY],
    )?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [SET_REVIEW_DATES_CLEARED],
    )?;
    Ok(())
}
