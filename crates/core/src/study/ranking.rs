// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The subjects studied the most over the whole history.

use rusqlite::Connection;

use super::{StudyError, SubjectId};

#[derive(Debug, PartialEq)]
pub struct RankedSubject {
    pub id: SubjectId,
    pub name: String,
    pub total_minutes: i64,
    /// Minutes relative to the first subject, in (0, 1].
    pub width: f32,
}

/// At most 6, most minutes first, ties by name. Archived subjects count; subjects without
/// minutes are left out.
pub fn top_subjects(conn: &Connection) -> Result<Vec<RankedSubject>, StudyError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, SUM(l.focus_minutes) AS total
         FROM subject s JOIN study_session l ON l.subject_id = s.id
         GROUP BY s.id HAVING total > 0
         ORDER BY total DESC, s.name LIMIT 6",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RankedSubject {
            id: SubjectId(r.get(0)?),
            name: r.get(1)?,
            total_minutes: r.get(2)?,
            width: 0.0,
        })
    })?;
    let mut ranked: Vec<RankedSubject> = rows.collect::<Result<_, _>>()?;
    let max = ranked.first().map_or(0, |s| s.total_minutes);
    for s in &mut ranked {
        s.width = s.total_minutes as f32 / max as f32;
    }
    Ok(ranked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::study::{create_subject, log_manual_session, set_subject_archived};

    fn log(conn: &Connection, s: SubjectId, minutes: i64) {
        let day = "2026-10-01".parse().unwrap();
        log_manual_session(conn, s, day, minutes, "", None, day).unwrap();
    }

    fn names(conn: &Connection) -> Vec<String> {
        top_subjects(conn)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    #[test]
    fn orders_by_minutes_and_skips_subjects_without_any() {
        let conn = db::test_conn();
        let a = create_subject(&conn, "A").unwrap();
        let b = create_subject(&conn, "B").unwrap();
        create_subject(&conn, "Empty").unwrap();
        log(&conn, a, 30);
        log(&conn, b, 90);
        log(&conn, a, 30);
        assert_eq!(names(&conn), ["B", "A"]);
        assert_eq!(top_subjects(&conn).unwrap()[1].total_minutes, 60);
    }

    #[test]
    fn keeps_six_and_includes_archived() {
        let conn = db::test_conn();
        for i in 1..=7 {
            let s = create_subject(&conn, &format!("M{i}")).unwrap();
            log(&conn, s, i * 10);
            if i == 7 {
                set_subject_archived(&conn, s, true).unwrap();
            }
        }
        assert_eq!(names(&conn), ["M7", "M6", "M5", "M4", "M3", "M2"]);
    }

    #[test]
    fn ties_go_by_name() {
        let conn = db::test_conn();
        let b = create_subject(&conn, "Beta").unwrap();
        let a = create_subject(&conn, "Alpha").unwrap();
        log(&conn, b, 20);
        log(&conn, a, 20);
        assert_eq!(names(&conn), ["Alpha", "Beta"]);
    }

    #[test]
    fn width_is_relative_to_the_first_and_nothing_is_empty() {
        let conn = db::test_conn();
        assert!(top_subjects(&conn).unwrap().is_empty());
        let a = create_subject(&conn, "A").unwrap();
        let b = create_subject(&conn, "B").unwrap();
        log(&conn, a, 80);
        log(&conn, b, 20);
        let widths: Vec<f32> = top_subjects(&conn)
            .unwrap()
            .iter()
            .map(|s| s.width)
            .collect();
        assert_eq!(widths, [1.0, 0.25]);
    }
}
