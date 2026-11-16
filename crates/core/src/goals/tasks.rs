// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tasks that belong to a goal. Marking one done is `tasks::set_done`.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::{GoalError, GoalId};
use crate::tasks::TaskId;

#[derive(Debug, PartialEq)]
pub struct GoalTask {
    pub id: TaskId,
    pub title: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
}

/// Trims the title. Any date is valid, past ones too.
pub fn create_task(
    conn: &Connection,
    goal: GoalId,
    title: &str,
    due: Option<NaiveDate>,
) -> Result<TaskId, GoalError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(GoalError::EmptyTitle);
    }
    // One statement: it inserts only if the goal exists.
    let rows = conn.execute(
        "INSERT INTO task (title, due_date, goal_id) SELECT ?2, ?3, id FROM goal WHERE id = ?1",
        (goal.0, title, due.map(|d| d.to_string())),
    )?;
    if rows == 0 {
        return Err(GoalError::NotFound);
    }
    Ok(TaskId(conn.last_insert_rowid()))
}

/// Pending first, then done; each group by creation and id.
pub fn list_tasks(conn: &Connection, goal: GoalId) -> Result<Vec<GoalTask>, GoalError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, done_at IS NOT NULL, due_date FROM task WHERE goal_id = ?1
         ORDER BY done_at IS NOT NULL, created_at, id",
    )?;
    let rows = stmt.query_map([goal.0], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    rows.map(|row| {
        let (id, title, done, due) = row?;
        Ok(GoalTask {
            id: TaskId(id),
            title,
            done,
            due: due
                .map(|d| d.parse().map_err(|_| GoalError::BadDate(d)))
                .transpose()?,
        })
    })
    .collect()
}
