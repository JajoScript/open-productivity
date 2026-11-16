// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Progress entries of a numeric goal: a dated value that adds to its total.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::{DATE_FORMAT, GoalError, GoalId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntryId(pub i64);

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub id: EntryId,
    pub date: NaiveDate,
    pub value: i64,
}

/// Any date is valid, future ones too. The goal must exist and be numeric.
pub fn add_entry(
    conn: &Connection,
    goal: GoalId,
    date: NaiveDate,
    value: i64,
) -> Result<EntryId, GoalError> {
    if value <= 0 {
        return Err(GoalError::BadValue);
    }
    // One statement: it inserts only if the goal exists and has a target.
    let rows = conn.execute(
        "INSERT INTO goal_entry (goal_id, date, value)
         SELECT id, ?2, ?3 FROM goal WHERE id = ?1 AND target_count IS NOT NULL",
        (goal.0, date.format(DATE_FORMAT).to_string(), value),
    )?;
    if rows == 0 {
        return Err(GoalError::NotNumeric);
    }
    Ok(EntryId(conn.last_insert_rowid()))
}

/// Most recent first: date descending, then newest id.
pub fn list_entries(conn: &Connection, goal: GoalId) -> Result<Vec<Entry>, GoalError> {
    let mut stmt = conn.prepare(
        "SELECT id, date, value FROM goal_entry WHERE goal_id = ?1 ORDER BY date DESC, id DESC",
    )?;
    let rows = stmt.query_map([goal.0], |r| {
        Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?))
    })?;
    rows.map(|row| {
        let (id, date, value) = row?;
        Ok(Entry {
            id: EntryId(id),
            date: NaiveDate::parse_from_str(&date, DATE_FORMAT)
                .map_err(|_| GoalError::BadDate(date))?,
            value,
        })
    })
    .collect()
}

pub fn delete_entry(conn: &Connection, id: EntryId) -> Result<(), GoalError> {
    if conn.execute("DELETE FROM goal_entry WHERE id = ?1", [id.0])? == 0 {
        return Err(GoalError::NotFound);
    }
    Ok(())
}
