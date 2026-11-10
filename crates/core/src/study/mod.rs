// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Study: subjects and the time studied on each.

use std::fmt;

use chrono::NaiveDate;
use rusqlite::Connection;

use crate::tasks::TaskId;

#[cfg(test)]
mod tests;
mod timer;
mod topics;

pub use timer::*;
pub use topics::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubjectId(pub i64);

#[derive(Debug, PartialEq)]
pub struct Subject {
    pub id: SubjectId,
    pub name: String,
    pub total_minutes: i64,
    pub sessions: i64,
    pub archived: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionId(pub i64);

#[derive(Debug, PartialEq)]
pub struct Session {
    pub id: SessionId,
    pub date: NaiveDate,
    pub minutes: i64,
    pub description: String,
    /// Title of the topic studied, if the session points at one.
    pub topic: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SubjectFilter {
    Active,
    Archived,
    All,
}

#[derive(Debug)]
pub enum StudyError {
    EmptyName,
    EmptyTitle,
    /// Minutes are not a positive whole number.
    BadMinutes,
    /// The session day is after today.
    FutureDate,
    SubjectNotFound,
    /// Archived subjects take no new sessions.
    SubjectArchived,
    /// The topic is missing, done, or belongs to another subject.
    TopicNotFound,
    SessionNotFound,
    /// A Flowtime session is already running.
    SessionActive,
    NoActiveSession,
    /// Under a minute studied: nothing worth saving.
    TooShort,
    /// A stored session date that cannot be read.
    BadDate(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for StudyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StudyError::EmptyName => write!(f, "the name is empty"),
            StudyError::EmptyTitle => write!(f, "the title is empty"),
            StudyError::BadMinutes => write!(f, "minutes must be a positive whole number"),
            StudyError::FutureDate => write!(f, "that day has not happened yet"),
            StudyError::SubjectNotFound => write!(f, "subject not found"),
            StudyError::SubjectArchived => write!(f, "subject is archived"),
            StudyError::TopicNotFound => write!(f, "topic not found"),
            StudyError::SessionNotFound => write!(f, "session not found"),
            StudyError::SessionActive => write!(f, "a session is already running"),
            StudyError::NoActiveSession => write!(f, "no session is running"),
            StudyError::TooShort => write!(f, "the session is shorter than a minute"),
            StudyError::BadDate(d) => write!(f, "invalid stored date: {d}"),
            StudyError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for StudyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StudyError::Sqlite(e) => Some(e),
            _ => None,
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

/// Oldest first, with minutes studied and number of sessions. Archived subjects keep their totals.
pub fn list_subjects(conn: &Connection, filter: SubjectFilter) -> Result<Vec<Subject>, StudyError> {
    let wanted = match filter {
        SubjectFilter::Active => "s.archived = 0",
        SubjectFilter::Archived => "s.archived = 1",
        SubjectFilter::All => "1",
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT s.id, s.name, COALESCE(SUM(l.focus_minutes), 0), COUNT(l.id), s.archived
         FROM subject s LEFT JOIN study_session l ON l.subject_id = s.id
         WHERE {wanted} GROUP BY s.id ORDER BY s.created_at, s.id"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok(Subject {
            id: SubjectId(r.get(0)?),
            name: r.get(1)?,
            total_minutes: r.get(2)?,
            sessions: r.get(3)?,
            archived: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Active or archived.
pub fn get_subject(conn: &Connection, id: SubjectId) -> Result<Subject, StudyError> {
    // shortcut: scans every subject, fine for a personal list; add a WHERE if it ever grows.
    list_subjects(conn, SubjectFilter::All)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or(StudyError::SubjectNotFound)
}

/// Trims the name.
pub fn rename_subject(conn: &Connection, id: SubjectId, name: &str) -> Result<(), StudyError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StudyError::EmptyName);
    }
    let rows = conn.execute("UPDATE subject SET name = ?2 WHERE id = ?1", (id.0, name))?;
    if rows == 0 {
        return Err(StudyError::SubjectNotFound);
    }
    Ok(())
}

/// Archiving is reversible and keeps every session.
pub fn set_subject_archived(
    conn: &Connection,
    id: SubjectId,
    archived: bool,
) -> Result<(), StudyError> {
    let rows = conn.execute(
        "UPDATE subject SET archived = ?2 WHERE id = ?1",
        (id.0, archived),
    )?;
    if rows == 0 {
        return Err(StudyError::SubjectNotFound);
    }
    Ok(())
}

/// Newest first: by day, then by when it was logged.
pub fn list_sessions(conn: &Connection, subject: SubjectId) -> Result<Vec<Session>, StudyError> {
    let mut stmt = conn.prepare(
        "SELECT l.id, l.date, l.focus_minutes, l.description, t.title
         FROM study_session l LEFT JOIN task t ON t.id = l.topic_id
         WHERE l.subject_id = ?1 ORDER BY l.date DESC, l.id DESC",
    )?;
    let rows = stmt.query_map([subject.0], |r| {
        Ok((
            r.get(0)?,
            r.get::<_, String>(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
        ))
    })?;
    rows.map(|row| {
        let (id, date, minutes, description, topic) = row?;
        Ok(Session {
            id: SessionId(id),
            date: date.parse().map_err(|_| StudyError::BadDate(date))?,
            minutes,
            description,
            topic,
        })
    })
    .collect()
}

pub fn delete_session(conn: &Connection, id: SessionId) -> Result<(), StudyError> {
    let rows = conn.execute("DELETE FROM study_session WHERE id = ?1", [id.0])?;
    if rows == 0 {
        return Err(StudyError::SessionNotFound);
    }
    Ok(())
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

/// Logs time studied on `date`, which may be today but not later. A `topic` must be a pending
/// topic of the same subject.
pub fn log_manual_session(
    conn: &Connection,
    subject: SubjectId,
    date: NaiveDate,
    minutes: i64,
    description: &str,
    topic: Option<TaskId>,
    today: NaiveDate,
) -> Result<(), StudyError> {
    if minutes <= 0 {
        return Err(StudyError::BadMinutes);
    }
    if date > today {
        return Err(StudyError::FutureDate);
    }
    // One statement: it inserts only if the subject exists and is not archived, and the topic,
    // if any, is a pending task of that subject.
    let rows = conn.execute(
        "INSERT INTO study_session (subject_id, date, focus_minutes, description, topic_id)
         SELECT id, ?2, ?3, ?4, ?5 FROM subject WHERE id = ?1 AND archived = 0
           AND (?5 IS NULL OR EXISTS (SELECT 1 FROM task
                WHERE id = ?5 AND subject_id = subject.id AND done_at IS NULL))",
        (
            subject.0,
            date.to_string(),
            minutes,
            description.trim(),
            topic.map(|t| t.0),
        ),
    )?;
    if rows == 0 {
        return Err(rejection(conn, subject));
    }
    Ok(())
}

/// Why a session insert on `subject` touched no row.
fn rejection(conn: &Connection, subject: SubjectId) -> StudyError {
    match conn.query_row(
        "SELECT archived FROM subject WHERE id = ?1",
        [subject.0],
        |r| r.get::<_, bool>(0),
    ) {
        Ok(true) => StudyError::SubjectArchived,
        // The subject takes sessions, so the topic was the reason.
        Ok(false) => StudyError::TopicNotFound,
        Err(rusqlite::Error::QueryReturnedNoRows) => StudyError::SubjectNotFound,
        Err(e) => e.into(),
    }
}

/// `0m`, `45m`, `2h`, `1h 30m`.
pub fn format_duration(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// Why an insert on an active subject touched no row.
fn missing_or_archived(conn: &Connection, subject: SubjectId) -> StudyError {
    match conn.query_row("SELECT 1 FROM subject WHERE id = ?1", [subject.0], |_| {
        Ok(())
    }) {
        Ok(()) => StudyError::SubjectArchived,
        Err(rusqlite::Error::QueryReturnedNoRows) => StudyError::SubjectNotFound,
        Err(e) => e.into(),
    }
}
