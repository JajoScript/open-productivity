// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for study.

use super::*;
use crate::db;

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

#[test]
fn create_trims_and_rejects_empty_name() {
    let conn = db::test_conn();
    let id = create_subject(&conn, "  Math  ").unwrap();
    let s = &list_subjects(&conn).unwrap()[0];
    assert_eq!((s.id, s.name.as_str()), (id, "Math"));
    assert!(matches!(
        create_subject(&conn, "   "),
        Err(StudyError::EmptyName)
    ));
}

#[test]
fn list_orders_by_creation_and_sums_sessions() {
    let conn = db::test_conn();
    let b = create_subject(&conn, "B").unwrap();
    let a = create_subject(&conn, "A").unwrap();
    let today = day("2026-11-08");
    log_manual_session(&conn, b, today, 30, "", today).unwrap();
    log_manual_session(&conn, b, today, 45, "", today).unwrap();
    let list = list_subjects(&conn).unwrap();
    assert_eq!(list.iter().map(|s| s.id).collect::<Vec<_>>(), [b, a]);
    assert_eq!((list[0].total_minutes, list[0].sessions), (75, 2));
    assert_eq!((list[1].total_minutes, list[1].sessions), (0, 0));
}

#[test]
fn manual_session_validations() {
    let conn = db::test_conn();
    let s = create_subject(&conn, "Math").unwrap();
    let today = day("2026-11-08");
    for m in [0, -5] {
        assert!(matches!(
            log_manual_session(&conn, s, today, m, "", today),
            Err(StudyError::BadMinutes)
        ));
    }
    assert!(matches!(
        log_manual_session(&conn, s, day("2026-11-09"), 10, "", today),
        Err(StudyError::FutureDate)
    ));
    assert!(matches!(
        log_manual_session(&conn, SubjectId(99), today, 10, "", today),
        Err(StudyError::SubjectNotFound)
    ));
    log_manual_session(&conn, s, today, 10, "  notes  ", today).unwrap();
    let d: String = conn
        .query_row("SELECT description FROM study_session", [], |r| r.get(0))
        .unwrap();
    assert_eq!(d, "notes");
    assert_eq!(list_subjects(&conn).unwrap()[0].sessions, 1);
}

#[test]
fn parse_minutes_accepts_only_positive_whole_numbers() {
    assert_eq!(parse_minutes(" 45 ").unwrap(), 45);
    for bad in [
        "",
        "0",
        "-3",
        "+3",
        "1.5",
        "abc",
        "1 0",
        "99999999999999999999999",
    ] {
        assert!(
            matches!(parse_minutes(bad), Err(StudyError::BadMinutes)),
            "{bad}"
        );
    }
}

#[test]
fn duration_format() {
    let f = format_duration;
    assert_eq!([f(0), f(45), f(120), f(90)], ["0m", "45m", "2h", "1h 30m"]);
}
