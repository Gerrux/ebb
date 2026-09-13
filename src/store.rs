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
        let path = db_path();
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
             );",
        )?;
        Ok(Self { conn })
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
