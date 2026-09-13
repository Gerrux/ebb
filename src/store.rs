//! SQLite persistence.

use std::path::PathBuf;

use egui::{pos2, vec2};
use rusqlite::{Connection, params};

use crate::card::{Card, DEFAULT_SIZE, Kind, Parsed};

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

/// Row of `SELECT id, kind, title, body, tags, pinned, archived, x, y, w, h, created_at`.
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
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn db_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("Ambient").join("ambient.db")
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
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS cards_kind_created ON cards(kind, created_at);
             CREATE INDEX IF NOT EXISTS cards_created ON cards(created_at);
             CREATE INDEX IF NOT EXISTS cards_updated ON cards(updated_at);
             CREATE INDEX IF NOT EXISTS cards_deleted ON cards(deleted_at) WHERE deleted_at IS NOT NULL;",
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
            "SELECT id, kind, title, body, tags, pinned, archived, x, y, w, h, created_at FROM cards WHERE id=?1",
        )?;
        let mut rows = stmt.query_map([id], card_row)?;
        rows.next().transpose()
    }

    /// Records that the user looked at a card (input for resurfacing).
    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET last_viewed_at=?2 WHERE id=?1", params![id, now()])?;
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
    /// - source ids to leave alone: their card was changed in Ambient (moved, edited,
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
            let mut link = tx.prepare("INSERT INTO imported (source_id, card_id, batch) VALUES (?1, ?2, ?3)")?;
            for (n, rect) in notes.iter().zip(rects) {
                let r = rect.unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, DEFAULT_SIZE));
                card.execute(params![
                    n.kind.as_str(),
                    n.title,
                    n.body,
                    n.tags.join(","),
                    rect.is_none(),
                    r.min.x,
                    r.min.y,
                    r.width(),
                    r.height(),
                    n.created_at,
                    n.updated_at,
                    // Not "viewed" in Ambient yet, except what lands on the layer now.
                    // One second back: timestamps are in seconds, and a move right
                    // after the import must still count as a change (import_state).
                    if rect.is_some() { (viewed - 1).max(n.updated_at) } else { n.updated_at },
                ])?;
                link.execute(params![format!("{prefix}{}", n.source_id), tx.last_insert_rowid(), batch])?;
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
            "SELECT id, kind, title, body, tags, pinned, archived, x, y, w, h, created_at
             FROM cards WHERE archived = 0 AND deleted_at IS NULL ORDER BY updated_at",
        )?;
        let rows = stmt.query_map([], card_row)?;
        rows.collect()
    }

    pub fn insert(&self, p: &Parsed, pos: egui::Pos2) -> rusqlite::Result<Card> {
        let t = now();
        self.conn.execute(
            "INSERT INTO cards (kind, title, body, tags, x, y, w, h, created_at, updated_at, last_viewed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9)",
            params![
                p.kind.as_str(),
                p.title,
                p.body,
                p.tags.join(","),
                pos.x,
                pos.y,
                DEFAULT_SIZE.x,
                DEFAULT_SIZE.y,
                t
            ],
        )?;
        Ok(Card {
            id: self.conn.last_insert_rowid(),
            kind: p.kind,
            title: p.title.clone(),
            body: p.body.clone(),
            tags: p.tags.clone(),
            pinned: false,
            archived: false,
            pos,
            size: DEFAULT_SIZE,
            created_at: t,
        })
    }

    pub fn save(&self, c: &Card) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE cards SET kind=?2, title=?3, body=?4, tags=?5, pinned=?6, archived=?7,
                 x=?8, y=?9, w=?10, h=?11, updated_at=?12 WHERE id=?1",
            params![
                c.id,
                c.kind.as_str(),
                c.title,
                c.body,
                c.tags.join(","),
                c.pinned,
                c.archived,
                c.pos.x,
                c.pos.y,
                c.size.x,
                c.size.y,
                now()
            ],
        )?;
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

    pub fn set_archived(&self, id: i64, archived: bool) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE cards SET archived=?2 WHERE id=?1", params![id, archived])?;
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
        let dir = std::env::temp_dir().join(format!("ambient-test-{name}-{}", std::process::id()));
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
        let mut secret = add(&store, "секрет: Wi-Fi\nпароль hunter2");

        // Case-insensitive Cyrillic, inflected form via stemming + prefix.
        assert_eq!(find(&store, "ОНБОРДИНГА").len(), 1);
        assert_eq!(find(&store, "vpn").len(), 1);
        assert_eq!(find(&store, "#infra").len(), 1);
        assert_eq!(find(&store, "идеи").len(), 1, "kind filter without text");
        // Private bodies are searchable but never shown.
        let hits = find(&store, "hunter2");
        assert_eq!(hits, ["Wi-Fi|"]);
        // Substring fallback for mid-word matches.
        assert_eq!(find(&store, "board").len(), 0, "LIKE is on title+body; 'board' isn't there");
        assert_eq!(find(&store, "taging").len(), 1);

        // Edits reindex; the trash is out of normal search but can be searched and
        // restored; purging drops the card from the index.
        secret.body = "пароль changed".into();
        store.save(&secret).unwrap();
        assert!(find(&store, "hunter2").is_empty());
        assert_eq!(find(&store, "changed").len(), 1);
        store.delete(secret.id).unwrap();
        assert!(find(&store, "changed").is_empty());
        let q = crate::search::parse("changed", now(), 0);
        let trashed = store.search(&q, Scope::Trash, 20).unwrap();
        assert_eq!(trashed.len(), 1);
        assert!(trashed[0].deleted_at.is_some());
        assert_eq!(store.counts().unwrap().2, 1);
        store.restore(secret.id).unwrap();
        assert_eq!(find(&store, "changed").len(), 1);
        store.set_archived(secret.id, true).unwrap();
        assert_eq!(store.search(&q, Scope::Archive, 20).unwrap().len(), 1);
        store.delete(secret.id).unwrap();
        store.purge(secret.id).unwrap();
        assert!(store.search(&q, Scope::Trash, 20).unwrap().is_empty());
        assert!(store.card(secret.id).unwrap().is_none());

        // Reopening an existing database without the index builds it.
        store.conn.execute_batch("DROP TABLE cards_fts;").unwrap();
        drop(store);
        let store = Store::open_at(dir.join("t.db")).unwrap();
        assert_eq!(find(&store, "онбординг").len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `cargo test --release search_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn search_speed() {
        let notes: usize = std::env::var("AMBIENT_SPEED_NOTES").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
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
