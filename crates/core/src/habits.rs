// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Habits: create them and mark them done per day.

use std::fmt;

use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, OptionalExtension};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HabitId(pub i64);

/// A habit as it appears on one day.
#[derive(Debug, PartialEq)]
pub struct DayHabit {
    pub id: HabitId,
    pub name: String,
    pub done: bool,
}

#[derive(Debug)]
pub enum HabitError {
    EmptyName,
    NotFound,
    /// The day is after today.
    Future,
    /// The day is before the habit started.
    BeforeStart,
    /// The habit is not scheduled on that weekday.
    NotScheduled,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for HabitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HabitError::EmptyName => write!(f, "the name is empty"),
            HabitError::NotFound => write!(f, "habit not found"),
            HabitError::Future => write!(f, "that day has not happened yet"),
            HabitError::BeforeStart => write!(f, "the habit had not started on that day"),
            HabitError::NotScheduled => write!(f, "the habit does not apply on that weekday"),
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

/// Starts today and applies every day of the week.
pub fn create(conn: &Connection, name: &str, today: NaiveDate) -> Result<HabitId, HabitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(HabitError::EmptyName);
    }
    conn.execute(
        "INSERT INTO habit (name, start_date) VALUES (?1, ?2)",
        (name, today.to_string()),
    )?;
    Ok(HabitId(conn.last_insert_rowid()))
}

/// The active habits that apply on `date`, oldest first.
pub fn for_day(conn: &Connection, date: NaiveDate) -> Result<Vec<DayHabit>, HabitError> {
    let mut stmt = conn.prepare(
        "SELECT h.id, h.name, l.habit_id IS NOT NULL \
         FROM habit h LEFT JOIN habit_log l ON l.habit_id = h.id AND l.date = ?1 \
         WHERE h.archived = 0 AND h.weekdays & ?2 != 0 AND h.start_date <= ?1 \
         ORDER BY h.id",
    )?;
    let habits = stmt
        .query_map((date.to_string(), weekday_bit(date)), |r| {
            Ok(DayHabit {
                id: HabitId(r.get(0)?),
                name: r.get(1)?,
                done: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(habits)
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

    fn names(conn: &Connection, d: &str) -> Vec<String> {
        for_day(conn, day(d))
            .unwrap()
            .into_iter()
            .map(|h| h.name)
            .collect()
    }

    #[test]
    fn create_trims_and_starts_today_on_every_weekday() {
        let (dir, conn) = setup("create");
        let id = create(&conn, "  Read \n", day(MON)).unwrap();
        let (weekdays, start): (i64, String) = conn
            .query_row("SELECT weekdays, start_date FROM habit", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((weekdays, start.as_str()), (127, MON));
        assert_eq!(
            for_day(&conn, day("2026-11-08")).unwrap(),
            [DayHabit {
                id,
                name: "Read".into(),
                done: false
            }]
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_name_is_rejected() {
        let (dir, conn) = setup("empty");
        assert!(matches!(
            create(&conn, " \t", day(MON)),
            Err(HabitError::EmptyName)
        ));
        assert!(names(&conn, MON).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn for_day_orders_by_creation_and_hides_not_started_and_unscheduled() {
        let (dir, conn) = setup("forday");
        create(&conn, "B", day(MON)).unwrap();
        create(&conn, "A", day(MON)).unwrap();
        create(&conn, "Later", day("2026-11-05")).unwrap();
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
        create(&conn, "Gone", day(MON)).unwrap();
        conn.execute("UPDATE habit SET archived = 1", []).unwrap();
        assert!(names(&conn, MON).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_marks_and_unmarks_and_is_idempotent() {
        let (dir, conn) = setup("toggle");
        let id = create(&conn, "Read", day(MON)).unwrap();
        let done = |c: &Connection| for_day(c, day(MON)).unwrap()[0].done;
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        set_done(&conn, id, day(MON), day(MON), true).unwrap();
        assert!(done(&conn));
        // Another day stays untouched.
        assert!(!for_day(&conn, day("2026-11-03")).unwrap()[0].done);
        set_done(&conn, id, day(MON), day(MON), false).unwrap();
        set_done(&conn, id, day(MON), day(MON), false).unwrap();
        assert!(!done(&conn));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_done_rejects_future_before_start_and_unscheduled_days() {
        let (dir, conn) = setup("rules");
        let id = create(&conn, "Read", day(MON)).unwrap();
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
}
