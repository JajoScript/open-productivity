// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tasks: create, list, mark as done and due dates.

use std::fmt;

use chrono::{DateTime, Days, FixedOffset, NaiveDate, NaiveTime, SecondsFormat, Utc};
use rusqlite::Connection;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskId(pub i64);

#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Today,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DueLabel {
    Overdue,
    Today,
    Tomorrow,
    On(NaiveDate),
}

#[derive(Debug)]
pub enum TaskError {
    EmptyTitle,
    NotFound,
    BadDate(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::EmptyTitle => write!(f, "the title is empty"),
            TaskError::NotFound => write!(f, "task not found"),
            TaskError::BadDate(d) => write!(f, "invalid stored date: {d}"),
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

pub fn create(
    conn: &Connection,
    title: &str,
    view: View,
    today: NaiveDate,
) -> Result<TaskId, TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    let due = (view == View::Today).then_some(today);
    conn.execute(
        "INSERT INTO task (title, due_date) VALUES (?1, ?2)",
        (title, due.map(|d| d.to_string())),
    )?;
    Ok(TaskId(conn.last_insert_rowid()))
}

/// All: pending first (oldest created first), then done (most recently completed first).
/// Today: pending due today or earlier (by due date, then creation), then done today
/// (local day of `now`) with a due date today or earlier, most recent first.
pub fn list(
    conn: &Connection,
    view: View,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Task>, TaskError> {
    let today = now.date_naive();
    // done_at is UTC text in SQLite's own format, so text comparison orders instants.
    // shortcut: the fixed offset of `now` is used for midnight, so on a DST-change day the
    // boundary can be off by an hour; upgrade with a tz-aware type if it matters.
    let midnight = today
        .and_time(NaiveTime::MIN)
        .and_local_timezone(*now.offset())
        .single()
        .ok_or_else(|| TaskError::BadDate(today.to_string()))?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let sql = match view {
        View::All => {
            "SELECT id, title, done_at IS NOT NULL, due_date FROM task              ORDER BY done_at IS NOT NULL, done_at DESC, created_at, id"
        }
        View::Today => {
            "SELECT id, title, done_at IS NOT NULL, due_date FROM task              WHERE due_date <= ?1 AND (done_at IS NULL OR done_at >= ?2)              ORDER BY done_at IS NOT NULL, done_at DESC, due_date, created_at, id"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map = |r: &rusqlite::Row| {
        Ok((
            Task {
                id: TaskId(r.get(0)?),
                title: r.get(1)?,
                done: r.get(2)?,
                due: None,
            },
            r.get::<_, Option<String>>(3)?,
        ))
    };
    let rows = match view {
        View::All => stmt.query_map([], map)?,
        View::Today => stmt.query_map((today.to_string(), midnight), map)?,
    }
    .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(mut t, due)| {
            t.due = due
                .map(|d| d.parse().map_err(|_| TaskError::BadDate(d)))
                .transpose()?;
            Ok(t)
        })
        .collect()
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

pub fn set_due_date(
    conn: &Connection,
    id: TaskId,
    due: Option<NaiveDate>,
) -> Result<(), TaskError> {
    match conn.execute(
        "UPDATE task SET due_date = ?1 WHERE id = ?2",
        (due.map(|d| d.to_string()), id.0),
    )? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

pub fn due_label(task: &Task, today: NaiveDate) -> Option<DueLabel> {
    let due = task.due?;
    Some(if due == today {
        DueLabel::Today
    } else if due < today && !task.done {
        DueLabel::Overdue
    } else if today.checked_add_days(Days::new(1)) == Some(due) {
        DueLabel::Tomorrow
    } else {
        DueLabel::On(due)
    })
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

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    const NOW: &str = "2026-06-15T12:00:00+00:00";

    fn mk(conn: &Connection, title: &str) -> Result<TaskId, TaskError> {
        create(conn, title, View::All, d("2026-06-15"))
    }

    fn ls(conn: &Connection) -> Vec<Task> {
        list(conn, View::All, at(NOW)).unwrap()
    }

    fn today_titles(conn: &Connection, now: &str) -> Vec<String> {
        list(conn, View::Today, at(now))
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect()
    }

    fn task(due: Option<&str>, done: bool) -> Task {
        Task {
            id: TaskId(1),
            title: String::new(),
            done,
            due: due.map(d),
        }
    }

    fn titles(conn: &Connection) -> Vec<String> {
        ls(conn).into_iter().map(|t| t.title).collect()
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
        mk(&conn, "  buy milk \n").unwrap();
        assert_eq!(titles(&conn), ["buy milk"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_title_is_rejected_and_not_inserted() {
        let (dir, conn) = setup("empty");
        assert!(matches!(mk(&conn, ""), Err(TaskError::EmptyTitle)));
        assert!(matches!(mk(&conn, " \t\n"), Err(TaskError::EmptyTitle)));
        assert!(ls(&conn).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_orders_pending_then_done_newest_first() {
        let (dir, conn) = setup("order");
        let ids: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|t| mk(&conn, t).unwrap())
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
        let id = mk(&conn, "a").unwrap();
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
        let id = mk(&conn, "a").unwrap();
        set_done(&conn, id, true).unwrap();
        assert!(done_at(&conn, id).is_some());
        assert!(ls(&conn)[0].done);
        set_done(&conn, id, false).unwrap();
        assert!(done_at(&conn, id).is_none());
        assert!(!ls(&conn)[0].done);
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

    #[test]
    fn create_sets_due_by_view() {
        let (dir, conn) = setup("create-due");
        create(&conn, "t", View::Today, d("2026-06-15")).unwrap();
        create(&conn, "a", View::All, d("2026-06-15")).unwrap();
        let l = ls(&conn);
        assert_eq!(l[0].due, Some(d("2026-06-15")));
        assert_eq!(l[1].due, None);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_includes_overdue_and_today_overdue_first() {
        let (dir, conn) = setup("today");
        for (title, due) in [
            ("today", Some("2026-06-15")),
            ("future", Some("2026-06-16")),
            ("none", None),
            ("late", Some("2026-06-10")),
        ] {
            let id = mk(&conn, title).unwrap();
            set_due_date(&conn, id, due.map(d)).unwrap();
        }
        assert_eq!(today_titles(&conn, NOW), ["late", "today"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_lists_done_today_after_pending() {
        let (dir, conn) = setup("today-done");
        for (title, due, done) in [
            ("done-today", "2026-06-15", Some("2026-06-15T08:00:00.000Z")),
            ("done-later", "2026-06-14", Some("2026-06-15T09:00:00.000Z")),
            (
                "done-yesterday",
                "2026-06-15",
                Some("2026-06-14T23:59:59.999Z"),
            ),
            (
                "done-future-due",
                "2026-06-20",
                Some("2026-06-15T10:00:00.000Z"),
            ),
            ("pending", "2026-06-15", None),
        ] {
            let id = mk(&conn, title).unwrap();
            conn.execute(
                "UPDATE task SET due_date = ?1, done_at = ?2 WHERE id = ?3",
                (due, done, id.0),
            )
            .unwrap();
        }
        assert_eq!(
            today_titles(&conn, NOW),
            ["pending", "done-later", "done-today"]
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_midnight_uses_the_offset_of_now() {
        let (dir, conn) = setup("today-offset");
        for (title, at) in [
            ("before", "2026-06-15T02:59:59.999Z"),
            ("after", "2026-06-15T03:00:00.000Z"),
        ] {
            let id = mk(&conn, title).unwrap();
            conn.execute(
                "UPDATE task SET due_date = '2026-06-15', done_at = ?1 WHERE id = ?2",
                (at, id.0),
            )
            .unwrap();
        }
        // 22:00 at UTC-3 on the 15th: local midnight is 03:00Z.
        assert_eq!(today_titles(&conn, "2026-06-15T22:00:00-03:00"), ["after"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_due_date_sets_clears_and_not_found() {
        let (dir, conn) = setup("set-due");
        let id = mk(&conn, "a").unwrap();
        set_due_date(&conn, id, Some(d("2026-07-01"))).unwrap();
        assert_eq!(ls(&conn)[0].due, Some(d("2026-07-01")));
        set_due_date(&conn, id, None).unwrap();
        assert_eq!(ls(&conn)[0].due, None);
        assert!(matches!(
            set_due_date(&conn, TaskId(42), None),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_date_is_rejected_by_the_database() {
        let (dir, conn) = setup("bad-date");
        mk(&conn, "a").unwrap();
        assert!(
            conn.execute("UPDATE task SET due_date = '2026-13-40'", [])
                .is_err()
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn due_label_cases() {
        let today = d("2026-06-15");
        assert_eq!(due_label(&task(None, false), today), None);
        let l = |due, done| due_label(&task(Some(due), done), today);
        assert_eq!(l("2026-06-14", false), Some(DueLabel::Overdue));
        assert_eq!(l("2026-06-14", true), Some(DueLabel::On(d("2026-06-14"))));
        assert_eq!(l("2026-06-15", false), Some(DueLabel::Today));
        assert_eq!(l("2026-06-15", true), Some(DueLabel::Today));
        assert_eq!(l("2026-06-16", false), Some(DueLabel::Tomorrow));
        assert_eq!(l("2026-06-17", false), Some(DueLabel::On(d("2026-06-17"))));
    }
}
