// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! SQLite connection setup and schema migrations.

use std::fmt;
use std::path::Path;
use std::time::Duration;

pub use rusqlite::Connection;

#[derive(Debug)]
pub enum DbError {
    Sqlite(rusqlite::Error),
    NewerSchema {
        found: u32,
        supported: u32,
    },
    Io(std::io::Error),
    /// The export destination is the open database itself.
    SameFile,
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Sqlite(e) => write!(f, "database error: {e}"),
            DbError::NewerSchema { found, supported } => write!(
                f,
                "database schema v{found} is newer than supported v{supported}"
            ),
            DbError::Io(e) => write!(f, "file error: {e}"),
            DbError::SameFile => write!(f, "the destination is the open database itself"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Sqlite(e) => Some(e),
            DbError::Io(e) => Some(e),
            DbError::NewerSchema { .. } | DbError::SameFile => None,
        }
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_baseline.sql"),
    include_str!("../migrations/0002_tasks.sql"),
    include_str!("../migrations/0003_task_due_date.sql"),
    include_str!("../migrations/0004_lists.sql"),
    include_str!("../migrations/0005_task_note.sql"),
];

pub fn open(path: &Path) -> Result<Connection, DbError> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    migrate(&mut conn, MIGRATIONS)?;
    Ok(conn)
}

/// Writes a consistent copy of the database to `dest`, replacing any file there.
pub fn export(conn: &Connection, dest: &Path) -> Result<(), DbError> {
    if let Some(src) = conn.path().filter(|p| !p.is_empty()) {
        let same = Path::new(src) == dest
            || matches!((std::fs::canonicalize(src), std::fs::canonicalize(dest)),
                (Ok(a), Ok(b)) if a == b);
        if same {
            return Err(DbError::SameFile);
        }
    }
    // Copy beside the target and rename, so an existing file survives a failed copy.
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    // If removal fails, VACUUM INTO reports the real error.
    let _ = std::fs::remove_file(&tmp);
    let result = conn
        .execute("VACUUM INTO ?1", [tmp.to_string_lossy()])
        .map_err(DbError::from)
        .and_then(|_| std::fs::rename(&tmp, dest).map_err(DbError::from));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<(), DbError> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let supported = migrations.len() as u32;
    if current > supported {
        return Err(DbError::NewerSchema {
            found: current,
            supported,
        });
    }
    if current == supported {
        return Ok(());
    }

    // VACUUM INTO stays consistent in WAL mode, unlike copying the file.
    if current > 0
        && let Some(path) = conn.path().filter(|p| !p.is_empty())
    {
        let backup = format!("{path}.bak-v{current}");
        // If removal fails, VACUUM INTO reports the real error.
        let _ = std::fs::remove_file(&backup);
        conn.execute("VACUUM INTO ?1", [&backup])?;
    }

    for (i, sql) in migrations.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn setup(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("op-db-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");
        (dir, db)
    }

    fn version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| r.get::<_, u32>(0),
        )
        .unwrap()
            > 0
    }

    fn backups(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".bak-v")
            })
            .count()
    }

    #[test]
    fn fresh_db_is_migrated_and_configured() {
        let (dir, db) = setup("fresh");
        let conn = open(&db).unwrap();
        assert_eq!(version(&conn), MIGRATIONS.len() as u32);
        assert!(has_table(&conn, "settings"));
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        assert_eq!(backups(&dir), 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopen_up_to_date_makes_no_backup() {
        let (dir, db) = setup("reopen");
        drop(open(&db).unwrap());
        let conn = open(&db).unwrap();
        assert_eq!(version(&conn), MIGRATIONS.len() as u32);
        assert_eq!(backups(&dir), 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_schema_is_rejected_untouched() {
        let (dir, db) = setup("newer");
        let conn = open(&db).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);
        let supported = MIGRATIONS.len() as u32;
        assert!(matches!(
            open(&db),
            Err(DbError::NewerSchema {
                found: 99,
                supported: s
            }) if s == supported
        ));
        let conn = Connection::open(&db).unwrap();
        assert_eq!(version(&conn), 99);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_migration_is_backed_up_first() {
        let (dir, db) = setup("backup");
        let mut conn = open(&db).unwrap();
        let extra = "CREATE TABLE extra (id INTEGER PRIMARY KEY) STRICT;";
        let all = [MIGRATIONS, &[extra]].concat();
        let before = MIGRATIONS.len() as u32;
        migrate(&mut conn, &all).unwrap();
        assert_eq!(version(&conn), before + 1);
        drop(conn);
        let bak = dir.join(format!("test.db.bak-v{before}"));
        assert!(bak.exists());
        let b = Connection::open(&bak).unwrap();
        assert_eq!(version(&b), before);
        assert!(!has_table(&b, "extra"));
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_migration_rolls_back() {
        let (dir, db) = setup("fail");
        let mut conn = open(&db).unwrap();
        let all = [MIGRATIONS, &["CREATE TABLE broken ("]].concat();
        assert!(migrate(&mut conn, &all).is_err());
        assert_eq!(version(&conn), MIGRATIONS.len() as u32);
        assert!(!has_table(&conn, "broken"));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_is_an_openable_copy() {
        let (dir, db) = setup("export");
        let conn = open(&db).unwrap();
        conn.execute("INSERT INTO task (title) VALUES ('copied')", [])
            .unwrap();
        let dest = dir.join("copy.db");
        export(&conn, &dest).unwrap();
        assert!(!dir.join("copy.db.tmp").exists());
        let copy = open(&dest).unwrap();
        assert_eq!(version(&copy), version(&conn));
        let n: u32 = copy
            .query_row("SELECT count(*) FROM task WHERE title='copied'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
        drop((copy, conn));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_replaces_an_existing_file() {
        let (dir, db) = setup("export-over");
        let conn = open(&db).unwrap();
        let dest = dir.join("copy.db");
        std::fs::write(&dest, b"old").unwrap();
        export(&conn, &dest).unwrap();
        assert_eq!(version(&open(&dest).unwrap()), version(&conn));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_onto_the_open_db_is_refused() {
        let (dir, db) = setup("export-same");
        let conn = open(&db).unwrap();
        assert!(matches!(export(&conn, &db), Err(DbError::SameFile)));
        // A different spelling of the same path is caught too.
        let alias = dir.join(".").join("test.db");
        assert!(matches!(export(&conn, &alias), Err(DbError::SameFile)));
        assert_eq!(version(&conn), MIGRATIONS.len() as u32);
        assert!(!dir.join("test.db.tmp").exists());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
