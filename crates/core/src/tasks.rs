// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tasks: create, list and mark as done.

use std::fmt;

use rusqlite::Connection;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskId(pub i64);

#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub done: bool,
}

#[derive(Debug)]
pub enum TaskError {
    EmptyTitle,
    NotFound,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::EmptyTitle => write!(f, "the title is empty"),
            TaskError::NotFound => write!(f, "task not found"),
            TaskError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for TaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TaskError::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for TaskError {
    fn from(e: rusqlite::Error) -> Self {
        TaskError::Sqlite(e)
    }
}

pub fn create(conn: &Connection, title: &str) -> Result<TaskId, TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    conn.execute("INSERT INTO task (title) VALUES (?1)", [title])?;
    Ok(TaskId(conn.last_insert_rowid()))
}

/// Pending first (oldest created first), then done (most recently completed first).
pub fn list(conn: &Connection) -> Result<Vec<Task>, TaskError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, done_at IS NOT NULL FROM task \
         ORDER BY done_at IS NOT NULL, done_at DESC, created_at, id",
    )?;
    let tasks = stmt
        .query_map([], |r| {
            Ok(Task {
                id: TaskId(r.get(0)?),
                title: r.get(1)?,
                done: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(tasks)
}

/// Marking an already done task keeps its original completion time.
pub fn set_done(conn: &Connection, id: TaskId, done: bool) -> Result<(), TaskError> {
    let sql = if done {
        "UPDATE task SET done_at = coalesce(done_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
         WHERE id = ?1"
    } else {
        "UPDATE task SET done_at = NULL WHERE id = ?1"
    };
    match conn.execute(sql, [id.0])? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup(name: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("op-tasks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db::open(&dir.join("test.db")).unwrap();
        (dir, conn)
    }

    fn titles(conn: &Connection) -> Vec<String> {
        list(conn).unwrap().into_iter().map(|t| t.title).collect()
    }

    fn done_at(conn: &Connection, id: TaskId) -> Option<String> {
        conn.query_row("SELECT done_at FROM task WHERE id = ?1", [id.0], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn create_trims_title() {
        let (dir, conn) = setup("trim");
        create(&conn, "  buy milk \n").unwrap();
        assert_eq!(titles(&conn), ["buy milk"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_title_is_rejected_and_not_inserted() {
        let (dir, conn) = setup("empty");
        assert!(matches!(create(&conn, ""), Err(TaskError::EmptyTitle)));
        assert!(matches!(create(&conn, " \t\n"), Err(TaskError::EmptyTitle)));
        assert!(list(&conn).unwrap().is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_orders_pending_then_done_newest_first() {
        let (dir, conn) = setup("order");
        let ids: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|t| create(&conn, t).unwrap())
            .collect();
        // Fixed timestamps: real ones could tie within a millisecond.
        for (id, at) in [
            (ids[0], "2026-01-01T00:00:01Z"),
            (ids[2], "2026-01-01T00:00:02Z"),
        ] {
            conn.execute("UPDATE task SET done_at = ?1 WHERE id = ?2", (at, id.0))
                .unwrap();
        }
        assert_eq!(titles(&conn), ["b", "d", "c", "a"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_is_idempotent_and_keeps_done_at() {
        let (dir, conn) = setup("idem");
        let id = create(&conn, "a").unwrap();
        conn.execute("UPDATE task SET done_at = '2026-01-01T00:00:00Z'", [])
            .unwrap();
        set_done(&conn, id, true).unwrap();
        assert_eq!(done_at(&conn, id).as_deref(), Some("2026-01-01T00:00:00Z"));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_sets_and_clears() {
        let (dir, conn) = setup("toggle");
        let id = create(&conn, "a").unwrap();
        set_done(&conn, id, true).unwrap();
        assert!(done_at(&conn, id).is_some());
        assert!(list(&conn).unwrap()[0].done);
        set_done(&conn, id, false).unwrap();
        assert!(done_at(&conn, id).is_none());
        assert!(!list(&conn).unwrap()[0].done);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_id_is_not_found() {
        let (dir, conn) = setup("missing");
        assert!(matches!(
            set_done(&conn, TaskId(42), true),
            Err(TaskError::NotFound)
        ));
        assert!(matches!(
            set_done(&conn, TaskId(42), false),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
