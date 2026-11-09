// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Habits: create them and mark them done per day.

use std::collections::HashSet;
use std::fmt;

use chrono::{Datelike, Months, NaiveDate, TimeDelta};
use rusqlite::{Connection, OptionalExtension};

use crate::calendar;

mod month;
#[cfg(test)]
mod tests;

pub use month::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HabitId(pub i64);

/// What one habit shows on one day of a month.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DayState {
    /// Applies, was done.
    Done,
    /// Applies, not done, up to today: can be marked.
    Open,
    /// Does not apply: weekday not selected or before the start. A mark stored there is ignored.
    Skipped,
    /// Applies but after today: cannot be marked.
    Future,
}

/// A habit and its state on every day of a month.
#[derive(Debug, PartialEq)]
pub struct MonthHabit {
    pub id: HabitId,
    pub name: String,
    /// One per day of the month, day 1 first.
    pub days: Vec<DayState>,
    pub stats: HabitStats,
    /// The weekdays it applies on, Monday first (to prefill the editor).
    pub weekdays: [bool; 7],
    pub start: NaiveDate,
}

/// Marks done out of days that apply, summed over habits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Total {
    pub done: u32,
    pub applicable: u32,
}

impl<'a> std::iter::Sum<&'a Total> for Total {
    fn sum<I: Iterator<Item = &'a Total>>(iter: I) -> Self {
        iter.fold(Total::default(), |a, t| Total {
            done: a.done + t.done,
            applicable: a.applicable + t.applicable,
        })
    }
}

impl Total {
    /// Done share floored; `None` when nothing applies.
    pub fn pct(self) -> Option<u32> {
        (self.done * 100).checked_div(self.applicable)
    }
}

/// How one day of the month heat map is shaded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heat {
    /// No habit applies.
    None,
    /// After today.
    Future,
    /// Share done: L0 is 0%, L1 1-25, L2 26-50, L3 51-75, L4 76-100.
    L0,
    L1,
    L2,
    L3,
    L4,
}

#[derive(Debug, PartialEq)]
pub struct HabitMonth {
    /// First day of the month shown.
    pub first: NaiveDate,
    /// Number of days in the month.
    pub days: u32,
    /// Day of the month that is today, when today falls in it.
    pub today: Option<u32>,
    /// Whether a later month can be shown (never past the current one).
    pub can_next: bool,
    /// Active habits, oldest first.
    pub habits: Vec<MonthHabit>,
    /// Per day of the month over the active habits. Future days apply; skipped ones do not.
    pub day_totals: Vec<Total>,
    /// Per block of seven days (the last may be short).
    pub week_totals: Vec<Total>,
    /// Sum of `day_totals`.
    pub month_total: Total,
    /// Blank cells before day 1 so the heat map starts on Monday.
    pub lead: u32,
    /// Heat per day of the month, day 1 first.
    pub heat: Vec<Heat>,
}

#[derive(Debug)]
pub enum HabitError {
    EmptyName,
    /// No weekday is selected.
    NoWeekdays,
    /// The habit would start after today.
    FutureStart,
    NotFound,
    /// The day is after today.
    Future,
    /// The day is before the habit started.
    BeforeStart,
    /// The habit is not scheduled on that weekday.
    NotScheduled,
    /// The month is after the current one.
    FutureMonth,
    /// The month is outside the supported dates.
    OutOfRange,
    /// A stored date could not be read.
    BadDate(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for HabitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HabitError::EmptyName => write!(f, "the name is empty"),
            HabitError::NoWeekdays => write!(f, "no weekday is selected"),
            HabitError::FutureStart => write!(f, "the habit cannot start in the future"),
            HabitError::NotFound => write!(f, "habit not found"),
            HabitError::Future => write!(f, "that day has not happened yet"),
            HabitError::BeforeStart => write!(f, "the habit had not started on that day"),
            HabitError::NotScheduled => write!(f, "the habit does not apply on that weekday"),
            HabitError::FutureMonth => write!(f, "that month has not happened yet"),
            HabitError::OutOfRange => write!(f, "the month is out of range"),
            HabitError::BadDate(d) => write!(f, "invalid stored date: {d}"),
            HabitError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for HabitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HabitError::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for HabitError {
    fn from(e: rusqlite::Error) -> Self {
        HabitError::Sqlite(e)
    }
}

/// Bit of `weekdays` for the date: bit 0 is Monday.
fn weekday_bit(date: NaiveDate) -> i64 {
    1 << date.weekday().num_days_from_monday()
}

/// A stored YYYY-MM-DD date; a corrupt one is an error, never a silent default.
fn parse_date(s: String) -> Result<NaiveDate, HabitError> {
    s.parse().map_err(|_| HabitError::BadDate(s))
}

/// The weekdays a habit applies on; never empty. Bit 0 is Monday.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weekdays(u8);

impl Weekdays {
    pub const ALL: Weekdays = Weekdays(127);

    /// From the seven flags, Monday first.
    pub fn from_flags(flags: [bool; 7]) -> Result<Self, HabitError> {
        let bits = flags
            .iter()
            .enumerate()
            .fold(0u8, |acc, (i, &on)| acc | (u8::from(on) << i));
        if bits == 0 {
            return Err(HabitError::NoWeekdays);
        }
        Ok(Weekdays(bits))
    }
}

/// Creates a habit on the flagged `days` (Monday first), starting on `start` (today or earlier).
pub fn create(
    conn: &Connection,
    name: &str,
    days: [bool; 7],
    start: NaiveDate,
    today: NaiveDate,
) -> Result<HabitId, HabitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(HabitError::EmptyName);
    }
    let weekdays = Weekdays::from_flags(days)?;
    if start > today {
        return Err(HabitError::FutureStart);
    }
    conn.execute(
        "INSERT INTO habit (name, weekdays, start_date) VALUES (?1, ?2, ?3)",
        (name, weekdays.0, start.to_string()),
    )?;
    Ok(HabitId(conn.last_insert_rowid()))
}

/// Changes name, weekdays and start of a habit, with the rules of `create`. Retroactive: the
/// logs stay as they are and days and stats recompute, so marks on days that no longer apply are
/// ignored until they apply again.
pub fn update(
    conn: &Connection,
    id: HabitId,
    name: &str,
    days: [bool; 7],
    start: NaiveDate,
    today: NaiveDate,
) -> Result<(), HabitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(HabitError::EmptyName);
    }
    let weekdays = Weekdays::from_flags(days)?;
    if start > today {
        return Err(HabitError::FutureStart);
    }
    let changed = conn.execute(
        "UPDATE habit SET name = ?2, weekdays = ?3, start_date = ?4 WHERE id = ?1",
        (id.0, name, weekdays.0, start.to_string()),
    )?;
    if changed == 0 {
        return Err(HabitError::NotFound);
    }
    Ok(())
}

/// Hides a habit from the grid; its logs are kept.
pub fn archive(conn: &Connection, id: HabitId) -> Result<(), HabitError> {
    let changed = conn.execute("UPDATE habit SET archived = 1 WHERE id = ?1", [id.0])?;
    if changed == 0 {
        return Err(HabitError::NotFound);
    }
    Ok(())
}

/// Brings an archived habit back to the grid; its logs were kept, so marks and streaks return.
pub fn restore(conn: &Connection, id: HabitId) -> Result<(), HabitError> {
    let changed = conn.execute("UPDATE habit SET archived = 0 WHERE id = ?1", [id.0])?;
    if changed == 0 {
        return Err(HabitError::NotFound);
    }
    Ok(())
}

/// Archived habits as (id, name), by name ignoring case, then id.
pub fn archived(conn: &Connection) -> Result<Vec<(HabitId, String)>, HabitError> {
    let mut stmt = conn.prepare(
        "SELECT id, name FROM habit WHERE archived = 1 ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| Ok((HabitId(r.get(0)?), r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The month `delta` months from the one holding `shown`, as its first day. Never past the
/// month of `today`.
pub fn shift_month(
    shown: NaiveDate,
    delta: i32,
    today: NaiveDate,
) -> Result<NaiveDate, HabitError> {
    let first = calendar::month_start(shown);
    let step = Months::new(delta.unsigned_abs());
    let target = if delta >= 0 {
        first.checked_add_months(step)
    } else {
        first.checked_sub_months(step)
    }
    .ok_or(HabitError::OutOfRange)?;
    if target > calendar::month_start(today) {
        return Err(HabitError::FutureMonth);
    }
    Ok(target)
}

/// A habit that applies today and whether today is marked.
#[derive(Debug, PartialEq)]
pub struct DayHabit {
    pub id: HabitId,
    pub name: String,
    pub done: bool,
}

/// How many of `habits` are marked.
pub fn done_count(habits: &[DayHabit]) -> usize {
    habits.iter().filter(|h| h.done).count()
}

/// The active habits that apply on `today` (weekday selected, already started), oldest first.
pub fn today(conn: &Connection, today: NaiveDate) -> Result<Vec<DayHabit>, HabitError> {
    let mut stmt = conn.prepare(
        "SELECT h.id, h.name, EXISTS (SELECT 1 FROM habit_log l WHERE l.habit_id = h.id AND l.date = ?1)
         FROM habit h WHERE h.archived = 0 AND h.weekdays & ?2 != 0 AND h.start_date <= ?1
         ORDER BY h.id",
    )?;
    let rows = stmt.query_map((today.to_string(), weekday_bit(today)), |r| {
        Ok(DayHabit {
            id: HabitId(r.get(0)?),
            name: r.get(1)?,
            done: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Marks or unmarks `date`; only days the habit applies on, up to `today`.
pub fn set_done(
    conn: &Connection,
    id: HabitId,
    date: NaiveDate,
    today: NaiveDate,
    done: bool,
) -> Result<(), HabitError> {
    let tx = conn.unchecked_transaction()?;
    let (weekdays, start): (i64, String) = tx
        .query_row(
            "SELECT weekdays, start_date FROM habit WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(HabitError::NotFound)?;
    if date > today {
        return Err(HabitError::Future);
    }
    // ISO dates compare correctly as text.
    if date.to_string() < start {
        return Err(HabitError::BeforeStart);
    }
    if weekdays & weekday_bit(date) == 0 {
        return Err(HabitError::NotScheduled);
    }
    if done {
        tx.execute(
            "INSERT OR IGNORE INTO habit_log (habit_id, date) VALUES (?1, ?2)",
            (id.0, date.to_string()),
        )?;
    } else {
        tx.execute(
            "DELETE FROM habit_log WHERE habit_id = ?1 AND date = ?2",
            (id.0, date.to_string()),
        )?;
    }
    tx.commit()?;
    Ok(())
}
