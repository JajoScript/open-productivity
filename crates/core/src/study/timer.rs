// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Flowtime: one running session, persisted so it survives a restart.

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension};

use super::{StudyError, SubjectId, log_manual_session, rejection};
use crate::tasks::TaskId;

/// A gap since the last banked instant longer than this (app closed, PC asleep) is not studied time.
pub const MAX_GAP_SECS: i64 = 120;

#[derive(Debug, PartialEq)]
pub struct ActiveSession {
    pub subject: SubjectId,
    pub subject_name: String,
    /// Title of the topic, if it still exists.
    pub topic: Option<String>,
    pub elapsed_secs: i64,
    pub paused: bool,
}

fn utc_text(now: DateTime<FixedOffset>) -> String {
    now.with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Seconds to add for the time since `last_seen`, and whether the session is paused by now. A
/// paused session adds nothing; a gap past `MAX_GAP_SECS` (asleep, closed) adds nothing and pauses;
/// a clock that went back adds nothing and does not pause.
fn advance(
    paused: bool,
    last_seen: &str,
    now: DateTime<FixedOffset>,
) -> Result<(i64, bool), StudyError> {
    if paused {
        return Ok((0, true));
    }
    let last = DateTime::parse_from_rfc3339(last_seen)
        .map_err(|_| StudyError::BadDate(last_seen.to_string()))?;
    let gap = now.signed_duration_since(last).num_seconds();
    Ok(match gap {
        0..=MAX_GAP_SECS => (gap, false),
        ..0 => (0, false),
        _ => (0, true),
    })
}

/// The stored row: subject, topic, started_on, counted seconds, last_seen and paused.
type Row = (i64, Option<i64>, String, i64, String, bool);

fn row(conn: &Connection) -> Result<Option<Row>, StudyError> {
    Ok(conn
        .query_row(
            "SELECT subject_id, topic_id, started_on, counted_secs, last_seen, paused
             FROM study_active",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?)
}

/// Same subject and topic rules as `log_manual_session`. Only one session can run.
pub fn start_session(
    conn: &Connection,
    subject: SubjectId,
    topic: Option<TaskId>,
    now: DateTime<FixedOffset>,
) -> Result<(), StudyError> {
    if row(conn)?.is_some() {
        return Err(StudyError::SessionActive);
    }
    let rows = conn.execute(
        "INSERT INTO study_active (id, subject_id, topic_id, started_on, last_seen)
         SELECT 1, id, ?2, ?3, ?4 FROM subject WHERE id = ?1 AND archived = 0
           AND (?2 IS NULL OR EXISTS (SELECT 1 FROM task
                WHERE id = ?2 AND subject_id = subject.id AND done_at IS NULL))",
        (
            subject.0,
            topic.map(|t| t.0),
            now.date_naive().to_string(),
            utc_text(now),
        ),
    )?;
    if rows == 0 {
        return Err(rejection(conn, subject));
    }
    Ok(())
}

/// Read-only: the banked time plus the gap since the last bank, if the gap is short and the
/// session is not paused.
pub fn active_session(
    conn: &Connection,
    now: DateTime<FixedOffset>,
) -> Result<Option<ActiveSession>, StudyError> {
    let Some((subject, topic, _, counted, last_seen, paused)) = row(conn)? else {
        return Ok(None);
    };
    let (subject_name, topic) = conn.query_row(
        "SELECT (SELECT name FROM subject WHERE id = ?1), (SELECT title FROM task WHERE id = ?2)",
        (subject, topic),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )?;
    let (add, paused) = advance(paused, &last_seen, now)?;
    Ok(Some(ActiveSession {
        subject: SubjectId(subject),
        subject_name,
        topic,
        elapsed_secs: counted + add,
        paused,
    }))
}

/// Banks the gap into the session (a long gap pauses it). Does nothing without one.
pub fn heartbeat(conn: &Connection, now: DateTime<FixedOffset>) -> Result<(), StudyError> {
    let Some((_, _, _, _, last_seen, paused)) = row(conn)? else {
        return Ok(());
    };
    let (add, paused) = advance(paused, &last_seen, now)?;
    conn.execute(
        "UPDATE study_active SET counted_secs = counted_secs + ?1, last_seen = ?2, paused = ?3",
        (add, utc_text(now), paused),
    )?;
    Ok(())
}

/// Banks the time so far and stops counting. Pausing a paused session does nothing.
pub fn pause_session(conn: &Connection, now: DateTime<FixedOffset>) -> Result<(), StudyError> {
    let tx = conn.unchecked_transaction()?;
    if row(&tx)?.is_none() {
        return Err(StudyError::NoActiveSession);
    }
    heartbeat(&tx, now)?;
    tx.execute("UPDATE study_active SET paused = 1", [])?;
    tx.commit()?;
    Ok(())
}

/// Starts counting again from `now`; the pause is not counted. Resuming a running session does
/// nothing.
pub fn resume_session(conn: &Connection, now: DateTime<FixedOffset>) -> Result<(), StudyError> {
    if row(conn)?.is_none() {
        return Err(StudyError::NoActiveSession);
    }
    conn.execute(
        "UPDATE study_active SET paused = 0, last_seen = ?1 WHERE paused = 1",
        [utc_text(now)],
    )?;
    Ok(())
}

/// Saves the whole minutes counted as a session on the day it started and returns them. Under a
/// minute, or if the session cannot be saved, nothing changes and it keeps running.
pub fn finish_session(conn: &Connection, now: DateTime<FixedOffset>) -> Result<i64, StudyError> {
    let tx = conn.unchecked_transaction()?;
    heartbeat(&tx, now)?;
    let Some((subject, topic, started_on, counted, _, _)) = row(&tx)? else {
        return Err(StudyError::NoActiveSession);
    };
    let minutes = counted / 60;
    if minutes < 1 {
        return Err(StudyError::TooShort);
    }
    let day = started_on
        .parse()
        .map_err(|_| StudyError::BadDate(started_on))?;
    log_manual_session(
        &tx,
        SubjectId(subject),
        day,
        minutes,
        "",
        topic.map(TaskId),
        day,
    )?;
    tx.execute("DELETE FROM study_active", [])?;
    tx.commit()?;
    Ok(minutes)
}

pub fn discard_session(conn: &Connection) -> Result<(), StudyError> {
    if conn.execute("DELETE FROM study_active", [])? == 0 {
        return Err(StudyError::NoActiveSession);
    }
    Ok(())
}

/// `MM:SS` under an hour, `H:MM:SS` from one hour.
pub fn format_clock(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// A fifth of the focus time, at least a minute.
pub fn suggested_break(minutes: i64) -> i64 {
    (minutes / 5).max(1)
}
