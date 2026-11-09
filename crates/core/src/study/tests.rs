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
    let s = &list_subjects(&conn, SubjectFilter::All).unwrap()[0];
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
    let list = list_subjects(&conn, SubjectFilter::All).unwrap();
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
    assert_eq!(
        list_subjects(&conn, SubjectFilter::All).unwrap()[0].sessions,
        1
    );
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

fn log(conn: &Connection, s: SubjectId, date: &str, minutes: i64) {
    log_manual_session(conn, s, day(date), minutes, "", day("2026-11-08")).unwrap();
}

#[test]
fn archive_restore_and_filters_keep_totals() {
    let conn = db::test_conn();
    let a = create_subject(&conn, "A").unwrap();
    let b = create_subject(&conn, "B").unwrap();
    log(&conn, a, "2026-11-08", 30);
    set_subject_archived(&conn, a, true).unwrap();
    let ids = |f| {
        list_subjects(&conn, f)
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(SubjectFilter::Active), [b]);
    assert_eq!(ids(SubjectFilter::Archived), [a]);
    assert_eq!(ids(SubjectFilter::All), [a, b]);
    let s = &list_subjects(&conn, SubjectFilter::Archived).unwrap()[0];
    assert_eq!((s.archived, s.total_minutes, s.sessions), (true, 30, 1));
    set_subject_archived(&conn, a, false).unwrap();
    assert_eq!(ids(SubjectFilter::Active), [a, b]);
    assert!(matches!(
        set_subject_archived(&conn, SubjectId(99), true),
        Err(StudyError::SubjectNotFound)
    ));
}

#[test]
fn rename_trims_and_validates() {
    let conn = db::test_conn();
    let a = create_subject(&conn, "A").unwrap();
    rename_subject(&conn, a, "  Physics ").unwrap();
    let list = list_subjects(&conn, SubjectFilter::All).unwrap();
    assert_eq!(list[0].name, "Physics");
    assert!(matches!(
        rename_subject(&conn, a, "  "),
        Err(StudyError::EmptyName)
    ));
    assert!(matches!(
        rename_subject(&conn, SubjectId(99), "X"),
        Err(StudyError::SubjectNotFound)
    ));
}

#[test]
fn sessions_list_newest_first_and_delete_lowers_totals() {
    let conn = db::test_conn();
    let a = create_subject(&conn, "A").unwrap();
    log(&conn, a, "2026-11-07", 10);
    log(&conn, a, "2026-11-08", 20);
    log(&conn, a, "2026-11-08", 40);
    let list = list_sessions(&conn, a).unwrap();
    let minutes: Vec<_> = list.iter().map(|s| s.minutes).collect();
    assert_eq!(minutes, [40, 20, 10]);
    delete_session(&conn, list[0].id).unwrap();
    let s = &list_subjects(&conn, SubjectFilter::All).unwrap()[0];
    assert_eq!((s.total_minutes, s.sessions), (30, 2));
}

#[test]
fn delete_missing_session() {
    let conn = db::test_conn();
    assert!(matches!(
        delete_session(&conn, SessionId(99)),
        Err(StudyError::SessionNotFound)
    ));
}

#[test]
fn archived_subject_cannot_receive_a_session() {
    let conn = db::test_conn();
    let a = create_subject(&conn, "A").unwrap();
    set_subject_archived(&conn, a, true).unwrap();
    assert!(matches!(
        log_manual_session(&conn, a, day("2026-11-08"), 10, "", day("2026-11-08")),
        Err(StudyError::SubjectArchived)
    ));
    assert!(list_sessions(&conn, a).unwrap().is_empty());
}

#[test]
fn unreadable_stored_date_is_an_error() {
    let conn = db::test_conn();
    let a = create_subject(&conn, "A").unwrap();
    log(&conn, a, "2026-11-08", 10);
    conn.execute_batch(
        "PRAGMA ignore_check_constraints = ON; UPDATE study_session SET date = 'nope';",
    )
    .unwrap();
    assert!(matches!(list_sessions(&conn, a), Err(StudyError::BadDate(d)) if d == "nope"));
}

#[test]
fn get_subject_finds_archived_and_reports_missing() {
    let conn = db::test_conn();
    let a = create_subject(&conn, "A").unwrap();
    set_subject_archived(&conn, a, true).unwrap();
    assert!(get_subject(&conn, a).unwrap().archived);
    assert!(matches!(
        get_subject(&conn, SubjectId(99)),
        Err(StudyError::SubjectNotFound)
    ));
}
