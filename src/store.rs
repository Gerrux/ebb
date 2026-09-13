//! SQLite persistence.

use std::path::PathBuf;

use egui::{pos2, vec2};
use rusqlite::{Connection, params};

use crate::card::{Card, DEFAULT_SIZE, Kind, Parsed};

pub struct Store {
    conn: Connection,
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
        Ok(Self { conn })
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

    pub fn imported_ids(&self, prefix: &str) -> rusqlite::Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT source_id FROM imported WHERE source_id LIKE ?1 || '%'")?;
        let rows = stmt.query_map([prefix], |r| r.get::<_, String>(0))?;
        rows.map(|r| r.map(|s| s[prefix.len()..].to_owned())).collect()
    }

    /// Inserts imported notes in one transaction. `positions[i]` is `Some` for notes
    /// that go onto the layer. Returns the batch id.
    pub fn import(
        &mut self,
        prefix: &str,
        notes: &[crate::sticky::Planned],
        positions: &[Option<egui::Pos2>],
    ) -> rusqlite::Result<i64> {
        let viewed = now();
        let tx = self.conn.transaction()?;
        let batch: i64 = tx.query_row("SELECT COALESCE(MAX(batch), 0) + 1 FROM imported", [], |r| r.get(0))?;
        {
            let mut card = tx.prepare(
                "INSERT INTO cards (kind, title, body, tags, archived, x, y, w, h, created_at, updated_at, last_viewed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            let mut link = tx.prepare("INSERT INTO imported (source_id, card_id, batch) VALUES (?1, ?2, ?3)")?;
            for (n, pos) in notes.iter().zip(positions) {
                let p = pos.unwrap_or(egui::Pos2::ZERO);
                card.execute(params![
                    n.kind.as_str(),
                    n.title,
                    n.body,
                    n.tags.join(","),
                    pos.is_none(),
                    p.x,
                    p.y,
                    DEFAULT_SIZE.x,
                    DEFAULT_SIZE.y,
                    n.created_at,
                    n.updated_at,
                    // Not "viewed" in Ambient yet, except what lands on the layer now.
                    if pos.is_some() { viewed } else { n.updated_at },
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
             FROM cards WHERE archived = 0 ORDER BY updated_at",
        )?;
        let rows = stmt.query_map([], |r| {
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
        })?;
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

    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM cards WHERE id=?1", [id])?;
        Ok(())
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
            old: false,
        }
    }

    #[test]
    fn import_is_idempotent_and_undoable() {
        let (mut store, dir) = temp_store("import");
        let notes = [planned("a", true), planned("b", false)];
        let batch = store.import("sticky:", &notes, &[Some(egui::pos2(10.0, 20.0)), None]).unwrap();

        let visible = store.load().unwrap();
        assert_eq!(visible.len(), 1, "only the on-layer note is loaded");
        assert_eq!(visible[0].created_at, 1_600_000_000, "original timestamps are kept");
        let ids = store.imported_ids("sticky:").unwrap();
        assert_eq!(ids, ["a".to_owned(), "b".to_owned()].into_iter().collect());

        // A second scan of the same source plans nothing new.
        let source: Vec<_> = ["a", "b"]
            .iter()
            .map(|id| crate::sticky::SourceNote {
                id: (*id).into(),
                text: format!("\\id=0b1e2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d note {id}"),
                is_open: true,
                created_at: 0,
                updated_at: 1_650_000_000,
            })
            .collect();
        assert!(plan(&source, &ids, 10, 1_700_000_000).notes.is_empty());

        assert_eq!(store.undo_import(batch).unwrap(), 2);
        assert!(store.load().unwrap().is_empty());
        assert!(store.imported_ids("sticky:").unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
