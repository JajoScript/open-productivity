// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Flowtime and Pomodoro: one running session, persisted so it survives a restart.

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{StudyError, SubjectId, log_manual_session, rejection};
use crate::tasks::TaskId;

/// A gap since the last banked instant longer than this (app closed, PC asleep) is not studied time.
pub const MAX_GAP_SECS: i64 = 120;

/// A Pomodoro length: minutes of focus, then a break.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    P25,
    P50,
    P15,
}

impl Preset {
    pub fn focus_minutes(self) -> i64 {
        match self {
            Preset::P25 => 25,
            Preset::P50 => 50,
            Preset::P15 => 15,
        }
    }

    pub fn break_minutes(self) -> i64 {
        match self {
            Preset::P25 => 5,
            Preset::P50 => 10,
            Preset::P15 => 3,
        }
    }

    /// The integer stored in `study_active.preset` (the focus minutes).
    pub fn to_db(self) -> i64 {
        self.focus_minutes()
    }

    pub fn from_db(n: i64) -> Option<Preset> {
        [Preset::P25, Preset::P50, Preset::P15]
            .into_iter()
            .find(|p| p.to_db() == n)
    }

    fn phase_secs(self, phase: Phase) -> i64 {
        60 * match phase {
            Phase::Focus => self.focus_minutes(),
            Phase::Break => self.break_minutes(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    Break,
}

/// Where a Pomodoro stands: the phase, what is left of it and the pomodoros completed.
#[derive(Debug, PartialEq)]
pub struct PomodoroStatus {
    pub preset: Preset,
    pub phase: Phase,
    pub left_secs: i64,
    pub pomodoros: i64,
}

/// The counters a session banks. `counted_secs` is focus time only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clock {
    counted_secs: i64,
    on_break: bool,
    phase_secs: i64,
    pomodoros: i64,
}

impl Clock {
    fn phase(self) -> Phase {
        if self.on_break {
            Phase::Break
        } else {
            Phase::Focus
        }
    }

    /// Adds `secs` of running time, crossing as many focus/break borders as it spans. A Flowtime
    /// session (no preset) only counts.
    fn advance(mut self, preset: Option<Preset>, mut secs: i64) -> Clock {
        let Some(preset) = preset else {
            self.counted_secs += secs;
            return self;
        };
        while secs > 0 {
            let len = preset.phase_secs(self.phase());
            let take = secs.min(len - self.phase_secs);
            secs -= take;
            self.phase_secs += take;
            if !self.on_break {
                self.counted_secs += take;
            }
            if self.phase_secs == len {
                self.pomodoros += i64::from(!self.on_break);
                self.on_break = !self.on_break;
                self.phase_secs = 0;
            }
        }
        self
    }

    fn status(self, preset: Option<Preset>) -> Option<PomodoroStatus> {
        let preset = preset?;
        Some(PomodoroStatus {
            preset,
            phase: self.phase(),
            left_secs: preset.phase_secs(self.phase()) - self.phase_secs,
            pomodoros: self.pomodoros,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct ActiveSession {
    pub subject: SubjectId,
    pub subject_name: String,
    /// Title of the topic, if it still exists.
    pub topic: Option<String>,
    pub elapsed_secs: i64,
    pub paused: bool,
    /// `None` for Flowtime.
    pub pomodoro: Option<PomodoroStatus>,
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

/// The stored row.
struct Stored {
    subject: i64,
    topic: Option<i64>,
    started_on: String,
    clock: Clock,
    last_seen: String,
    paused: bool,
    preset: Option<Preset>,
}

fn row(conn: &Connection) -> Result<Option<Stored>, StudyError> {
    Ok(conn
        .query_row(
            "SELECT subject_id, topic_id, started_on, counted_secs, last_seen, paused,
                    preset, on_break, phase_secs, pomodoros
             FROM study_active",
            [],
            |r| {
                Ok(Stored {
                    subject: r.get(0)?,
                    topic: r.get(1)?,
                    started_on: r.get(2)?,
                    last_seen: r.get(4)?,
                    paused: r.get(5)?,
                    preset: r.get::<_, Option<i64>>(6)?.and_then(Preset::from_db),
                    clock: Clock {
                        counted_secs: r.get(3)?,
                        on_break: r.get(7)?,
                        phase_secs: r.get(8)?,
                        pomodoros: r.get(9)?,
                    },
                })
            },
        )
        .optional()?)
}

/// Same subject and topic rules as `log_manual_session`. Only one session can run. No preset
/// means Flowtime.
pub fn start_session(
    conn: &Connection,
    subject: SubjectId,
    topic: Option<TaskId>,
    now: DateTime<FixedOffset>,
    preset: Option<Preset>,
) -> Result<(), StudyError> {
    if row(conn)?.is_some() {
        return Err(StudyError::SessionActive);
    }
    let rows = conn.execute(
        "INSERT INTO study_active (id, subject_id, topic_id, started_on, last_seen, preset)
         SELECT 1, id, ?2, ?3, ?4, ?5 FROM subject WHERE id = ?1 AND archived = 0
           AND (?2 IS NULL OR EXISTS (SELECT 1 FROM task
                WHERE id = ?2 AND subject_id = subject.id AND done_at IS NULL))",
        (
            subject.0,
            topic.map(|t| t.0),
            now.date_naive().to_string(),
            utc_text(now),
            preset.map(Preset::to_db),
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
    let Some(st) = row(conn)? else {
        return Ok(None);
    };
    let (subject_name, topic) = conn.query_row(
        "SELECT (SELECT name FROM subject WHERE id = ?1), (SELECT title FROM task WHERE id = ?2)",
        (st.subject, st.topic),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )?;
    let (add, paused) = advance(st.paused, &st.last_seen, now)?;
    let clock = st.clock.advance(st.preset, add);
    Ok(Some(ActiveSession {
        subject: SubjectId(st.subject),
        subject_name,
        topic,
        elapsed_secs: clock.counted_secs,
        paused,
        pomodoro: clock.status(st.preset),
    }))
}

/// Banks the gap into the session (a long gap pauses it). Does nothing without one.
pub fn heartbeat(conn: &Connection, now: DateTime<FixedOffset>) -> Result<(), StudyError> {
    let Some(st) = row(conn)? else {
        return Ok(());
    };
    let (add, paused) = advance(st.paused, &st.last_seen, now)?;
    let c = st.clock.advance(st.preset, add);
    conn.execute(
        "UPDATE study_active SET counted_secs = ?1, on_break = ?2, phase_secs = ?3,
                pomodoros = ?4, last_seen = ?5, paused = ?6",
        (
            c.counted_secs,
            c.on_break,
            c.phase_secs,
            c.pomodoros,
            utc_text(now),
            paused,
        ),
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
    let Some(Stored {
        subject,
        topic,
        started_on,
        clock,
        ..
    }) = row(&tx)?
    else {
        return Err(StudyError::NoActiveSession);
    };
    let minutes = clock.counted_secs / 60;
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

#[cfg(test)]
mod tests {
    use super::*;

    const FRESH: Clock = Clock {
        counted_secs: 0,
        on_break: false,
        phase_secs: 0,
        pomodoros: 0,
    };

    #[test]
    fn flowtime_only_counts() {
        let c = FRESH.advance(None, 5000);
        assert_eq!(
            (c.counted_secs, c.on_break, c.phase_secs, c.pomodoros),
            (5000, false, 0, 0)
        );
    }

    #[test]
    fn focus_ends_in_a_break_that_does_not_count() {
        let c = FRESH.advance(Some(Preset::P15), 900);
        assert_eq!(
            (c.counted_secs, c.on_break, c.phase_secs, c.pomodoros),
            (900, true, 0, 1)
        );
        let c = c.advance(Some(Preset::P15), 100);
        assert_eq!(
            (c.counted_secs, c.on_break, c.phase_secs, c.pomodoros),
            (900, true, 100, 1)
        );
    }

    #[test]
    fn one_span_crosses_several_borders() {
        // 15 focus + 3 break + 1 focus + leftover 30s.
        let c = FRESH.advance(Some(Preset::P15), 900 + 180 + 900 + 30);
        assert_eq!(
            (c.counted_secs, c.on_break, c.phase_secs, c.pomodoros),
            (1800, true, 30, 2)
        );
        let c = FRESH.advance(Some(Preset::P15), 900 + 180 + 30);
        assert_eq!(
            (c.counted_secs, c.on_break, c.phase_secs, c.pomodoros),
            (930, false, 30, 1)
        );
    }

    #[test]
    fn presets_round_trip_and_invalid_is_none() {
        for p in [Preset::P25, Preset::P50, Preset::P15] {
            assert_eq!(Preset::from_db(p.to_db()), Some(p));
        }
        assert_eq!(Preset::from_db(30), None);
    }
}
