// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Goals and projects: create, view, edit and delete.

use std::fmt;

use chrono::NaiveDate;
use rusqlite::{Connection, params};

mod entries;
mod tasks;
#[cfg(test)]
mod tests;

pub use entries::{Entry, EntryId, add_entry, delete_entry, list_entries};
pub use tasks::{GoalTask, create_task, list_tasks};

const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoalId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GoalKind {
    Goal,
    Project,
}

impl GoalKind {
    fn as_str(self) -> &'static str {
        match self {
            GoalKind::Goal => "goal",
            GoalKind::Project => "project",
        }
    }
}

/// The numeric measure of a goal: reach `target` `unit`s by adding up entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Numeric {
    pub target: i64,
    pub unit: String,
}

/// Progress toward a numeric goal's target.
#[derive(Clone, Debug, PartialEq)]
pub struct NumericProgress {
    pub target: i64,
    pub unit: String,
    /// Sum of the entries; it may pass `target`.
    pub total: i64,
    /// `total` over `target` as a whole 0..=100, truncated and capped.
    pub percent: u32,
}

/// What the user types when creating or editing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoalDraft {
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub deadline: Option<NaiveDate>,
    /// Only goals (not projects) take a measure, and it never changes after creating.
    pub numeric: Option<Numeric>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: GoalId,
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub kind: GoalKind,
    pub deadline: Option<NaiveDate>,
    /// Days from `today` to the deadline (negative once passed); `None` without a deadline.
    pub days_left: Option<i64>,
    pub tasks_done: u32,
    pub tasks_total: u32,
    /// `tasks_done` over `tasks_total` as a whole 0..=100; 0 without tasks.
    pub tasks_percent: u32,
    pub numeric: Option<NumericProgress>,
}

/// Whole percent of `done` over `total`, truncated; 0 when there is nothing to do.
pub fn percent(done: u32, total: u32) -> u32 {
    (done * 100).checked_div(total).unwrap_or(0)
}

/// A whole number typed in a field: digits, optionally grouped by `.` in thousands, from 1 to
/// 1000 million (so sums of a few entries still fit the UI's i32).
pub fn parse_count(text: &str) -> Result<i64, GoalError> {
    let text = text.trim();
    let mut groups = text.split('.');
    let first = groups.next().unwrap_or_default();
    let grouped = !text.contains('.') || (first.len() <= 3 && groups.all(|g| g.len() == 3));
    let digits = text.replace('.', "");
    if !grouped || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(GoalError::BadNumber);
    }
    match digits.parse::<i64>() {
        Ok(n) if (1..=1_000_000_000).contains(&n) => Ok(n),
        _ => Err(GoalError::BadNumber),
    }
}

#[derive(Debug)]
pub enum GoalError {
    EmptyTitle,
    NotFound,
    /// A project takes no numeric measure.
    ProjectMeasure,
    /// The target is not above 0.
    BadTarget,
    /// Editing tried to add or remove the measure.
    MeasureChanged,
    /// An entry value is not above 0.
    BadValue,
    /// An entry for a goal that does not exist or has no measure.
    NotNumeric,
    /// Text that is not a positive whole number.
    BadNumber,
    /// A stored kind or date that cannot be read.
    BadDate(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for GoalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoalError::EmptyTitle => write!(f, "the title is empty"),
            GoalError::NotFound => write!(f, "goal not found"),
            GoalError::ProjectMeasure => write!(f, "a project takes no measure"),
            GoalError::BadTarget => write!(f, "the target must be above 0"),
            GoalError::MeasureChanged => write!(f, "the measure cannot change"),
            GoalError::BadValue => write!(f, "the value must be above 0"),
            GoalError::NotNumeric => write!(f, "the goal has no numeric measure"),
            GoalError::BadNumber => write!(f, "not a positive whole number"),
            GoalError::BadDate(d) => write!(f, "invalid stored goal value: {d}"),
            GoalError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for GoalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GoalError::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for GoalError {
    fn from(e: rusqlite::Error) -> Self {
        GoalError::Sqlite(e)
    }
}

/// Days from `today` to `deadline` (negative once it has passed).
pub fn days_left(deadline: NaiveDate, today: NaiveDate) -> i64 {
    (deadline - today).num_days()
}

fn title(draft: &GoalDraft) -> Result<&str, GoalError> {
    match draft.title.trim() {
        "" => Err(GoalError::EmptyTitle),
        t => Ok(t),
    }
}

fn deadline_text(draft: &GoalDraft) -> Option<String> {
    draft.deadline.map(|d| d.format(DATE_FORMAT).to_string())
}

/// The target and trimmed unit to store; the target must be above 0.
fn measure(draft: &GoalDraft) -> Result<(Option<i64>, Option<&str>), GoalError> {
    match &draft.numeric {
        None => Ok((None, None)),
        Some(n) if n.target <= 0 => Err(GoalError::BadTarget),
        Some(n) => Ok((Some(n.target), Some(n.unit.trim()))),
    }
}

pub fn create(conn: &Connection, kind: GoalKind, draft: &GoalDraft) -> Result<GoalId, GoalError> {
    let title = title(draft)?;
    if draft.numeric.is_some() && kind == GoalKind::Project {
        return Err(GoalError::ProjectMeasure);
    }
    let (target, unit) = measure(draft)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO goal (title, emoji, description, kind, deadline, target_count, unit)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            title,
            draft.emoji.trim(),
            draft.description.trim(),
            kind.as_str(),
            deadline_text(draft),
            target,
            unit
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(GoalId(id))
}

/// Edits the text and the deadline, and the target and unit of a numeric goal. The kind and the
/// measure never change, and `None` clears the deadline.
pub fn update(conn: &Connection, id: GoalId, draft: &GoalDraft) -> Result<(), GoalError> {
    let title = title(draft)?;
    let (target, unit) = measure(draft)?;
    let tx = conn.unchecked_transaction()?;
    let numeric: bool = match tx.query_row(
        "SELECT target_count IS NOT NULL FROM goal WHERE id = ?1",
        [id.0],
        |r| r.get(0),
    ) {
        Ok(n) => n,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(GoalError::NotFound),
        Err(e) => return Err(e.into()),
    };
    if numeric != draft.numeric.is_some() {
        return Err(GoalError::MeasureChanged);
    }
    tx.execute(
        "UPDATE goal SET title = ?1, emoji = ?2, description = ?3, deadline = ?4,
         target_count = ?5, unit = ?6 WHERE id = ?7",
        params![
            title,
            draft.emoji.trim(),
            draft.description.trim(),
            deadline_text(draft),
            target,
            unit,
            id.0
        ],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: GoalId) -> Result<(), GoalError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE task SET goal_id = NULL WHERE goal_id = ?1", [id.0])?;
    tx.execute("DELETE FROM goal_entry WHERE goal_id = ?1", [id.0])?;
    if tx.execute("DELETE FROM goal WHERE id = ?1", [id.0])? == 0 {
        return Err(GoalError::NotFound);
    }
    tx.commit()?;
    Ok(())
}

const SELECT: &str = "SELECT id, title, emoji, description, kind, deadline,
    (SELECT count(*) FROM task WHERE goal_id = goal.id AND done_at IS NOT NULL),
    (SELECT count(*) FROM task WHERE goal_id = goal.id),
    target_count, coalesce(unit, ''),
    (SELECT coalesce(sum(value), 0) FROM goal_entry WHERE goal_id = goal.id)
    FROM goal";

fn row(r: &rusqlite::Row, today: NaiveDate) -> rusqlite::Result<Result<Goal, GoalError>> {
    let kind: String = r.get(4)?;
    let deadline: Option<String> = r.get(5)?;
    let (id, title, emoji, description) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (tasks_done, tasks_total): (u32, u32) = (r.get(6)?, r.get(7)?);
    let (target, unit, total): (Option<i64>, String, i64) = (r.get(8)?, r.get(9)?, r.get(10)?);
    let kind = match kind.as_str() {
        "goal" => GoalKind::Goal,
        "project" => GoalKind::Project,
        _ => return Ok(Err(GoalError::BadDate(kind))),
    };
    let deadline = match deadline {
        None => None,
        Some(d) => match NaiveDate::parse_from_str(&d, DATE_FORMAT) {
            Ok(d) => Some(d),
            Err(_) => return Ok(Err(GoalError::BadDate(d))),
        },
    };
    Ok(Ok(Goal {
        id: GoalId(id),
        title,
        emoji,
        description,
        kind,
        deadline,
        days_left: deadline.map(|d| days_left(d, today)),
        tasks_done,
        tasks_total,
        tasks_percent: percent(tasks_done, tasks_total),
        numeric: target.map(|target| NumericProgress {
            target,
            unit,
            total,
            percent: (total.saturating_mul(100) / target).min(100) as u32,
        }),
    }))
}

pub fn get(conn: &Connection, id: GoalId, today: NaiveDate) -> Result<Goal, GoalError> {
    match conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id.0], |r| {
        row(r, today)
    }) {
        Ok(g) => g,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(GoalError::NotFound),
        Err(e) => Err(e.into()),
    }
}

/// Nearest deadline first, undated last, then creation order.
pub fn list(conn: &Connection, today: NaiveDate) -> Result<Vec<Goal>, GoalError> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY deadline IS NULL, deadline, created_at, id"
    ))?;
    let rows = stmt.query_map([], |r| row(r, today))?;
    rows.map(|r| r?).collect()
}
