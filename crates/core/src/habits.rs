// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Habits: create them and mark them done per day.

use std::collections::HashSet;
use std::fmt;

use chrono::{Datelike, Months, NaiveDate, TimeDelta};
use rusqlite::{Connection, OptionalExtension};

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

/// First day of the month that holds `date`.
pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("day 1 exists in every month")
}

/// The month `delta` months from the one holding `shown`, as its first day. Never past the
/// month of `today`.
pub fn shift_month(
    shown: NaiveDate,
    delta: i32,
    today: NaiveDate,
) -> Result<NaiveDate, HabitError> {
    let first = month_start(shown);
    let step = Months::new(delta.unsigned_abs());
    let target = if delta >= 0 {
        first.checked_add_months(step)
    } else {
        first.checked_sub_months(step)
    }
    .ok_or(HabitError::OutOfRange)?;
    if target > month_start(today) {
        return Err(HabitError::FutureMonth);
    }
    Ok(target)
}

/// Streaks and the share of the shown month done.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HabitStats {
    /// Consecutive applicable done days up to today; an undone today does not break it.
    pub current: u32,
    /// Longest run of consecutive applicable done days.
    pub best: u32,
    /// Done share of the shown month's applicable days up to today, floored.
    pub month_pct: u32,
}

/// Stats of one habit. Streaks are as of `today`; the percentage is for the month holding
/// `shown`. Only days on a selected weekday and from `start` count: other marks are ignored.
fn stats(
    weekdays: i64,
    start: NaiveDate,
    today: NaiveDate,
    shown: NaiveDate,
    done: &HashSet<NaiveDate>,
) -> HabitStats {
    // (date, done) of every applicable day from the start to today.
    let seq: Vec<(NaiveDate, bool)> = start
        .iter_days()
        .take_while(|d| *d <= today)
        .filter(|d| weekdays & weekday_bit(*d) != 0)
        .map(|d| (d, done.contains(&d)))
        .collect();
    let (mut best, mut run) = (0, 0);
    for &(_, ok) in &seq {
        run = if ok { run + 1 } else { 0 };
        best = best.max(run);
    }
    let mut current = 0;
    for (i, &(d, ok)) in seq.iter().rev().enumerate() {
        if ok {
            current += 1;
        } else if i > 0 || d != today {
            break;
        }
    }
    let in_month = seq
        .iter()
        .filter(|(d, _)| month_start(*d) == month_start(shown));
    let total = in_month.clone().count() as u32;
    let hits = in_month.filter(|(_, ok)| *ok).count() as u32;
    HabitStats {
        current,
        best,
        month_pct: (hits * 100).checked_div(total).unwrap_or(0),
    }
}

/// The month holding `shown`: every active habit with its state per day and its stats, from
/// one query over all their logs.
pub fn month(
    conn: &Connection,
    shown: NaiveDate,
    today: NaiveDate,
) -> Result<HabitMonth, HabitError> {
    let first = month_start(shown);
    let next = first
        .checked_add_months(Months::new(1))
        .ok_or(HabitError::OutOfRange)?;
    let len = (next - first).num_days() as u32;
    let mut stmt = conn.prepare(
        "SELECT h.id, h.name, h.weekdays, h.start_date, l.date          FROM habit h LEFT JOIN habit_log l ON l.habit_id = h.id          WHERE h.archived = 0 ORDER BY h.id",
    )?;
    // (id, name, weekdays, start, done dates) per habit.
    let mut raw: Vec<(HabitId, String, i64, NaiveDate, HashSet<NaiveDate>)> = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let id = HabitId(r.get(0)?);
        if raw.last().is_none_or(|h| h.0 != id) {
            let start = parse_date(r.get(3)?)?;
            raw.push((id, r.get(1)?, r.get(2)?, start, HashSet::new()));
        }
        if let (Some(date), Some(h)) = (r.get::<_, Option<String>>(4)?, raw.last_mut()) {
            h.4.insert(parse_date(date)?);
        }
    }
    let habits: Vec<MonthHabit> = raw
        .into_iter()
        .map(|(id, name, weekdays, start, done)| {
            let days = (0..len)
                .map(|i| {
                    let date = first + TimeDelta::days(i.into());
                    if weekdays & weekday_bit(date) == 0 || date < start {
                        DayState::Skipped
                    } else if date > today {
                        DayState::Future
                    } else if done.contains(&date) {
                        DayState::Done
                    } else {
                        DayState::Open
                    }
                })
                .collect();
            MonthHabit {
                id,
                name,
                days,
                stats: stats(weekdays, start, today, first, &done),
                weekdays: std::array::from_fn(|i| weekdays & (1 << i) != 0),
                start,
            }
        })
        .collect();
    let day_totals = day_totals(&habits, len as usize);
    let week_totals = day_totals
        .chunks(7)
        .map(|c| {
            c.iter().fold(Total::default(), |a, t| Total {
                done: a.done + t.done,
                applicable: a.applicable + t.applicable,
            })
        })
        .collect();
    let month_total = day_totals.iter().fold(Total::default(), |a, t| Total {
        done: a.done + t.done,
        applicable: a.applicable + t.applicable,
    });
    let heat = day_totals
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let date = first + TimeDelta::days(i as i64);
            match t.pct() {
                None => Heat::None,
                _ if date > today => Heat::Future,
                Some(0) => Heat::L0,
                Some(1..=25) => Heat::L1,
                Some(26..=50) => Heat::L2,
                Some(51..=75) => Heat::L3,
                Some(_) => Heat::L4,
            }
        })
        .collect();
    Ok(HabitMonth {
        lead: first.weekday().num_days_from_monday(),
        month_total,
        heat,
        first,
        days: len,
        today: (month_start(today) == first).then(|| today.day()),
        can_next: next <= today,
        habits,
        day_totals,
        week_totals,
    })
}

/// Per day: habits done and habits that apply (a done mark only counts on an applicable day).
fn day_totals(habits: &[MonthHabit], len: usize) -> Vec<Total> {
    (0..len)
        .map(|i| {
            habits
                .iter()
                .fold(Total::default(), |a, h| match h.days[i] {
                    DayState::Skipped => a,
                    DayState::Done => Total {
                        done: a.done + 1,
                        applicable: a.applicable + 1,
                    },
                    _ => Total {
                        applicable: a.applicable + 1,
                        ..a
                    },
                })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup(name: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("op-habits-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db::open(&dir.join("test.db")).unwrap();
        (dir, conn)
    }

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    // 2026-11-02 is a Monday.
    const MON: &str = "2026-11-02";

    /// State of every habit on `d`, seen on `today`.
    fn states(conn: &Connection, d: &str, today: &str) -> Vec<DayState> {
        let i = day(d).day0() as usize;
        month(conn, day(d), day(today))
            .unwrap()
            .habits
            .iter()
            .map(|h| h.days[i])
            .collect()
    }

    #[test]
    fn corrupt_stored_date_is_an_error() {
        let (dir, conn) = setup("baddate");
        create(&conn, "Read", [true; 7], day(MON), day(MON)).unwrap();
        conn.execute("UPDATE habit SET start_date = 'nope'", [])
            .unwrap();
        assert!(matches!(
            month(&conn, day(MON), day(MON)),
            Err(HabitError::BadDate(d)) if d == "nope"
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Names of the habits that apply on `d` (seen on `d`).
    fn names(conn: &Connection, d: &str) -> Vec<String> {
        let m = month(conn, day(d), day(d)).unwrap();
        let i = day(d).day0() as usize;
        m.habits
            .into_iter()
            .filter(|h| h.days[i] != DayState::Skipped)
            .map(|h| h.name)
            .collect()
    }

    #[test]
    fn create_trims_and_starts_today_on_every_weekday() {
        let (dir, conn) = setup("create");
        let id = create(&conn, "  Read \n", [true; 7], day(MON), day(MON)).unwrap();
        let (weekdays, start): (i64, String) = conn
            .query_row("SELECT weekdays, start_date FROM habit", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((weekdays, start.as_str()), (127, MON));
        let m = month(&conn, day(MON), day(MON)).unwrap();
        assert_eq!((m.habits[0].id, m.habits[0].name.as_str()), (id, "Read"));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_name_is_rejected() {
        let (dir, conn) = setup("empty");
        assert!(matches!(
            create(&conn, " \t", [true; 7], day(MON), day(MON)),
            Err(HabitError::EmptyName)
        ));
        assert!(names(&conn, MON).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn weekdays_from_flags_is_monday_first_and_rejects_empty() {
        let w = Weekdays::from_flags([true, false, false, false, false, false, true]).unwrap();
        assert_eq!(w, Weekdays(0b100_0001));
        assert_eq!(Weekdays::from_flags([true; 7]).unwrap(), Weekdays::ALL);
        assert!(matches!(
            Weekdays::from_flags([false; 7]),
            Err(HabitError::NoWeekdays)
        ));
    }

    #[test]
    fn create_validates_name_then_weekdays_then_start() {
        let (dir, conn) = setup("validate");
        let tomorrow = day("2026-11-03");
        assert!(matches!(
            create(&conn, " ", [false; 7], tomorrow, day(MON)),
            Err(HabitError::EmptyName)
        ));
        assert!(matches!(
            create(&conn, "A", [false; 7], tomorrow, day(MON)),
            Err(HabitError::NoWeekdays)
        ));
        assert!(matches!(
            create(&conn, "A", [true; 7], tomorrow, day(MON)),
            Err(HabitError::FutureStart)
        ));
        assert!(names(&conn, MON).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selected_weekdays_and_start_decide_when_it_shows() {
        let (dir, conn) = setup("schedule");
        // Mon, Wed, Fri, started on Monday the 2nd; created on Tuesday the 3rd.
        let mwf = [true, false, true, false, true, false, false];
        create(&conn, "Gym", mwf, day(MON), day("2026-11-03")).unwrap();
        assert_eq!(names(&conn, MON), ["Gym"]);
        assert!(names(&conn, "2026-11-03").is_empty());
        assert_eq!(names(&conn, "2026-11-04"), ["Gym"]);
        // Started yesterday: shows today, not the day before yesterday.
        create(
            &conn,
            "Walk",
            [true; 7],
            day("2026-11-05"),
            day("2026-11-06"),
        )
        .unwrap();
        assert_eq!(names(&conn, "2026-11-06"), ["Gym", "Walk"]);
        assert_eq!(names(&conn, "2026-11-04"), ["Gym"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn month_orders_by_creation_and_hides_not_started_and_unscheduled() {
        let (dir, conn) = setup("order");
        create(&conn, "B", [true; 7], day(MON), day(MON)).unwrap();
        create(&conn, "A", [true; 7], day(MON), day(MON)).unwrap();
        create(
            &conn,
            "Later",
            [true; 7],
            day("2026-11-05"),
            day("2026-11-05"),
        )
        .unwrap();
        // Only Monday and Tuesday.
        conn.execute(
            "INSERT INTO habit (name, weekdays, start_date) VALUES ('Mon-Tue', 3, ?1)",
            [MON],
        )
        .unwrap();
        assert_eq!(names(&conn, MON), ["B", "A", "Mon-Tue"]);
        assert_eq!(names(&conn, "2026-11-04"), ["B", "A"]);
        assert_eq!(names(&conn, "2026-11-05"), ["B", "A", "Later"]);
        // Before every start date.
        assert!(names(&conn, "2026-11-01").is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archived_habits_are_hidden() {
        let (dir, conn) = setup("archived");
        create(&conn, "Gone", [true; 7], day(MON), day(MON)).unwrap();
        conn.execute("UPDATE habit SET archived = 1", []).unwrap();
        assert!(names(&conn, MON).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_marks_and_unmarks_and_is_idempotent() {
        let (dir, conn) = setup("toggle");
        let id = create(&conn, "Read", [true; 7], day(MON), day(MON)).unwrap();
        let done = |c: &Connection| states(c, MON, MON)[0] == DayState::Done;
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        assert!(done(&conn));
        // Another day stays untouched.
        assert_eq!(states(&conn, "2026-11-03", "2026-11-03"), [DayState::Open]);
        set_done(&conn, id, day(MON), day(MON), false).unwrap();
        set_done(&conn, id, day(MON), day(MON), false).unwrap();
        assert!(!done(&conn));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_rejects_future_before_start_and_unscheduled_days() {
        let (dir, conn) = setup("rules");
        let id = create(&conn, "Read", [true; 7], day(MON), day(MON)).unwrap();
        let tue = day("2026-11-03");
        assert!(matches!(
            set_done(&conn, id, tue, day(MON), true),
            Err(HabitError::Future)
        ));
        assert!(matches!(
            set_done(&conn, id, day("2026-11-01"), day(MON), true),
            Err(HabitError::BeforeStart)
        ));
        conn.execute("UPDATE habit SET weekdays = 1", []).unwrap();
        assert!(matches!(
            set_done(&conn, id, tue, tue, true),
            Err(HabitError::NotScheduled)
        ));
        let logged: i64 = conn
            .query_row("SELECT count(*) FROM habit_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_on_unknown_habit_is_not_found() {
        let (dir, conn) = setup("notfound");
        assert!(matches!(
            set_done(&conn, HabitId(99), day(MON), day(MON), true),
            Err(HabitError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn month_classifies_every_day() {
        let (dir, conn) = setup("month");
        // Mon, Wed, Fri, started Wed the 4th; today is Fri the 6th.
        let mwf = [true, false, true, false, true, false, false];
        let id = create(&conn, "Gym", mwf, day("2026-11-04"), day("2026-11-04")).unwrap();
        set_done(&conn, id, day("2026-11-04"), day("2026-11-06"), true).unwrap();
        let today = day("2026-11-06");
        let m = month(&conn, day("2026-11-17"), today).unwrap();
        assert_eq!((m.first, m.days), (day("2026-11-01"), 30));
        let d = &m.habits[0].days;
        assert_eq!(d.len(), 30);
        use DayState::*;
        // Sun/Tue/Thu/Sat not scheduled, Mon 2nd before the start, Wed done, Fri open,
        // Mon 9th scheduled but in the future.
        assert_eq!(
            d[..9],
            [
                Skipped, Skipped, Skipped, Done, Skipped, Open, Skipped, Skipped, Future
            ]
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_mark_on_a_day_that_does_not_apply_is_ignored() {
        let (dir, conn) = setup("ignored");
        let id = create(&conn, "Read", [true; 7], day(MON), day(MON)).unwrap();
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        // Schedule changes later: Monday no longer applies, the old mark must not show.
        conn.execute("UPDATE habit SET weekdays = 2", []).unwrap();
        assert_eq!(states(&conn, MON, MON), [DayState::Skipped]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn month_shows_only_its_own_marks_and_active_habits() {
        let (dir, conn) = setup("scope");
        let a = create(&conn, "A", [true; 7], day("2026-10-01"), day(MON)).unwrap();
        create(&conn, "B", [true; 7], day("2026-10-01"), day(MON)).unwrap();
        create(&conn, "Gone", [true; 7], day("2026-10-01"), day(MON)).unwrap();
        conn.execute("UPDATE habit SET archived = 1 WHERE name = 'Gone'", [])
            .unwrap();
        set_done(&conn, a, day("2026-10-31"), day(MON), true).unwrap();
        set_done(&conn, a, day("2026-11-02"), day(MON), true).unwrap();
        let nov = month(&conn, day(MON), day(MON)).unwrap();
        let oct = month(&conn, day("2026-10-15"), day(MON)).unwrap();
        assert_eq!(nov.habits.len(), 2);
        let done = |m: &HabitMonth, h: usize| {
            m.habits[h]
                .days
                .iter()
                .filter(|d| **d == DayState::Done)
                .count()
        };
        assert_eq!((done(&nov, 0), done(&nov, 1)), (1, 0));
        assert_eq!((done(&oct, 0), done(&oct, 1)), (1, 0));
        assert_eq!(oct.habits[0].days.len(), 31);
        assert_eq!(oct.habits[0].days[30], DayState::Done);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn month_reports_today_and_whether_next_is_allowed() {
        let (dir, conn) = setup("nav");
        let m = month(&conn, day(MON), day(MON)).unwrap();
        assert_eq!((m.today, m.can_next), (Some(2), false));
        let m = month(&conn, day("2026-10-20"), day(MON)).unwrap();
        assert_eq!((m.today, m.can_next), (None, true));
        assert!(
            month(&conn, day("2026-10-20"), day(MON))
                .unwrap()
                .habits
                .is_empty()
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shift_month_crosses_years_and_stops_at_the_current_month() {
        let today = day("2026-11-07");
        assert_eq!(
            shift_month(day("2026-11-01"), -1, today).unwrap(),
            day("2026-10-01")
        );
        assert_eq!(
            shift_month(day("2026-01-20"), -1, today).unwrap(),
            day("2025-12-01")
        );
        assert_eq!(
            shift_month(day("2026-10-01"), 1, today).unwrap(),
            day("2026-11-01")
        );
        assert!(matches!(
            shift_month(day("2026-11-01"), 1, today),
            Err(HabitError::FutureMonth)
        ));
        assert!(matches!(
            shift_month(NaiveDate::MIN, -1, today),
            Err(HabitError::OutOfRange)
        ));
        assert_eq!(month_start(day("2026-11-30")), day("2026-11-01"));
    }

    fn st(wd: i64, start: &str, today: &str, shown: &str, done: &[&str]) -> (u32, u32, u32) {
        let done = done.iter().map(|d| day(d)).collect();
        let s = stats(wd, day(start), day(today), day(shown), &done);
        (s.current, s.best, s.month_pct)
    }

    #[test]
    fn streaks_count_back_from_today_and_undone_today_does_not_break() {
        let done = ["2026-11-03", "2026-11-04", "2026-11-05"];
        let s = |today| st(127, "2026-11-02", today, today, &done);
        assert_eq!(s("2026-11-05"), (3, 3, 75));
        // Today (the 6th) not done yet: the run up to yesterday stays.
        assert_eq!(s("2026-11-06"), (3, 3, 60));
        // The undone 6th is now an earlier day: it breaks the streak.
        assert_eq!(s("2026-11-07"), (0, 3, 50));
    }

    #[test]
    fn non_applicable_days_neither_cut_nor_add() {
        // Mon, Wed, Fri done; Tue is marked but does not apply.
        let wd = 0b001_0101;
        let done = ["2026-11-02", "2026-11-03", "2026-11-04", "2026-11-06"];
        assert_eq!(
            st(wd, "2026-11-02", "2026-11-06", "2026-11-06", &done),
            (3, 3, 100)
        );
        // Today is a Saturday: still a streak of 3.
        assert_eq!(
            st(wd, "2026-11-02", "2026-11-07", "2026-11-07", &done),
            (3, 3, 100)
        );
    }

    #[test]
    fn marks_before_start_and_no_applicable_days_give_zero() {
        assert_eq!(
            st(
                127,
                "2026-11-05",
                "2026-11-06",
                "2026-11-06",
                &["2026-11-01"]
            ),
            (0, 0, 0)
        );
        // Only Sundays, started Monday, today Wednesday: nothing applies yet.
        assert_eq!(
            st(0b100_0000, "2026-11-02", "2026-11-04", "2026-11-04", &[]),
            (0, 0, 0)
        );
    }

    #[test]
    fn percentage_is_floored_from_start_and_streaks_ignore_the_shown_month() {
        // Started mid-month (the 4th), seen on the 6th: 3 applicable days, 1 done.
        assert_eq!(
            st(
                127,
                "2026-11-04",
                "2026-11-06",
                "2026-11-06",
                &["2026-11-05"]
            ),
            (1, 1, 33)
        );
        // Shown October (past): the whole month counts; the streak is as of today.
        let done = ["2026-10-30", "2026-10-31", "2026-11-01"];
        assert_eq!(
            st(127, "2026-10-01", "2026-11-01", "2026-10-15", &done),
            (3, 3, 6)
        );
    }

    #[test]
    fn month_carries_stats_from_all_logs() {
        let (dir, conn) = setup("stats");
        let a = create(&conn, "A", [true; 7], day("2026-10-01"), day(MON)).unwrap();
        create(&conn, "B", [true; 7], day("2026-10-01"), day(MON)).unwrap();
        for d in ["2026-10-31", "2026-11-01", MON] {
            set_done(&conn, a, day(d), day(MON), true).unwrap();
        }
        let m = month(&conn, day(MON), day(MON)).unwrap();
        let full = HabitStats {
            current: 3,
            best: 3,
            month_pct: 100,
        };
        assert_eq!(m.habits[0].stats, full);
        assert_eq!(m.habits[1].stats.current, 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_validates_like_create_and_needs_an_existing_habit() {
        let (dir, conn) = setup("update-rules");
        let id = create(&conn, "A", [true; 7], day(MON), day(MON)).unwrap();
        let up = |n, d, s, i| update(&conn, i, n, d, day(s), day(MON));
        let tue = "2026-11-03";
        assert!(matches!(
            up(" ", [false; 7], tue, id),
            Err(HabitError::EmptyName)
        ));
        assert!(matches!(
            up("B", [false; 7], tue, id),
            Err(HabitError::NoWeekdays)
        ));
        assert!(matches!(
            up("B", [true; 7], tue, id),
            Err(HabitError::FutureStart)
        ));
        assert!(matches!(
            up("B", [true; 7], MON, HabitId(99)),
            Err(HabitError::NotFound)
        ));
        assert_eq!(names(&conn, MON), ["A"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_is_retroactive_and_keeps_marks() {
        let (dir, conn) = setup("update-retro");
        let id = create(&conn, "A", [true; 7], day(MON), day(MON)).unwrap();
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        // Tuesday only: Monday's mark is ignored, not deleted.
        let tue_only = [false, true, false, false, false, false, false];
        update(&conn, id, " B ", tue_only, day(MON), day(MON)).unwrap();
        assert_eq!(states(&conn, MON, MON), [DayState::Skipped]);
        let m = month(&conn, day(MON), day(MON)).unwrap();
        let h = &m.habits[0];
        assert_eq!(
            (h.name.as_str(), h.weekdays, h.start),
            ("B", tue_only, day(MON))
        );
        update(&conn, id, "B", [true; 7], day("2026-11-01"), day(MON)).unwrap();
        assert_eq!(states(&conn, MON, MON), [DayState::Done]);
        // The new start applies to the past too.
        assert_eq!(states(&conn, "2026-11-01", MON), [DayState::Open]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_hides_keeps_logs_and_needs_an_existing_habit() {
        let (dir, conn) = setup("archive");
        let id = create(&conn, "A", [true; 7], day(MON), day(MON)).unwrap();
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        archive(&conn, id).unwrap();
        assert!(names(&conn, MON).is_empty());
        let logged: i64 = conn
            .query_row("SELECT count(*) FROM habit_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 1);
        assert!(matches!(
            archive(&conn, HabitId(99)),
            Err(HabitError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_works_on_any_past_applicable_day() {
        let (dir, conn) = setup("past");
        let id = create(&conn, "Read", [true; 7], day("2026-10-01"), day(MON)).unwrap();
        set_done(&conn, id, day("2026-10-05"), day(MON), true).unwrap();
        assert_eq!(states(&conn, "2026-10-05", MON), [DayState::Done]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn totals_per_day_and_week_count_future_and_ignore_skipped() {
        let (dir, conn) = setup("totals");
        let today = day("2026-11-03");
        // A: every day since the 1st; B: Mondays only (the 2nd, 9th, ...), started the 2nd.
        let a = create(&conn, "A", [true; 7], day("2026-11-01"), today).unwrap();
        let mon = [true, false, false, false, false, false, false];
        let b = create(&conn, "B", mon, day(MON), today).unwrap();
        set_done(&conn, a, day("2026-11-01"), today, true).unwrap();
        set_done(&conn, a, day(MON), today, true).unwrap();
        set_done(&conn, b, day(MON), today, true).unwrap();
        // A stray mark on a Tuesday B does not apply on.
        conn.execute(
            "INSERT INTO habit_log (habit_id, date) VALUES (?1, '2026-11-03')",
            [b.0],
        )
        .unwrap();
        let m = month(&conn, today, today).unwrap();
        let t = |done, applicable| Total { done, applicable };
        // Day 1: A only. Day 2: both done. Day 3: B skipped, A open. Day 9 (future Monday): both.
        assert_eq!(m.day_totals[..3], [t(1, 1), t(2, 2), t(0, 1)]);
        assert_eq!(m.day_totals[8], t(0, 2));
        assert_eq!(m.day_totals.len(), 30);
        // Block 1: 7 A days + one B day; block 5 has days 29-30 (A twice, B on Monday the 30th).
        assert_eq!(m.week_totals.len(), 5);
        assert_eq!(m.week_totals[0], t(3, 8));
        assert_eq!(m.week_totals[4], t(0, 3));
        assert_eq!(t(3, 8).pct(), Some(37));
        assert_eq!(t(0, 0).pct(), None);
        // Month total is the sum of the days; 2026-11-01 is a Sunday, so six blanks lead.
        assert_eq!(m.month_total.done, 3);
        assert_eq!(m.lead, 6);
        use Heat::{Future, L0, L4};
        // Day 1: 1/1 done; day 2: 2/2; day 3 (today): 0/1; day 4+: future.
        assert_eq!(m.heat[..4], [L4, L4, L0, Future]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn heat_buckets_by_share_done() {
        let (dir, conn) = setup("heat");
        let today = day("2026-11-30");
        let ids: Vec<_> = (0..4)
            .map(|i| create(&conn, &format!("h{i}"), [true; 7], day("2026-11-01"), today).unwrap())
            .collect();
        // Done counts per day 1..=5 of 4 habits: 0, 1 (25%), 2 (50%), 3 (75%), 4 (100%).
        for (d, n) in [(2, 1), (3, 2), (4, 3), (5, 4)] {
            for id in &ids[..n] {
                set_done(&conn, *id, day(&format!("2026-11-0{d}")), today, true).unwrap();
            }
        }
        let m = month(&conn, today, today).unwrap();
        use Heat::{L0, L1, L2, L3, L4};
        assert_eq!(m.heat[..5], [L0, L1, L2, L3, L4]);
        // Past month with no habit started: nothing applies.
        let m = month(&conn, day("2026-10-10"), today).unwrap();
        assert!(m.heat.iter().all(|h| *h == Heat::None));
        assert_eq!(m.lead, 3);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_brings_back_marks_and_streaks() {
        let (dir, conn) = setup("restore");
        let b = create(&conn, "b", [true; 7], day(MON), day(MON)).unwrap();
        let a = create(&conn, "A", [true; 7], day(MON), day(MON)).unwrap();
        set_done(&conn, a, day(MON), day(MON), true).unwrap();
        archive(&conn, b).unwrap();
        archive(&conn, a).unwrap();
        let list = archived(&conn).unwrap();
        assert_eq!(list, [(a, "A".to_string()), (b, "b".to_string())]);
        restore(&conn, a).unwrap();
        assert_eq!(archived(&conn).unwrap(), [(b, "b".to_string())]);
        let m = month(&conn, day(MON), day(MON)).unwrap();
        assert_eq!(m.habits.len(), 1);
        assert_eq!(m.habits[0].stats.current, 1);
        assert!(m.habits[0].days.contains(&DayState::Done));
        assert!(matches!(
            restore(&conn, HabitId(99)),
            Err(HabitError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
