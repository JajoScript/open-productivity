// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Cumulative hours studied, one point per week.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::StudyError;
use crate::calendar::{shift, week_start};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoursRange {
    Months3,
    Months6,
    Year,
    All,
}

#[derive(Debug, PartialEq)]
pub struct HoursChart {
    /// Minutes accumulated since the start of the range, one point per week, oldest first.
    pub points: Vec<i64>,
    /// Each point relative to the final total; all 0 when nothing was studied.
    pub heights: Vec<f32>,
    pub total_minutes: i64,
    pub current_week_minutes: i64,
    /// Monday of the first week.
    pub start: NaiveDate,
    pub weeks: usize,
}

fn parse(date: String) -> Result<NaiveDate, StudyError> {
    date.parse().map_err(|_| StudyError::BadDate(date))
}

/// Weeks (Monday to Sunday) up to the one that holds `today`, with the minutes studied
/// so far at the end of each. Archived subjects count.
pub fn hours_chart(
    conn: &Connection,
    today: NaiveDate,
    range: HoursRange,
) -> Result<HoursChart, StudyError> {
    let this_week = week_start(today);
    let weeks = match range {
        HoursRange::Months3 => 13,
        HoursRange::Months6 => 26,
        HoursRange::Year => 52,
        HoursRange::All => {
            let oldest: Option<String> =
                conn.query_row("SELECT MIN(date) FROM study_session", [], |r| r.get(0))?;
            match oldest {
                Some(d) => ((this_week - week_start(parse(d)?)).num_days() / 7 + 1).max(1) as usize,
                None => 1,
            }
        }
    };
    let start = shift(this_week, -7 * (weeks as i64 - 1)).unwrap_or(this_week);
    let end = shift(this_week, 6).unwrap_or(this_week);
    // The Monday of each date's week; NULL when the stored date is unreadable.
    let mut stmt = conn.prepare(
        "SELECT date(date, '-' || ((CAST(strftime('%w', date) AS INTEGER) + 6) % 7) || ' days'),
                SUM(focus_minutes), MIN(date)
         FROM study_session WHERE date BETWEEN ?1 AND ?2 GROUP BY 1",
    )?;
    let rows = stmt.query_map((start.to_string(), end.to_string()), |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut per_week = vec![0i64; weeks];
    for row in rows {
        let (monday, minutes, sample) = row?;
        let monday = parse(monday.ok_or_else(|| StudyError::BadDate(sample.clone()))?)?;
        let i = (monday - start).num_days() / 7;
        if let Some(slot) = usize::try_from(i).ok().and_then(|i| per_week.get_mut(i)) {
            *slot += minutes;
        }
    }
    let current_week_minutes = per_week[weeks - 1];
    let mut acc = 0;
    let points: Vec<i64> = per_week
        .iter()
        .map(|m| {
            acc += m;
            acc
        })
        .collect();
    let heights = points
        .iter()
        .map(|&p| if acc > 0 { p as f32 / acc as f32 } else { 0.0 })
        .collect();
    Ok(HoursChart {
        points,
        heights,
        total_minutes: acc,
        current_week_minutes,
        start,
        weeks,
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
        log_manual_session(conn, s, day(date), minutes, "", None, day("2030-01-01")).unwrap();
    }

    fn setup() -> (Connection, SubjectId) {
        let conn = db::test_conn();
        let s = create_subject(&conn, "A").unwrap();
        (conn, s)
    }

    #[test]
    fn weeks_run_monday_to_sunday() {
        let (conn, s) = setup();
        log(&conn, s, "2026-10-25", 30); // Sunday
        log(&conn, s, "2026-10-26", 60); // next Monday
        let c = hours_chart(&conn, day("2026-10-29"), HoursRange::Months3).unwrap();
        assert_eq!(c.weeks, 13);
        assert_eq!(c.start, day("2026-08-03"));
        assert_eq!(&c.points[10..], [0, 30, 90]);
        assert_eq!((c.total_minutes, c.current_week_minutes), (90, 60));
    }

    #[test]
    fn ranges_have_fixed_lengths_and_ignore_older_sessions() {
        let (conn, s) = setup();
        log(&conn, s, "2025-01-01", 500);
        log(&conn, s, "2026-10-20", 10);
        let t = day("2026-10-29");
        let c = |r| hours_chart(&conn, t, r).unwrap();
        assert_eq!(c(HoursRange::Months6).weeks, 26);
        assert_eq!(c(HoursRange::Months6).points.len(), 26);
        assert_eq!(c(HoursRange::Year).weeks, 52);
        assert_eq!(c(HoursRange::Year).total_minutes, 10);
        assert_eq!(c(HoursRange::All).total_minutes, 510);
    }

    #[test]
    fn all_starts_at_the_oldest_session_week_including_archived() {
        let (conn, s) = setup();
        log(&conn, s, "2026-10-07", 45); // Wednesday
        set_subject_archived(&conn, s, true).unwrap();
        let c = hours_chart(&conn, day("2026-10-29"), HoursRange::All).unwrap();
        assert_eq!((c.weeks, c.start), (4, day("2026-10-05")));
        assert_eq!(c.points, [45, 45, 45, 45]);
        assert_eq!(c.current_week_minutes, 0);
    }

    #[test]
    fn no_sessions_gives_a_flat_line() {
        let (conn, _) = setup();
        let c = hours_chart(&conn, day("2026-10-29"), HoursRange::All).unwrap();
        assert_eq!((c.weeks, c.points, c.total_minutes), (1, vec![0], 0));
        assert_eq!(c.start, day("2026-10-26"));
        let c = hours_chart(&conn, day("2026-10-29"), HoursRange::Months3).unwrap();
        assert!(c.points.iter().all(|&p| p == 0));
        assert!(c.heights.iter().all(|&h| h == 0.0));
    }

    #[test]
    fn heights_are_relative_to_the_final_total() {
        let (conn, s) = setup();
        log(&conn, s, "2026-10-12", 30);
        log(&conn, s, "2026-10-26", 90);
        let c = hours_chart(&conn, day("2026-10-29"), HoursRange::All).unwrap();
        assert_eq!(c.heights, [0.25, 0.25, 1.0]);
    }

    #[test]
    fn unreadable_stored_date_is_an_error() {
        let (conn, s) = setup();
        conn.execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        conn.execute(
            "INSERT INTO study_session (subject_id, date, focus_minutes, description)
             VALUES (?1, '2026-10-2x', 5, '')",
            [s.0],
        )
        .unwrap();
        for r in [HoursRange::Months3, HoursRange::All] {
            assert!(matches!(
                hours_chart(&conn, day("2026-10-29"), r),
                Err(StudyError::BadDate(_))
            ));
        }
    }
}
