// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tasks: create, list, mark as done, due dates, lists, edit and delete.

use std::fmt;

use chrono::{DateTime, Days, FixedOffset, NaiveDate, NaiveTime, SecondsFormat, Utc};
use rusqlite::{Connection, ffi};

use crate::calendar;
use crate::lists::ListId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskId(pub i64);

#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub note: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    /// The list id and its name.
    pub list: Option<(ListId, String)>,
    pub priority: Priority,
}

/// The Eisenhower marks, set by hand.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Priority {
    pub urgent: bool,
    pub important: bool,
}

impl Priority {
    /// The four quadrants in the order `eisenhower` returns them: do, schedule, delegate,
    /// evaluate.
    pub const QUADRANTS: [Priority; 4] = [
        Priority {
            urgent: true,
            important: true,
        },
        Priority {
            urgent: false,
            important: true,
        },
        Priority {
            urgent: true,
            important: false,
        },
        Priority {
            urgent: false,
            important: false,
        },
    ];

    /// Index of this priority in `QUADRANTS`.
    pub fn quadrant(self) -> usize {
        usize::from(!self.urgent) + 2 * usize::from(!self.important)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    /// Due today.
    Today,
    /// Due after today.
    Scheduled,
    /// Without a list and without a due date.
    Inbox,
    /// Due before today.
    Overdue,
    All,
    List(ListId),
}

impl View {
    /// The only place each view's rule is written: an SQL condition on the task `t` and
    /// on `p.today`, shared by `list` and `counts` so they never disagree.
    fn rule(self) -> String {
        match self {
            View::Today => "t.due_date = p.today".into(),
            View::Scheduled => "t.due_date > p.today".into(),
            View::Inbox => "t.list_id IS NULL AND t.due_date IS NULL".into(),
            View::Overdue => "t.due_date < p.today".into(),
            View::All => "1".into(),
            // An integer, so formatting it into the SQL cannot inject anything.
            View::List(id) => format!("t.list_id = {}", id.0),
        }
    }

    /// Smart views are day-based: they show what is done today, not everything done.
    fn is_smart(self) -> bool {
        !matches!(self, View::All | View::List(_))
    }

    /// A new task can never be overdue, so Overdue has no "new task" field.
    pub fn can_create(self) -> bool {
        self != View::Overdue
    }
}

/// Pending (not done) tasks per view. Lists without pending tasks are absent from `per_list`.
#[derive(Debug, Default, PartialEq)]
pub struct Counts {
    pub today: u32,
    pub scheduled: u32,
    pub inbox: u32,
    pub overdue: u32,
    pub all: u32,
    pub per_list: Vec<(ListId, u32)>,
}

impl Counts {
    pub fn for_list(&self, id: ListId) -> u32 {
        self.per_list
            .iter()
            .find(|(l, _)| *l == id)
            .map_or(0, |(_, n)| *n)
    }

    pub fn for_view(&self, view: View) -> u32 {
        match view {
            View::Today => self.today,
            View::Scheduled => self.scheduled,
            View::Inbox => self.inbox,
            View::Overdue => self.overdue,
            View::All => self.all,
            View::List(id) => self.for_list(id),
        }
    }
}

/// Index of the first done task in a `list` result, which puts every pending task first;
/// the length when nothing is done.
pub fn first_done(tasks: &[Task]) -> usize {
    tasks.partition_point(|t| !t.done)
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
    /// The view does not take new tasks (see `View::can_create`).
    CannotCreateHere,
    NotFound,
    BadDate(String),
    ListNotFound,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::EmptyTitle => write!(f, "the title is empty"),
            TaskError::CannotCreateHere => write!(f, "tasks cannot be created in this view"),
            TaskError::NotFound => write!(f, "task not found"),
            TaskError::BadDate(d) => write!(f, "invalid stored date: {d}"),
            TaskError::ListNotFound => write!(f, "list not found"),
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
        match e {
            // The only foreign key a task has is its list.
            rusqlite::Error::SqliteFailure(f, _)
                if f.extended_code == ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
            {
                TaskError::ListNotFound
            }
            e => TaskError::Sqlite(e),
        }
    }
}

pub fn create(
    conn: &Connection,
    title: &str,
    view: View,
    today: NaiveDate,
) -> Result<TaskId, TaskError> {
    if !view.can_create() {
        return Err(TaskError::CannotCreateHere);
    }
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    // The new task must show up in the view it was created from.
    let due = match view {
        View::Today => Some(today),
        View::Scheduled => Some(
            today
                .succ_opt()
                .ok_or_else(|| TaskError::BadDate(today.to_string()))?,
        ),
        _ => None,
    };
    let list = match view {
        View::List(id) => Some(id.0),
        _ => None,
    };
    conn.execute(
        "INSERT INTO task (title, due_date, list_id) VALUES (?1, ?2, ?3)",
        (title, due.map(|d| d.to_string()), list),
    )?;
    Ok(TaskId(conn.last_insert_rowid()))
}

/// Tasks matching the view's rule, pending first, then done (most recently completed first).
/// All and lists: every task, pending oldest created first.
/// Smart views (Today, Scheduled, Inbox, Overdue): pending by due date, then creation, and
/// only the tasks done today (local day of `now`).
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
    let (done, order) = if view.is_smart() {
        (
            "AND (t.done_at IS NULL OR t.done_at >= p.midnight)",
            "t.due_date, t.created_at, t.id",
        )
    } else {
        ("", "t.created_at, t.id")
    };
    let sql = format!(
        "WITH p(today, midnight) AS (SELECT ?1, ?2) {SELECT_TASKS} CROSS JOIN p \
         WHERE ({}) {done} \
         ORDER BY t.done_at IS NOT NULL, t.done_at DESC, {order}",
        view.rule()
    );
    query(conn, &sql, (today.to_string(), midnight))
}

/// Tasks due on each of the 7 days from `monday`, pending first, then oldest created first.
pub fn week(conn: &Connection, monday: NaiveDate) -> Result<[Vec<Task>; 7], TaskError> {
    let sunday =
        calendar::shift(monday, 6).ok_or_else(|| TaskError::BadDate(monday.to_string()))?;
    let sql = format!(
        "{SELECT_TASKS} WHERE t.due_date BETWEEN ?1 AND ?2 \
         ORDER BY t.due_date, t.done_at IS NOT NULL, t.created_at, t.id"
    );
    let mut days: [Vec<Task>; 7] = Default::default();
    for t in query(conn, &sql, (monday.to_string(), sunday.to_string()))? {
        // In range by the query, so the offset is 0..=6.
        let i = t.due.map_or(0, |d| (d - monday).num_days()) as usize;
        days[i].push(t);
    }
    Ok(days)
}

/// Kanban column of a task.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// Pending, not started.
    Icebox,
    InProgress,
    Done,
}

/// Moves a task to a Kanban column in one statement. Done keeps an earlier completion time
/// and the in-progress mark, so unchecking it later returns it to the column it came from.
pub fn set_stage(conn: &Connection, id: TaskId, stage: Stage) -> Result<(), TaskError> {
    let sql = match stage {
        Stage::Icebox => "UPDATE task SET in_progress = 0, done_at = NULL WHERE id = ?1",
        Stage::InProgress => "UPDATE task SET in_progress = 1, done_at = NULL WHERE id = ?1",
        Stage::Done => return set_done(conn, id, true),
    };
    match conn.execute(sql, [id.0])? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// How far back the Done column looks.
pub const KANBAN_DONE_DAYS: i64 = 7;

/// The Icebox, In progress and Done columns. Pending ones by due date (undated last), then
/// creation; Done holds what was finished in the last `KANBAN_DONE_DAYS`, newest first.
pub fn kanban(conn: &Connection, now: DateTime<Utc>) -> Result<[Vec<Task>; 3], TaskError> {
    const PENDING_ORDER: &str = "ORDER BY t.due_date IS NULL, t.due_date, t.created_at, t.id";
    let since = (now - chrono::TimeDelta::days(KANBAN_DONE_DAYS))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    Ok([
        query(
            conn,
            &format!(
                "{SELECT_TASKS} WHERE t.done_at IS NULL AND NOT t.in_progress {PENDING_ORDER}"
            ),
            (),
        )?,
        query(
            conn,
            &format!("{SELECT_TASKS} WHERE t.done_at IS NULL AND t.in_progress {PENDING_ORDER}"),
            (),
        )?,
        query(
            conn,
            &format!("{SELECT_TASKS} WHERE t.done_at >= ?1 ORDER BY t.done_at DESC, t.id"),
            [since],
        )?,
    ])
}

const SELECT_TASKS: &str = "SELECT t.id, t.title, t.note, t.done_at IS NOT NULL, t.due_date, \
     t.list_id, l.name, t.urgent, t.important FROM task t LEFT JOIN list l ON l.id = t.list_id";

/// Runs a `SELECT_TASKS` query and parses the stored due dates.
fn query(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Task>, TaskError> {
    let map = |r: &rusqlite::Row| {
        let list = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<String>>(6)?) {
            (Some(id), Some(name)) => Some((ListId(id), name)),
            _ => None,
        };
        Ok((
            Task {
                id: TaskId(r.get(0)?),
                title: r.get(1)?,
                note: r.get(2)?,
                done: r.get(3)?,
                due: None,
                list,
                priority: Priority {
                    urgent: r.get(7)?,
                    important: r.get(8)?,
                },
            },
            r.get::<_, Option<String>>(4)?,
        ))
    };
    conn.prepare(sql)?
        .query_map(params, map)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
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

/// `None` removes the task from its list. An unknown list is `ListNotFound`.
pub fn set_list(conn: &Connection, id: TaskId, list: Option<ListId>) -> Result<(), TaskError> {
    match conn.execute(
        "UPDATE task SET list_id = ?1 WHERE id = ?2",
        (list.map(|l| l.0), id.0),
    )? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// Saves the editor in one statement. The title is trimmed and must not be empty; the note
/// keeps inner newlines but loses trailing whitespace.
pub fn update(
    conn: &Connection,
    id: TaskId,
    title: &str,
    note: &str,
    priority: Priority,
) -> Result<(), TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    match conn.execute(
        "UPDATE task SET title = ?1, note = ?2, urgent = ?3, important = ?4 WHERE id = ?5",
        (
            title,
            note.trim_end(),
            priority.urgent,
            priority.important,
            id.0,
        ),
    )? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// Moves a task to an Eisenhower quadrant.
pub fn set_priority(conn: &Connection, id: TaskId, priority: Priority) -> Result<(), TaskError> {
    match conn.execute(
        "UPDATE task SET urgent = ?1, important = ?2 WHERE id = ?3",
        (priority.urgent, priority.important, id.0),
    )? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// Pending tasks per quadrant, in `Priority::QUADRANTS` order; each by due date (undated
/// last), then creation.
pub fn eisenhower(conn: &Connection) -> Result<[Vec<Task>; 4], TaskError> {
    let sql = format!(
        "{SELECT_TASKS} WHERE t.done_at IS NULL \
         ORDER BY t.due_date IS NULL, t.due_date, t.created_at, t.id"
    );
    let mut quadrants: [Vec<Task>; 4] = Default::default();
    for t in query(conn, &sql, ())? {
        quadrants[t.priority.quadrant()].push(t);
    }
    Ok(quadrants)
}

pub fn delete(conn: &Connection, id: TaskId) -> Result<(), TaskError> {
    match conn.execute("DELETE FROM task WHERE id = ?1", [id.0])? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// Pending counts for the lists panel, with the same rules as `list`.
pub fn counts(conn: &Connection, now: DateTime<FixedOffset>) -> Result<Counts, TaskError> {
    let views = [
        View::Today,
        View::Scheduled,
        View::Inbox,
        View::Overdue,
        View::All,
    ];
    let columns: Vec<_> = views
        .iter()
        .map(|v| format!("count(CASE WHEN {} THEN 1 END)", v.rule()))
        .collect();
    let sql = format!(
        "WITH p(today) AS (SELECT ?1) SELECT {} FROM task t CROSS JOIN p WHERE t.done_at IS NULL",
        columns.join(", ")
    );
    let n = conn.query_row(&sql, [now.date_naive().to_string()], |r| {
        Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?])
    })?;
    let [today, scheduled, inbox, overdue, all] = n;
    let per_list = conn
        .prepare(
            "SELECT list_id, count(*) FROM task \
             WHERE done_at IS NULL AND list_id IS NOT NULL GROUP BY list_id",
        )?
        .query_map([], |r| Ok((ListId(r.get(0)?), r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(Counts {
        today,
        scheduled,
        inbox,
        overdue,
        all,
        per_list,
    })
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
    use crate::{db, lists};

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
        view_titles(conn, View::Today, now)
    }

    fn view_titles(conn: &Connection, view: View, now: &str) -> Vec<String> {
        list(conn, view, at(now))
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect()
    }

    fn task(due: Option<&str>, done: bool) -> Task {
        Task {
            id: TaskId(1),
            title: String::new(),
            note: String::new(),
            done,
            due: due.map(d),
            list: None,
            priority: Priority::default(),
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
    fn update_changes_title_and_note_and_trims() {
        let (dir, conn) = setup("update");
        let id = mk(&conn, "a").unwrap();
        update(
            &conn,
            id,
            "  new title 
",
            "  line 1

line 3 
	 ",
            Priority {
                urgent: true,
                important: false,
            },
        )
        .unwrap();
        let t = &ls(&conn)[0];
        assert_eq!(t.title, "new title");
        assert_eq!(
            t.priority,
            Priority {
                urgent: true,
                important: false
            }
        );
        assert_eq!(
            t.note,
            "  line 1

line 3"
        );
        update(&conn, id, "new title", "", Priority::default()).unwrap();
        assert_eq!(ls(&conn)[0].note, "");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_with_empty_title_changes_nothing() {
        let (dir, conn) = setup("update-empty");
        let id = mk(&conn, "a").unwrap();
        update(&conn, id, "a", "keep", Priority::default()).unwrap();
        assert!(matches!(
            update(
                &conn,
                id,
                " 
",
                "other",
                Priority::default()
            ),
            Err(TaskError::EmptyTitle)
        ));
        let t = &ls(&conn)[0];
        assert_eq!((t.title.as_str(), t.note.as_str()), ("a", "keep"));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_and_delete_unknown_id_are_not_found() {
        let (dir, conn) = setup("edit-missing");
        assert!(matches!(
            update(&conn, TaskId(42), "a", "", Priority::default()),
            Err(TaskError::NotFound)
        ));
        assert!(matches!(
            delete(&conn, TaskId(42)),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_task_and_its_count() {
        let (dir, conn) = setup("delete");
        let a = mk(&conn, "a").unwrap();
        mk(&conn, "b").unwrap();
        assert_eq!(counts(&conn, at(NOW)).unwrap().all, 2);
        delete(&conn, a).unwrap();
        assert_eq!(titles(&conn), ["b"]);
        assert_eq!(counts(&conn, at(NOW)).unwrap().all, 1);
        assert!(matches!(delete(&conn, a), Err(TaskError::NotFound)));
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
    fn create_sets_due_by_view_and_shows_up_there() {
        let (dir, conn) = setup("create-due");
        let today = d("2026-06-15");
        for (title, view) in [
            ("t", View::Today),
            ("s", View::Scheduled),
            ("i", View::Inbox),
            ("a", View::All),
        ] {
            create(&conn, title, view, today).unwrap();
            assert!(view_titles(&conn, view, NOW).contains(&title.to_string()));
        }
        let dues: Vec<_> = ls(&conn).into_iter().map(|t| t.due).collect();
        assert_eq!(dues, [Some(today), Some(d("2026-06-16")), None, None]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn week_puts_each_task_on_its_due_day() {
        let (dir, conn) = setup("week");
        for (title, due, done) in [
            ("mon", Some("2026-11-02"), false),
            ("sun", Some("2026-11-08"), false),
            ("thu-done", Some("2026-11-05"), true),
            ("thu", Some("2026-11-05"), false),
            ("before", Some("2026-11-01"), false),
            ("after", Some("2026-11-09"), false),
            ("none", None, false),
        ] {
            let id = mk(&conn, title).unwrap();
            set_due_date(&conn, id, due.map(d)).unwrap();
            set_done(&conn, id, done).unwrap();
        }
        let w = week(&conn, d("2026-11-02")).unwrap();
        let titles: Vec<Vec<&str>> = w
            .iter()
            .map(|day| day.iter().map(|t| t.title.as_str()).collect())
            .collect();
        let empty: Vec<&str> = vec![];
        assert_eq!(
            titles,
            [
                vec!["mon"],
                empty.clone(),
                empty.clone(),
                // Pending before done.
                vec!["thu", "thu-done"],
                empty.clone(),
                empty.clone(),
                vec!["sun"],
            ]
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn kanban_titles(conn: &Connection) -> Vec<Vec<String>> {
        let now = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        kanban(conn, now)
            .unwrap()
            .into_iter()
            .map(|c| c.into_iter().map(|t| t.title).collect())
            .collect()
    }

    #[test]
    fn kanban_moves_tasks_between_columns() {
        let (dir, conn) = setup("kanban");
        let a = mk(&conn, "a").unwrap();
        let b = mk(&conn, "b").unwrap();
        let c = mk(&conn, "c").unwrap();
        set_due_date(&conn, b, Some(d("2026-06-20"))).unwrap();
        // Dated before undated in the icebox.
        assert_eq!(kanban_titles(&conn), [vec!["b", "a", "c"], vec![], vec![]]);
        set_stage(&conn, a, Stage::InProgress).unwrap();
        // Finished within the last 7 days.
        conn.execute(
            "UPDATE task SET done_at = '2026-06-10T00:00:00.000Z' WHERE id = ?1",
            [c.0],
        )
        .unwrap();
        assert_eq!(kanban_titles(&conn), [vec!["b"], vec!["a"], vec!["c"]]);
        // Done then unchecked: back to the column it came from.
        set_stage(&conn, a, Stage::Done).unwrap();
        set_done(&conn, a, false).unwrap();
        assert_eq!(kanban_titles(&conn)[1], ["a"]);
        set_stage(&conn, a, Stage::Icebox).unwrap();
        assert_eq!(kanban_titles(&conn)[0], ["b", "a"]);
        assert!(matches!(
            set_stage(&conn, TaskId(42), Stage::InProgress),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kanban_done_column_keeps_only_the_last_7_days() {
        let (dir, conn) = setup("kanban-old");
        for (title, done_at) in [
            ("old", "2026-06-08T11:59:59.999Z"),
            ("edge", "2026-06-08T12:00:00.000Z"),
        ] {
            let id = mk(&conn, title).unwrap();
            conn.execute(
                "UPDATE task SET done_at = ?1 WHERE id = ?2",
                (done_at, id.0),
            )
            .unwrap();
        }
        assert_eq!(kanban_titles(&conn)[2], ["edge"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quadrant_indexes_match_the_quadrants() {
        for (i, q) in Priority::QUADRANTS.iter().enumerate() {
            assert_eq!(q.quadrant(), i, "{q:?}");
        }
    }

    #[test]
    fn eisenhower_sorts_pending_tasks_into_quadrants() {
        let (dir, conn) = setup("eisenhower");
        let p = |urgent, important| Priority { urgent, important };
        for (title, priority, done) in [
            ("do", p(true, true), false),
            ("plan", p(false, true), false),
            ("delegate", p(true, false), false),
            ("drop", p(false, false), false),
            ("done", p(true, true), true),
        ] {
            let id = mk(&conn, title).unwrap();
            set_priority(&conn, id, priority).unwrap();
            set_done(&conn, id, done).unwrap();
        }
        let titles = |conn: &Connection| -> Vec<Vec<String>> {
            eisenhower(conn)
                .unwrap()
                .into_iter()
                .map(|q| q.into_iter().map(|t| t.title).collect())
                .collect()
        };
        // Done tasks stay out.
        assert_eq!(
            titles(&conn),
            [vec!["do"], vec!["plan"], vec!["delegate"], vec!["drop"]]
        );
        let drop_id = ls(&conn).iter().find(|t| t.title == "drop").unwrap().id;
        set_priority(&conn, drop_id, p(true, true)).unwrap();
        assert_eq!(titles(&conn)[0], ["do", "drop"]);
        assert!(matches!(
            set_priority(&conn, TaskId(42), p(true, true)),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overdue_takes_no_new_tasks() {
        let (dir, conn) = setup("create-overdue");
        let r = create(&conn, "a", View::Overdue, d("2026-06-15"));
        assert!(matches!(r, Err(TaskError::CannotCreateHere)));
        assert!(ls(&conn).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn smart_views_split_tasks_by_due_date_and_list() {
        let (dir, conn) = setup("smart");
        let work = lists::create(&conn, "Work").unwrap();
        for (title, due, list) in [
            ("today", Some("2026-06-15"), None),
            ("future", Some("2026-06-16"), None),
            ("none", None, None),
            ("listed", None, Some(work)),
            ("late", Some("2026-06-10"), None),
            ("later", Some("2026-06-12"), Some(work)),
        ] {
            let id = mk(&conn, title).unwrap();
            set_due_date(&conn, id, due.map(d)).unwrap();
            set_list(&conn, id, list).unwrap();
        }
        let v = |view| view_titles(&conn, view, NOW);
        assert_eq!(v(View::Today), ["today"]);
        assert_eq!(v(View::Scheduled), ["future"]);
        // A task with a list is not in the Inbox, even without a date.
        assert_eq!(v(View::Inbox), ["none"]);
        assert_eq!(v(View::Overdue), ["late", "later"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_match_the_pending_tasks_each_view_lists() {
        let (dir, conn) = setup("counts-match");
        let work = lists::create(&conn, "Work").unwrap();
        for (due, list, done) in [
            (Some("2026-06-15"), None, false),
            (Some("2026-06-15"), None, true),
            (Some("2026-06-20"), Some(work), false),
            (None, None, false),
            (None, None, true),
            (None, Some(work), false),
            (Some("2026-06-01"), None, false),
            (Some("2026-06-01"), Some(work), true),
        ] {
            let id = mk(&conn, "t").unwrap();
            set_due_date(&conn, id, due.map(d)).unwrap();
            set_list(&conn, id, list).unwrap();
            set_done(&conn, id, done).unwrap();
        }
        let c = counts(&conn, at(NOW)).unwrap();
        for view in [
            View::Today,
            View::Scheduled,
            View::Inbox,
            View::Overdue,
            View::All,
            View::List(work),
        ] {
            let tasks = list(&conn, view, at(NOW)).unwrap();
            let pending = tasks.iter().filter(|t| !t.done).count();
            assert_eq!(pending, c.for_view(view) as usize, "{view:?}");
            // Pending first, so the done ones start right after them.
            assert_eq!(first_done(&tasks), pending, "{view:?}");
            assert!(tasks[first_done(&tasks)..].iter().all(|t| t.done));
        }
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
        assert_eq!(today_titles(&conn, NOW), ["pending", "done-today"]);
        // Overdue keeps what was finished today, so checking a task does not hide it.
        assert_eq!(view_titles(&conn, View::Overdue, NOW), ["done-later"]);
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

    fn list_of(conn: &Connection, id: TaskId) -> Option<(ListId, String)> {
        ls(conn).into_iter().find(|t| t.id == id).unwrap().list
    }

    #[test]
    fn create_in_list_view_assigns_the_list_without_due() {
        let (dir, conn) = setup("create-list");
        let work = lists::create(&conn, "Work").unwrap();
        let id = create(&conn, "a", View::List(work), d("2026-06-15")).unwrap();
        let t = ls(&conn).remove(0);
        assert_eq!(t.id, id);
        assert_eq!(t.due, None);
        assert_eq!(t.list, Some((work, "Work".to_string())));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_in_unknown_list_fails_cleanly() {
        let (dir, conn) = setup("create-bad-list");
        let r = create(&conn, "a", View::List(ListId(42)), d("2026-06-15"));
        assert!(matches!(r, Err(TaskError::ListNotFound)));
        assert!(ls(&conn).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_view_shows_only_that_list_in_all_order() {
        let (dir, conn) = setup("list-view");
        let work = lists::create(&conn, "Work").unwrap();
        let home = lists::create(&conn, "Home").unwrap();
        for (title, list) in [
            ("w1", Some(work)),
            ("h1", Some(home)),
            ("n1", None),
            ("w2", Some(work)),
        ] {
            let id = mk(&conn, title).unwrap();
            set_list(&conn, id, list).unwrap();
        }
        conn.execute(
            "UPDATE task SET done_at = '2026-01-01T00:00:00Z' WHERE title = 'w1'",
            [],
        )
        .unwrap();
        let v = list(&conn, View::List(work), at(NOW)).unwrap();
        let titles: Vec<_> = v.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["w2", "w1"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_list_moves_clears_and_reports_errors() {
        let (dir, conn) = setup("set-list");
        let work = lists::create(&conn, "Work").unwrap();
        let id = mk(&conn, "a").unwrap();
        assert_eq!(list_of(&conn, id), None);
        set_list(&conn, id, Some(work)).unwrap();
        assert_eq!(list_of(&conn, id), Some((work, "Work".to_string())));
        set_list(&conn, id, None).unwrap();
        assert_eq!(list_of(&conn, id), None);
        assert!(matches!(
            set_list(&conn, TaskId(42), Some(work)),
            Err(TaskError::NotFound)
        ));
        assert!(matches!(
            set_list(&conn, id, Some(ListId(42))),
            Err(TaskError::ListNotFound)
        ));
        assert_eq!(list_of(&conn, id), None);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_pending_per_view_and_list() {
        let (dir, conn) = setup("counts");
        let work = lists::create(&conn, "Work").unwrap();
        let empty = lists::create(&conn, "Empty").unwrap();
        // (list, due, done)
        for (list, due, done) in [
            (Some(work), Some("2026-06-15"), false),
            (Some(work), Some("2026-06-10"), false),
            (Some(work), Some("2026-06-15"), true),
            (None, Some("2026-06-16"), false),
            (None, None, false),
            (None, Some("2026-06-15"), false),
        ] {
            let id = mk(&conn, "t").unwrap();
            set_list(&conn, id, list).unwrap();
            set_due_date(&conn, id, due.map(d)).unwrap();
            if done {
                set_done(&conn, id, true).unwrap();
            }
        }
        let c = counts(&conn, at(NOW)).unwrap();
        assert_eq!(
            (c.today, c.scheduled, c.inbox, c.overdue, c.all),
            (2, 1, 1, 1, 5)
        );
        assert_eq!(c.for_list(work), 2);
        assert_eq!(c.for_list(empty), 0);
        assert_eq!(c.per_list, [(work, 2)]);
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
