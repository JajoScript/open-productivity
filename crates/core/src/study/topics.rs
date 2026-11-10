// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Topics: tasks that belong to a subject.

use chrono::{DateTime, FixedOffset, NaiveDate};
use rusqlite::Connection;

use super::{StudyError, SubjectId, missing_or_archived};
use crate::tasks::{TaskId, local_midnight};

#[derive(Debug, PartialEq)]
pub struct Topic {
    pub id: TaskId,
    pub title: String,
    pub subject: SubjectId,
    pub subject_name: String,
    pub due: Option<NaiveDate>,
    pub done: bool,
}

/// Trims the title. Any date is valid, past ones too. Marking it done is `tasks::set_done`.
pub fn create_topic(
    conn: &Connection,
    subject: SubjectId,
    title: &str,
    due: Option<NaiveDate>,
) -> Result<TaskId, StudyError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(StudyError::EmptyTitle);
    }
    // One statement: it inserts only if the subject exists and is not archived.
    let rows = conn.execute(
        "INSERT INTO task (title, due_date, subject_id)
         SELECT ?2, ?3, id FROM subject WHERE id = ?1 AND archived = 0",
        (subject.0, title, due.map(|d| d.to_string())),
    )?;
    if rows == 0 {
        return Err(missing_or_archived(conn, subject));
    }
    Ok(TaskId(conn.last_insert_rowid()))
}

/// Topics of active subjects, pending or done today (local day of `now`). Pending first, then
/// by due date (undated last), creation and id.
pub fn list_topics(
    conn: &Connection,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Topic>, StudyError> {
    let midnight = local_midnight(now).map_err(|e| StudyError::BadDate(e.to_string()))?;
    query_topics(conn, "(t.done_at IS NULL OR t.done_at >= ?1)", [midnight])
}

/// The topics of `list_topics` due exactly today, the same rule as the Tasks Today view.
pub fn today_topics(
    conn: &Connection,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Topic>, StudyError> {
    let midnight = local_midnight(now).map_err(|e| StudyError::BadDate(e.to_string()))?;
    query_topics(
        conn,
        "(t.done_at IS NULL OR t.done_at >= ?1) AND t.due_date = ?2",
        (midnight, now.date_naive().to_string()),
    )
}

/// The pending topics of one active subject, in `list_topics` order: what a session can point at.
pub fn subject_topics(conn: &Connection, subject: SubjectId) -> Result<Vec<Topic>, StudyError> {
    query_topics(conn, "t.done_at IS NULL AND s.id = ?1", [subject.0])
}

/// `condition` is a literal written here in `study`, never user input; values go in `params`.
fn query_topics(
    conn: &Connection,
    condition: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Topic>, StudyError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t.id, t.title, s.id, s.name, t.due_date, t.done_at IS NOT NULL
         FROM task t JOIN subject s ON s.id = t.subject_id
         WHERE s.archived = 0 AND {condition}
         ORDER BY t.done_at IS NOT NULL, t.due_date IS NULL, t.due_date, t.created_at, t.id"
    ))?;
    let rows = stmt.query_map(params, |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get(5)?,
        ))
    })?;
    rows.map(|row| {
        let (id, title, subject, subject_name, due, done) = row?;
        Ok(Topic {
            id: TaskId(id),
            title,
            subject: SubjectId(subject),
            subject_name,
            due: due
                .map(|d| d.parse().map_err(|_| StudyError::BadDate(d)))
                .transpose()?,
            done,
        })
    })
    .collect()
}
