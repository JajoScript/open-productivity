// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Goals and projects: create, view, edit and delete.

use std::fmt;

use chrono::NaiveDate;
use rusqlite::{Connection, params};

mod tasks;
#[cfg(test)]
mod tests;

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

/// What the user types when creating or editing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoalDraft {
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub deadline: Option<NaiveDate>,
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
}

/// Whole percent of `done` over `total`, truncated; 0 when there is nothing to do.
pub fn percent(done: u32, total: u32) -> u32 {
    (done * 100).checked_div(total).unwrap_or(0)
}

#[derive(Debug)]
pub enum GoalError {
    EmptyTitle,
    NotFound,
    /// A stored kind or date that cannot be read.
    BadDate(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for GoalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoalError::EmptyTitle => write!(f, "the title is empty"),
            GoalError::NotFound => write!(f, "goal not found"),
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

pub fn create(conn: &Connection, kind: GoalKind, draft: &GoalDraft) -> Result<GoalId, GoalError> {
    let title = title(draft)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO goal (title, emoji, description, kind, deadline) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            title,
            draft.emoji.trim(),
            draft.description.trim(),
            kind.as_str(),
            deadline_text(draft)
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(GoalId(id))
}

/// Edits the text and the deadline; the kind never changes and `None` clears the deadline.
pub fn update(conn: &Connection, id: GoalId, draft: &GoalDraft) -> Result<(), GoalError> {
    let title = title(draft)?;
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        "UPDATE goal SET title = ?1, emoji = ?2, description = ?3, deadline = ?4 WHERE id = ?5",
        params![
            title,
            draft.emoji.trim(),
            draft.description.trim(),
            deadline_text(draft),
            id.0
        ],
    )?;
    if n == 0 {
        return Err(GoalError::NotFound);
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: GoalId) -> Result<(), GoalError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE task SET goal_id = NULL WHERE goal_id = ?1", [id.0])?;
    if tx.execute("DELETE FROM goal WHERE id = ?1", [id.0])? == 0 {
        return Err(GoalError::NotFound);
    }
    tx.commit()?;
    Ok(())
}

const SELECT: &str = "SELECT id, title, emoji, description, kind, deadline,
    (SELECT count(*) FROM task WHERE goal_id = goal.id AND done_at IS NOT NULL),
    (SELECT count(*) FROM task WHERE goal_id = goal.id)
    FROM goal";

fn row(r: &rusqlite::Row, today: NaiveDate) -> rusqlite::Result<Result<Goal, GoalError>> {
    let kind: String = r.get(4)?;
    let deadline: Option<String> = r.get(5)?;
    let (id, title, emoji, description) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (tasks_done, tasks_total): (u32, u32) = (r.get(6)?, r.get(7)?);
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
