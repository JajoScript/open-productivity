// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Minutes studied per day over a week or a month.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;

use super::StudyError;
use crate::calendar::{month_start, shift, week_start};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DaysRange {
    Week,
    Month,
}

#[derive(Debug, PartialEq)]
pub struct DayBar {
    pub date: NaiveDate,
    pub minutes: i64,
    /// Minutes relative to the busiest day of the range; 0 when nothing was studied.
    pub height: f32,
    pub today: bool,
    /// Whether the axis names this day.
    pub labeled: bool,
}

#[derive(Debug, PartialEq)]
pub struct DayChart {
    pub days: Vec<DayBar>,
    pub total_minutes: i64,
}

/// The week (Monday to Sunday) or month that holds `today`, one bar per day, future days at 0.
/// Archived subjects count.
pub fn day_minutes(
    conn: &Connection,
    today: NaiveDate,
    range: DaysRange,
) -> Result<DayChart, StudyError> {
    let (first, last) = match range {
        DaysRange::Week => {
            let first = week_start(today);
            (first, shift(first, 6))
        }
        DaysRange::Month => {
            let first = month_start(today);
            (
                first,
                shift(month_start(shift(first, 31).unwrap_or(first)), -1),
            )
        }
    };
    // Only the end of the supported date range has no last day: nothing to chart there.
    let Some(last) = last else {
        return Ok(DayChart {
            days: Vec::new(),
            total_minutes: 0,
        });
    };
    let mut stmt = conn.prepare(
        "SELECT date, SUM(focus_minutes) FROM study_session
         WHERE date BETWEEN ?1 AND ?2 GROUP BY date",
    )?;
    let rows = stmt.query_map((first.to_string(), last.to_string()), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut by_date = HashMap::new();
    for row in rows {
        let (date, minutes) = row?;
        let date: NaiveDate = date.parse().map_err(|_| StudyError::BadDate(date))?;
        by_date.insert(date, minutes);
    }
    let mut days: Vec<DayBar> = first
        .iter_days()
        .take_while(|d| *d <= last)
        .map(|date| DayBar {
            date,
            minutes: by_date.get(&date).copied().unwrap_or(0),
            height: 0.0,
            today: date == today,
            labeled: match range {
                DaysRange::Week => true,
                DaysRange::Month => [1, 5, 10, 15, 20, 25].contains(&date.day()) || date == last,
            },
        })
        .collect();
    let max = days.iter().map(|d| d.minutes).max().unwrap_or(0);
    if max > 0 {
        for d in &mut days {
            d.height = d.minutes as f32 / max as f32;
        }
    }
    let total_minutes = days.iter().map(|d| d.minutes).sum();
    Ok(DayChart {
        days,
        total_minutes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::study::{SubjectId, create_subject, log_manual_session, set_subject_archived};

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn log(conn: &Connection, s: SubjectId, date: &str, minutes: i64) {
        // `today` is far ahead so any test date is allowed.
        log_manual_session(conn, s, day(date), minutes, "", None, day("2030-01-01")).unwrap();
    }

    fn setup() -> (Connection, SubjectId) {
        let conn = db::test_conn();
        let s = create_subject(&conn, "Math").unwrap();
        (conn, s)
    }

    #[test]
    fn week_runs_monday_to_sunday_with_the_future_at_zero() {
        let (conn, s) = setup();
        // 2026-11-11 is a Wednesday.
        log(&conn, s, "2026-11-09", 30);
        log(&conn, s, "2026-11-12", 60);
        let c = day_minutes(&conn, day("2026-11-11"), DaysRange::Week).unwrap();
        assert_eq!(c.days.first().unwrap().date, day("2026-11-09"));
        assert_eq!(c.days.last().unwrap().date, day("2026-11-15"));
        assert_eq!(c.days.len(), 7);
        assert!(c.days.iter().all(|d| d.labeled));
        assert_eq!(
            c.days.iter().map(|d| d.minutes).collect::<Vec<_>>(),
            [30, 0, 0, 60, 0, 0, 0]
        );
    }

    #[test]
    fn month_has_its_calendar_length() {
        let (conn, _) = setup();
        for (today, len) in [
            ("2026-02-10", 28),
            ("2028-02-10", 29),
            ("2026-04-10", 30),
            ("2026-12-31", 31),
        ] {
            let c = day_minutes(&conn, day(today), DaysRange::Month).unwrap();
            assert_eq!(c.days.len(), len, "{today}");
            assert_eq!(c.days[0].date.day(), 1);
        }
    }

    #[test]
    fn month_leaves_out_other_months() {
        let (conn, s) = setup();
        log(&conn, s, "2026-10-31", 10);
        log(&conn, s, "2026-11-01", 20);
        log(&conn, s, "2026-11-30", 30);
        log(&conn, s, "2026-12-01", 40);
        let c = day_minutes(&conn, day("2026-11-10"), DaysRange::Month).unwrap();
        assert_eq!(c.total_minutes, 50);
    }

    #[test]
    fn sessions_of_a_day_add_up_even_when_archived() {
        let (conn, s) = setup();
        let other = create_subject(&conn, "Art").unwrap();
        log(&conn, s, "2026-11-10", 20);
        log(&conn, other, "2026-11-10", 25);
        set_subject_archived(&conn, other, true).unwrap();
        let c = day_minutes(&conn, day("2026-11-10"), DaysRange::Week).unwrap();
        assert_eq!(c.days[1].minutes, 45);
        assert_eq!(c.total_minutes, 45);
    }

    #[test]
    fn height_is_relative_to_the_busiest_day_and_zero_when_empty() {
        let (conn, s) = setup();
        let c = day_minutes(&conn, day("2026-11-10"), DaysRange::Week).unwrap();
        assert!(c.days.iter().all(|d| d.height == 0.0));
        log(&conn, s, "2026-11-09", 30);
        log(&conn, s, "2026-11-10", 60);
        let c = day_minutes(&conn, day("2026-11-10"), DaysRange::Week).unwrap();
        assert_eq!(
            (c.days[0].height, c.days[1].height, c.days[2].height),
            (0.5, 1.0, 0.0)
        );
    }

    #[test]
    fn only_today_is_marked() {
        let (conn, _) = setup();
        let c = day_minutes(&conn, day("2026-11-10"), DaysRange::Month).unwrap();
        let marked: Vec<_> = c.days.iter().filter(|d| d.today).map(|d| d.date).collect();
        assert_eq!(marked, [day("2026-11-10")]);
    }

    #[test]
    fn month_labels_five_day_steps_and_the_last_day() {
        let (conn, _) = setup();
        let c = day_minutes(&conn, day("2026-11-10"), DaysRange::Month).unwrap();
        let labeled: Vec<_> = c
            .days
            .iter()
            .filter(|d| d.labeled)
            .map(|d| d.date.day())
            .collect();
        assert_eq!(labeled, [1, 5, 10, 15, 20, 25, 30]);
    }

    #[test]
    fn unreadable_stored_date_is_an_error() {
        let (conn, s) = setup();
        // The table checks dates; a damaged file would not.
        conn.execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        conn.execute(
            "INSERT INTO study_session (subject_id, date, focus_minutes, description)
             VALUES (?1, '2026-11-1x', 5, '')",
            [s.0],
        )
        .unwrap();
        assert!(matches!(
            day_minutes(&conn, day("2026-11-10"), DaysRange::Month),
            Err(StudyError::BadDate(_))
        ));
    }
}
