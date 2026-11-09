// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Study: subjects and the time studied on each.

use std::fmt;

use chrono::NaiveDate;
use rusqlite::Connection;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubjectId(pub i64);

#[derive(Debug, PartialEq)]
pub struct Subject {
    pub id: SubjectId,
    pub name: String,
    pub total_minutes: i64,
    pub sessions: i64,
}

#[derive(Debug)]
pub enum StudyError {
    EmptyName,
    /// Minutes are not a positive whole number.
    BadMinutes,
    /// The session day is after today.
    FutureDate,
    SubjectNotFound,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for StudyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StudyError::EmptyName => write!(f, "the name is empty"),
            StudyError::BadMinutes => write!(f, "minutes must be a positive whole number"),
            StudyError::FutureDate => write!(f, "that day has not happened yet"),
            StudyError::SubjectNotFound => write!(f, "subject not found"),
            StudyError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl From<rusqlite::Error> for StudyError {
    fn from(e: rusqlite::Error) -> Self {
        StudyError::Sqlite(e)
    }
}

/// Trims the name.
pub fn create_subject(conn: &Connection, name: &str) -> Result<SubjectId, StudyError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StudyError::EmptyName);
    }
    conn.execute("INSERT INTO subject (name) VALUES (?1)", [name])?;
    Ok(SubjectId(conn.last_insert_rowid()))
}

/// Oldest first, with minutes studied and number of sessions.
pub fn list_subjects(conn: &Connection) -> Result<Vec<Subject>, StudyError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, COALESCE(SUM(l.focus_minutes), 0), COUNT(l.id)
         FROM subject s LEFT JOIN study_session l ON l.subject_id = s.id
         GROUP BY s.id ORDER BY s.created_at, s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Subject {
            id: SubjectId(r.get(0)?),
            name: r.get(1)?,
            total_minutes: r.get(2)?,
            sessions: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A positive whole number typed in a text field; surrounding spaces are ignored.
pub fn parse_minutes(text: &str) -> Result<i64, StudyError> {
    let text = text.trim();
    // Digits only: no sign, no separators. Overflow fails the parse.
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(StudyError::BadMinutes);
    }
    text.parse()
        .ok()
        .filter(|&n| n > 0)
        .ok_or(StudyError::BadMinutes)
}

/// Logs time studied on `date`, which may be today but not later.
pub fn log_manual_session(
    conn: &Connection,
    subject: SubjectId,
    date: NaiveDate,
    minutes: i64,
    description: &str,
    today: NaiveDate,
) -> Result<(), StudyError> {
    if minutes <= 0 {
        return Err(StudyError::BadMinutes);
    }
    if date > today {
        return Err(StudyError::FutureDate);
    }
    // One statement: it inserts only if the subject exists.
    let rows = conn.execute(
        "INSERT INTO study_session (subject_id, date, focus_minutes, description)
         SELECT id, ?2, ?3, ?4 FROM subject WHERE id = ?1",
        (subject.0, date.to_string(), minutes, description.trim()),
    )?;
    if rows == 0 {
        return Err(StudyError::SubjectNotFound);
    }
    Ok(())
}

/// `0m`, `45m`, `2h`, `1h 30m`.
pub fn format_duration(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}
