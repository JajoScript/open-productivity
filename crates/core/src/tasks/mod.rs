// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tasks: create, list, mark as done, due dates, lists, edit and delete.

use std::fmt;

use chrono::{DateTime, Days, FixedOffset, NaiveDate, NaiveTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, ffi};

use crate::calendar;
use crate::lists::ListId;
use crate::recurrence::{self, Frequency};

mod queries;
mod repeat;
#[cfg(test)]
mod tests;

pub use queries::*;
pub(crate) use queries::{SELECT_TASKS, local_midnight, query};
use repeat::complete;
pub use repeat::{parse_interval, set_recurrence};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskId(pub i64);

#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub note: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    /// Local start and end times of the due day; only with a due date, the end only with a
    /// start and strictly after it.
    pub start: Option<NaiveTime>,
    pub end: Option<NaiveTime>,
    /// The list id and its name.
    pub list: Option<(ListId, String)>,
    pub priority: Priority,
    /// Repeats when completed: how often and every how many periods. Only with a due date.
    pub repeat: Option<(Frequency, u32)>,
}

impl Task {
    /// `09:00` or `09:00–`+`10:30`; empty without a start time.
    pub fn time_label(&self) -> String {
        match (self.start, self.end) {
            (Some(s), Some(e)) => format!("{}\u{2013}{}", s.format("%H:%M"), e.format("%H:%M")),
            (Some(s), None) => s.format("%H:%M").to_string(),
            _ => String::new(),
        }
    }

    /// The title with its start time in front, as the calendar shows it.
    pub fn timed_title(&self) -> String {
        match self.start {
            Some(s) => format!("{} {}", s.format("%H:%M"), self.title),
            None => self.title.clone(),
        }
    }
}

/// A time as the editor shows it (`09:00`); empty for none.
pub fn format_time(t: Option<NaiveTime>) -> String {
    t.map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// Reads the user's text for a time: `H:MM` or `HH:MM`; blank means no time.
pub fn parse_time(text: &str) -> Result<Option<NaiveTime>, TaskError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let bad = || TaskError::BadTime(text.to_string());
    let (h, m) = text.split_once(':').ok_or_else(bad)?;
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !(1..=2).contains(&h.len()) || m.len() != 2 || !digits(h) || !digits(m) {
        return Err(bad());
    }
    let (h, m) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    NaiveTime::from_hms_opt(h, m, 0).map(Some).ok_or_else(bad)
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
    /// Without a list, a due date or a subject (topics are not inbox).
    Inbox,
    /// Due before today.
    Overdue,
    All,
    List(ListId),
}

impl View {
    /// The only place each view's rule is written: an SQL condition on the task `t` and
    /// on `p.today`, shared by `list` and `counts` so they never disagree.
    pub(crate) fn rule(self) -> String {
        match self {
            View::Today => "t.due_date = p.today".into(),
            View::Scheduled => "t.due_date > p.today".into(),
            View::Inbox => {
                "t.list_id IS NULL AND t.due_date IS NULL AND t.subject_id IS NULL".into()
            }
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
    /// Text (or a stored value) that is not a valid `HH:MM` time.
    BadTime(String),
    /// A start time needs a due date.
    TimeWithoutDate,
    /// An end time needs a start time.
    EndWithoutStart,
    /// The end must be later than the start, on the same day.
    EndNotAfterStart,
    ListNotFound,
    /// A repeat rule needs a due date to count from.
    RecurrenceWithoutDate,
    /// Text that is not a whole number from 1 to 999.
    BadInterval(String),
    /// A stored frequency that is not one of ours.
    BadFrequency(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::EmptyTitle => write!(f, "the title is empty"),
            TaskError::CannotCreateHere => write!(f, "tasks cannot be created in this view"),
            TaskError::NotFound => write!(f, "task not found"),
            TaskError::BadDate(d) => write!(f, "invalid stored date: {d}"),
            TaskError::BadTime(t) => write!(f, "invalid time: {t}"),
            TaskError::TimeWithoutDate => write!(f, "a time needs a due date"),
            TaskError::EndWithoutStart => write!(f, "an end time needs a start time"),
            TaskError::EndNotAfterStart => write!(f, "the end must be after the start"),
            TaskError::ListNotFound => write!(f, "list not found"),
            TaskError::RecurrenceWithoutDate => write!(f, "a repeating task needs a due date"),
            TaskError::BadInterval(t) => write!(f, "invalid repeat interval: {t}"),
            TaskError::BadFrequency(t) => write!(f, "invalid stored frequency: {t}"),
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
    insert(conn, title, due, list)
}

/// A task with the given due date and no list, in a single INSERT; any date is valid, past too.
pub fn create_on(conn: &Connection, title: &str, due: NaiveDate) -> Result<TaskId, TaskError> {
    insert(conn, title, Some(due), None)
}

fn insert(
    conn: &Connection,
    title: &str,
    due: Option<NaiveDate>,
    list: Option<i64>,
) -> Result<TaskId, TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    conn.execute(
        "INSERT INTO task (title, due_date, list_id) VALUES (?1, ?2, ?3)",
        (title, due.map(|d| d.to_string()), list),
    )?;
    Ok(TaskId(conn.last_insert_rowid()))
}

/// Kanban column of a task.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// Pending, not started.
    Icebox,
    InProgress,
    Done,
}

/// Moves a task to a Kanban column. Done is `set_done` (so a repeating task makes its next one);
/// it keeps an earlier completion time and the in-progress mark, so unchecking it later returns
/// it to the column it came from.
pub fn set_stage(
    conn: &Connection,
    id: TaskId,
    stage: Stage,
    today: NaiveDate,
) -> Result<(), TaskError> {
    let sql = match stage {
        Stage::Icebox => "UPDATE task SET in_progress = 0, done_at = NULL WHERE id = ?1",
        Stage::InProgress => "UPDATE task SET in_progress = 1, done_at = NULL WHERE id = ?1",
        Stage::Done => return set_done(conn, id, true, today),
    };
    match conn.execute(sql, [id.0])? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// How far back the Done column looks.
pub const KANBAN_DONE_DAYS: i64 = 7;

/// Marking an already done task keeps its original completion time. Completing a task that
/// repeats creates the next one in the same transaction (see `complete`); unchecking only
/// reopens the task. `today` is the user's local day.
pub fn set_done(
    conn: &Connection,
    id: TaskId,
    done: bool,
    today: NaiveDate,
) -> Result<(), TaskError> {
    let tx = conn.unchecked_transaction()?;
    let sql = if done {
        "UPDATE task SET done_at = coalesce(done_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
         WHERE id = ?1"
    } else {
        "UPDATE task SET done_at = NULL WHERE id = ?1"
    };
    if tx.execute(sql, [id.0])? == 0 {
        return Err(TaskError::NotFound);
    }
    if done {
        complete(&tx, id, today)?;
    }
    Ok(tx.commit()?)
}

/// Without a date the times and the repeat rule go too; a new date is the rule's new anchor day.
pub fn set_due_date(
    conn: &Connection,
    id: TaskId,
    due: Option<NaiveDate>,
) -> Result<(), TaskError> {
    let tx = conn.unchecked_transaction()?;
    let due = due.map(|d| d.to_string());
    if tx.execute(
        "UPDATE task SET due_date = ?1, \
         due_time = CASE WHEN ?1 IS NULL THEN NULL ELSE due_time END, \
         end_time = CASE WHEN ?1 IS NULL THEN NULL ELSE end_time END WHERE id = ?2",
        (&due, id.0),
    )? == 0
    {
        return Err(TaskError::NotFound);
    }
    move_anchor(&tx, id, due.as_deref())?;
    if due.is_none() {
        set_recurrence(&tx, id, None)?;
    }
    Ok(tx.commit()?)
}

/// A new due date is the rule's new anchor day.
pub(crate) fn move_anchor(
    conn: &Connection,
    id: TaskId,
    due: Option<&str>,
) -> Result<(), TaskError> {
    conn.execute(
        "UPDATE task_recurrence SET anchor_day = CAST(strftime('%d', ?1) AS INTEGER) \
         WHERE task_id = ?2 AND ?1 IS NOT NULL",
        (due, id.0),
    )?;
    Ok(())
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

/// The times as the stored `HH:MM` text, after the rules that need no database.
fn check_times(
    start: Option<NaiveTime>,
    end: Option<NaiveTime>,
) -> Result<[Option<String>; 2], TaskError> {
    match (start, end) {
        (None, Some(_)) => Err(TaskError::EndWithoutStart),
        (Some(s), Some(e)) if e <= s => Err(TaskError::EndNotAfterStart),
        _ => Ok([start, end].map(|t| t.map(|t| t.format("%H:%M").to_string()))),
    }
}

/// Zero rows changed: the task is missing, or the guard "a start time needs a due date" failed.
fn no_row(conn: &Connection, id: TaskId) -> TaskError {
    match conn.query_row("SELECT 1 FROM task WHERE id = ?1", [id.0], |_| Ok(())) {
        Ok(()) => TaskError::TimeWithoutDate,
        Err(rusqlite::Error::QueryReturnedNoRows) => TaskError::NotFound,
        Err(e) => e.into(),
    }
}

/// Sets or clears the start and end times in one statement. See `Task::start`.
pub fn set_time(
    conn: &Connection,
    id: TaskId,
    start: Option<NaiveTime>,
    end: Option<NaiveTime>,
) -> Result<(), TaskError> {
    let [s, e] = check_times(start, end)?;
    match conn.execute(
        "UPDATE task SET due_time = ?1, end_time = ?2 \
         WHERE id = ?3 AND (?1 IS NULL OR due_date IS NOT NULL)",
        (s, e, id.0),
    )? {
        0 => Err(no_row(conn, id)),
        _ => Ok(()),
    }
}

/// Saves the editor in one statement, so nothing is half saved. The title is trimmed and must
/// not be empty; the note keeps inner newlines but loses trailing whitespace; `start` and `end`
/// are the user's text (see `parse_time`); `repeat` is the rule with the user's "every" text (see
/// `set_recurrence`). The task and its rule are saved in one transaction.
#[allow(clippy::too_many_arguments)]
pub fn update(
    conn: &Connection,
    id: TaskId,
    title: &str,
    note: &str,
    priority: Priority,
    start: &str,
    end: &str,
    repeat: Option<(Frequency, &str)>,
) -> Result<(), TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    let [s, e] = check_times(parse_time(start)?, parse_time(end)?)?;
    let tx = conn.unchecked_transaction()?;
    match tx.execute(
        "UPDATE task SET title = ?1, note = ?2, urgent = ?3, important = ?4, \
         due_time = ?5, end_time = ?6 \
         WHERE id = ?7 AND (?5 IS NULL OR due_date IS NOT NULL)",
        (
            title,
            note.trim_end(),
            priority.urgent,
            priority.important,
            s,
            e,
            id.0,
        ),
    )? {
        0 => return Err(no_row(&tx, id)),
        _ => set_recurrence(&tx, id, repeat)?,
    }
    Ok(tx.commit()?)
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

pub fn delete(conn: &Connection, id: TaskId) -> Result<(), TaskError> {
    match conn.execute("DELETE FROM task WHERE id = ?1", [id.0])? {
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
