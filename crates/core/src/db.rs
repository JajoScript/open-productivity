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

/// Name of the database file inside an instance folder.
pub const FILE_NAME: &str = "open-productivity.db";

/// How many pre-migration backups to keep per database.
const BACKUPS_KEPT: usize = 2;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_baseline.sql"),
    include_str!("../migrations/0002_tasks.sql"),
    include_str!("../migrations/0003_task_due_date.sql"),
    include_str!("../migrations/0004_lists.sql"),
    include_str!("../migrations/0005_task_note.sql"),
    include_str!("../migrations/0006_task_in_progress.sql"),
    include_str!("../migrations/0007_task_priority.sql"),
    include_str!("../migrations/0008_habits.sql"),
    include_str!("../migrations/0009_task_time.sql"),
    include_str!("../migrations/0010_task_recurrence.sql"),
    include_str!("../migrations/0011_study.sql"),
    include_str!("../migrations/0012_subject_archived.sql"),
    include_str!("../migrations/0013_task_subject.sql"),
    include_str!("../migrations/0014_session_topic.sql"),
    include_str!("../migrations/0015_study_active.sql"),
    include_str!("../migrations/0016_study_paused.sql"),
];

pub fn open(path: &Path) -> Result<Connection, DbError> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // An instance is any folder the user picks, so its file may come from someone else: its
    // views, triggers and defaults may only call SQLite's own harmless functions.
    conn.pragma_update(None, "trusted_schema", "OFF")?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    migrate(&mut conn, MIGRATIONS)?;
    Ok(conn)
}

/// A migrated in-memory database for tests that do not exercise files.
#[cfg(test)]
pub(crate) fn test_conn() -> Connection {
    open(Path::new(":memory:")).unwrap()
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

/// Deletes all but the newest `BACKUPS_KEPT` `<db>.bak-v<N>` files; failures are ignored.
fn prune_backups(db: &Path, newest: u32) {
    let (Some(dir), Some(name)) = (db.parent(), db.file_name()) else {
        return;
    };
    let prefix = format!("{}.bak-v", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<(u32, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let n = e
                .file_name()
                .to_string_lossy()
                .strip_prefix(&prefix)?
                .parse()
                .ok()?;
            Some((n, e.path()))
        })
        .filter(|(n, _)| *n != newest)
        .collect();
    found.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    // The fresh backup always counts as one of the kept files.
    for (_, path) in found.into_iter().skip(BACKUPS_KEPT - 1) {
        let _ = std::fs::remove_file(path);
    }
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
        prune_backups(Path::new(path), current);
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
        let trusted: i64 = conn
            .pragma_query_value(None, "trusted_schema", |r| r.get(0))
            .unwrap();
        assert_eq!(trusted, 0);
        assert_eq!(backups(&dir), 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_time_migration_keeps_existing_tasks() {
        let (dir, db) = setup("task-time");
        let mut conn = Connection::open(&db).unwrap();
        migrate(&mut conn, &MIGRATIONS[..8]).unwrap();
        conn.execute(
            "INSERT INTO task (title, due_date) VALUES ('old', '2026-11-08'), ('bare', NULL)",
            [],
        )
        .unwrap();
        drop(conn);
        let conn = open(&db).unwrap();
        assert_eq!(version(&conn), MIGRATIONS.len() as u32);
        type Row = (String, Option<String>, Option<String>, Option<String>);
        let rows: Vec<Row> = conn
            .prepare("SELECT title, due_date, due_time, end_time FROM task ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                ("old".into(), Some("2026-11-08".into()), None, None),
                ("bare".into(), None, None, None)
            ]
        );
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
    fn only_the_newest_backups_are_kept() {
        let (dir, db) = setup("prune");
        let mut conn = open(&db).unwrap();
        for f in [
            "test.db.bak-v1",
            "test.db.bak-v2",
            "test.db.bak-v90",
            "other.db.bak-v1",
            "test.db.bak-vX",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let extra = "CREATE TABLE extra (id INTEGER PRIMARY KEY) STRICT;";
        migrate(&mut conn, &[MIGRATIONS, &[extra]].concat()).unwrap();
        drop(conn);
        let new = format!("test.db.bak-v{}", MIGRATIONS.len());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".bak-v"))
            .collect();
        names.sort();
        let mut want = vec!["other.db.bak-v1", "test.db.bak-v90", "test.db.bak-vX", &new];
        want.sort();
        assert_eq!(names, want);
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
