//! Automatic backup: a consistent copy of the database (`VACUUM INTO`, safe
//! while other connections write) in a `backups` folder next to it, once a week,
//! the last few kept. Private values are copied as the encrypted bytes they are
//! stored as, so a copy is no more readable than the database itself.

use std::path::{Path, PathBuf};

use super::{Store, db_path};
use crate::resurface::DAY;
use crate::search::civil_from_days;

pub const BACKUP_EVERY_DAYS: i64 = 7;
pub const BACKUPS_KEPT: usize = 4;
/// Unix time of the last backup made.
pub const SET_BACKUP_LAST: &str = "backup.last";

fn io_error(error: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

/// `%LOCALAPPDATA%\Ebb\backups`, next to the database.
pub fn backup_dir() -> PathBuf {
    db_path()
        .parent()
        .map_or_else(|| PathBuf::from("backups"), |p| p.join("backups"))
}

/// Only "ebb-YYYY-MM-DD.db": pruning must never touch a file the user put there.
fn is_backup(name: &str) -> bool {
    let Some(date) = name.strip_prefix("ebb-").and_then(|n| n.strip_suffix(".db")) else {
        return false;
    };
    date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// One backup at a time in this process: the automatic and the manual one
/// would otherwise share a temp file.
static BACKUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Backup files in `dir`, newest first (the date in the name sorts as text).
fn backups(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| is_backup(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    found.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    found
}

fn remove_stale_temps(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name
            .to_string_lossy()
            .strip_suffix(".tmp")
            .is_some_and(is_backup)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The newest backup in `dir`.
pub fn latest_backup(dir: &Path) -> Option<PathBuf> {
    backups(dir).into_iter().next()
}

impl Store {
    /// Copies the database to `dir/ebb-YYYY-MM-DD.db` (local date; a copy from
    /// the same day is replaced), notes the time and drops the oldest copies.
    pub fn backup_into(
        &self,
        dir: &Path,
        now: i64,
        utc_offset: i64,
    ) -> rusqlite::Result<PathBuf> {
        let _one_at_a_time = BACKUP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::create_dir_all(dir).map_err(io_error)?;
        let (y, m, d) = civil_from_days((now + utc_offset).div_euclid(DAY));
        let target = dir.join(format!("ebb-{y:04}-{m:02}-{d:02}.db"));
        let temp = dir.join(format!("ebb-{y:04}-{m:02}-{d:02}.db.tmp"));
        // Temp files an interrupted copy left behind, from any day. VACUUM INTO
        // refuses to write over a file, so today's must go.
        remove_stale_temps(dir);
        match std::fs::remove_file(&temp) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io_error(e)),
            _ => {}
        }
        if let Err(e) = self
            .conn
            .execute("VACUUM INTO ?1", [temp.to_string_lossy().as_ref()])
        {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&temp, &target) {
            let _ = std::fs::remove_file(&temp);
            return Err(io_error(e));
        }
        self.set_setting(SET_BACKUP_LAST, &now.to_string())?;
        for old in backups(dir).into_iter().skip(BACKUPS_KEPT) {
            let _ = std::fs::remove_file(old);
        }
        Ok(target)
    }

    /// `backup_into` when a week has passed since the last one (or its folder
    /// was emptied). An empty database is never copied: it would only push
    /// good copies out.
    pub fn backup_if_due(
        &self,
        dir: &Path,
        now: i64,
        utc_offset: i64,
    ) -> rusqlite::Result<Option<PathBuf>> {
        let any_cards: bool = self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM cards)", [], |r| r.get(0))?;
        if !any_cards {
            return Ok(None);
        }
        let last = self
            .setting(SET_BACKUP_LAST)
            .and_then(|v| v.parse::<i64>().ok());
        // A time in the future (the clock was set back since) doesn't count as
        // recent, or backups would stop until the clock caught up.
        let recent = last.is_some_and(|t| (0..BACKUP_EVERY_DAYS * DAY).contains(&(now - t)));
        if recent && latest_backup(dir).is_some() {
            return Ok(None);
        }
        self.backup_into(dir, now, utc_offset).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Kind;

    const T0: i64 = 1_700_000_000; // 2023-11-14 22:13 UTC

    fn setup(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ebb-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Store::open_at(dir.join("t.db")).unwrap(), dir)
    }

    fn add(store: &Store, text: &str) -> crate::card::Card {
        store
            .insert(&crate::card::parse_capture(text), egui::pos2(0.0, 0.0))
            .unwrap()
    }

    fn count(store: &Store) -> i64 {
        store
            .conn
            .query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))
            .unwrap()
    }

    fn secret_bytes(store: &Store, id: i64) -> Option<Vec<u8>> {
        store
            .conn
            .query_row("SELECT secret FROM cards WHERE id=?1", [id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn copy_opens_with_the_same_cards_and_private_bytes() {
        let (store, dir) = setup("copy");
        add(&store, "first");
        add(&store, "second");
        let private = add(&store, "Wi-Fi\nhunter2");
        store.set_kind(private.id, Kind::Private).unwrap();
        let before = secret_bytes(&store, private.id);
        assert!(before.is_some());

        let path = store.backup_into(&dir.join("backups"), T0, 3 * 3600).unwrap();
        assert_eq!(path.file_name().unwrap(), "ebb-2023-11-15.db");
        let copy = Store::open_at(path).unwrap();
        assert_eq!(count(&copy), count(&store));
        assert_eq!(secret_bytes(&copy, private.id), before);
        assert_eq!(store.setting(SET_BACKUP_LAST), Some(T0.to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn due_once_a_week_or_when_the_folder_is_empty() {
        let (store, dir) = setup("due");
        let backups_dir = dir.join("backups");
        add(&store, "note");
        assert!(store.backup_if_due(&backups_dir, T0, 0).unwrap().is_some());
        let soon = T0 + (BACKUP_EVERY_DAYS * DAY - 1);
        assert!(store.backup_if_due(&backups_dir, soon, 0).unwrap().is_none());
        let later = T0 + BACKUP_EVERY_DAYS * DAY;
        assert!(store.backup_if_due(&backups_dir, later, 0).unwrap().is_some());

        std::fs::remove_dir_all(&backups_dir).unwrap();
        assert!(store.backup_if_due(&backups_dir, later + 60, 0).unwrap().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_database_makes_no_copy() {
        let (store, dir) = setup("empty");
        let backups_dir = dir.join("backups");
        assert!(store.backup_if_due(&backups_dir, T0, 0).unwrap().is_none());
        assert!(latest_backup(&backups_dir).is_none());
        assert!(!backups_dir.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_the_newest_copies_and_leaves_other_files_alone() {
        let (store, dir) = setup("prune");
        let backups_dir = dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        std::fs::write(backups_dir.join("notes.txt"), "mine").unwrap();
        add(&store, "note");
        for day in 0..6 {
            store.backup_into(&backups_dir, T0 + day * DAY, 0).unwrap();
        }
        let names: Vec<String> = backups(&backups_dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "ebb-2023-11-19.db",
                "ebb-2023-11-18.db",
                "ebb-2023-11-17.db",
                "ebb-2023-11-16.db"
            ]
        );
        assert_eq!(
            latest_backup(&backups_dir).unwrap().file_name().unwrap(),
            "ebb-2023-11-19.db"
        );
        assert!(backups_dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_dated_copies_count_as_backups() {
        let (store, dir) = setup("names");
        let backups_dir = dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        // Sort after any dated copy, so they'd be "newest" and survive, while
        // real copies were pruned - or be pruned themselves.
        for name in ["ebb-mine.db", "ebb-old.db", "ebb-2023-1-1.db"] {
            std::fs::write(backups_dir.join(name), "mine").unwrap();
        }
        add(&store, "note");
        for day in 0..5 {
            store.backup_into(&backups_dir, T0 + day * DAY, 0).unwrap();
        }
        assert_eq!(backups(&backups_dir).len(), BACKUPS_KEPT);
        for name in ["ebb-mine.db", "ebb-old.db", "ebb-2023-1-1.db"] {
            assert!(backups_dir.join(name).exists(), "{name}");
        }
        assert_eq!(
            latest_backup(&backups_dir).unwrap().file_name().unwrap(),
            "ebb-2023-11-18.db"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn leftover_temp_files_are_cleared() {
        let (store, dir) = setup("temps");
        let backups_dir = dir.join("backups");
        std::fs::create_dir_all(&backups_dir).unwrap();
        let (today, earlier) = ("ebb-2023-11-14.db.tmp", "ebb-2023-11-02.db.tmp");
        std::fs::write(backups_dir.join(today), "half").unwrap();
        std::fs::write(backups_dir.join(earlier), "half").unwrap();
        std::fs::write(backups_dir.join("mine.tmp"), "mine").unwrap();
        add(&store, "note");
        store.backup_into(&backups_dir, T0, 0).unwrap();
        assert!(!backups_dir.join(today).exists());
        assert!(!backups_dir.join(earlier).exists());
        assert!(backups_dir.join("mine.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_last_backup_in_the_future_does_not_stop_backups() {
        let (store, dir) = setup("future");
        let backups_dir = dir.join("backups");
        add(&store, "note");
        let future = T0 + 365 * DAY;
        store.backup_into(&backups_dir, future, 0).unwrap();
        // The clock was set back a year.
        assert!(store.backup_if_due(&backups_dir, T0, 0).unwrap().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_backups_do_not_collide() {
        let (store, dir) = setup("race");
        let backups_dir = dir.join("backups");
        add(&store, "note");
        let db = dir.join("t.db");
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (db, backups_dir) = (db.clone(), backups_dir.clone());
                std::thread::spawn(move || {
                    Store::open_at(db).unwrap().backup_into(&backups_dir, T0, 0)
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap().unwrap();
        }
        assert_eq!(backups(&backups_dir).len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
