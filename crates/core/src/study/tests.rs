// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for study.

use super::*;
use crate::db;
use crate::tasks::TaskId;
use chrono::{DateTime, FixedOffset, Utc};

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
    log_manual_session(&conn, b, today, 30, "", None, today).unwrap();
    log_manual_session(&conn, b, today, 45, "", None, today).unwrap();
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
            log_manual_session(&conn, s, today, m, "", None, today),
            Err(StudyError::BadMinutes)
        ));
    }
    assert!(matches!(
        log_manual_session(&conn, s, day("2026-11-09"), 10, "", None, today),
        Err(StudyError::FutureDate)
    ));
    assert!(matches!(
        log_manual_session(&conn, SubjectId(99), today, 10, "", None, today),
        Err(StudyError::SubjectNotFound)
    ));
    log_manual_session(&conn, s, today, 10, "  notes  ", None, today).unwrap();
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
    log_manual_session(conn, s, day(date), minutes, "", None, day("2026-11-08")).unwrap();
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
        log_manual_session(&conn, a, day("2026-11-08"), 10, "", None, day("2026-11-08")),
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

fn at(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

fn topic_titles(conn: &Connection, now: &str) -> Vec<String> {
    list_topics(conn, at(now))
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect()
}

const NOW: &str = "2026-06-15T12:00:00+00:00";

#[test]
fn create_topic_trims_and_rejects_empty_title() {
    let conn = db::test_conn();
    let math = create_subject(&conn, "Math").unwrap();
    create_topic(&conn, math, "  Limits  ", Some(day("2026-01-01"))).unwrap();
    let t = &list_topics(&conn, at(NOW)).unwrap()[0];
    assert_eq!(
        (t.title.as_str(), t.subject, t.subject_name.as_str()),
        ("Limits", math, "Math")
    );
    assert_eq!((t.due, t.done), (Some(day("2026-01-01")), false));
    assert!(matches!(
        create_topic(&conn, math, "   ", None),
        Err(StudyError::EmptyTitle)
    ));
    assert_eq!(topic_titles(&conn, NOW), ["Limits"]);
}

#[test]
fn create_topic_needs_an_active_subject() {
    let conn = db::test_conn();
    let math = create_subject(&conn, "Math").unwrap();
    set_subject_archived(&conn, math, true).unwrap();
    assert!(matches!(
        create_topic(&conn, math, "x", None),
        Err(StudyError::SubjectArchived)
    ));
    assert!(matches!(
        create_topic(&conn, SubjectId(999), "x", None),
        Err(StudyError::SubjectNotFound)
    ));
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM task", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn list_topics_orders_pending_then_done_and_dated_first() {
    let conn = db::test_conn();
    let s = create_subject(&conn, "S").unwrap();
    let today = day("2026-06-15");
    let undated = create_topic(&conn, s, "undated", None).unwrap();
    create_topic(&conn, s, "late", Some(day("2026-07-01"))).unwrap();
    create_topic(&conn, s, "early", Some(day("2026-06-20"))).unwrap();
    let done = create_topic(&conn, s, "done", Some(day("2026-06-01"))).unwrap();
    crate::tasks::set_done(&conn, done, true, today).unwrap();
    assert_eq!(
        topic_titles(&conn, NOW),
        ["early", "late", "undated", "done"]
    );
    crate::tasks::set_done(&conn, undated, true, today).unwrap();
    assert_eq!(
        topic_titles(&conn, NOW),
        ["early", "late", "done", "undated"]
    );
}

#[test]
fn list_topics_hides_old_done_and_archived_subjects() {
    let conn = db::test_conn();
    let s = create_subject(&conn, "S").unwrap();
    let old = create_topic(&conn, s, "old", None).unwrap();
    crate::tasks::set_done(&conn, old, true, day("2026-06-15")).unwrap();
    // Done now (UTC) is "today" at NOW's day, and "yesterday" a day later.
    let today = Utc::now().to_rfc3339();
    assert_eq!(topic_titles(&conn, &today), ["old"]);
    let tomorrow = (Utc::now() + chrono::TimeDelta::days(1)).to_rfc3339();
    assert!(topic_titles(&conn, &tomorrow).is_empty());
    let other = create_subject(&conn, "O").unwrap();
    create_topic(&conn, other, "hidden", None).unwrap();
    set_subject_archived(&conn, other, true).unwrap();
    assert_eq!(topic_titles(&conn, &today), ["old"]);
}

fn sessions(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM study_session", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn session_points_only_at_a_pending_topic_of_its_subject() {
    let conn = db::test_conn();
    let math = create_subject(&conn, "Math").unwrap();
    let art = create_subject(&conn, "Art").unwrap();
    let today = day("2026-06-15");
    let limits = create_topic(&conn, math, "Limits", None).unwrap();
    let other = create_topic(&conn, art, "Color", None).unwrap();
    let done = create_topic(&conn, math, "Sets", None).unwrap();
    crate::tasks::set_done(&conn, done, true, today).unwrap();
    for bad in [other, done, TaskId(999)] {
        assert!(matches!(
            log_manual_session(&conn, math, today, 10, "", Some(bad), today),
            Err(StudyError::TopicNotFound)
        ));
    }
    assert_eq!(sessions(&conn), 0);
    log_manual_session(&conn, math, today, 10, "", Some(limits), today).unwrap();
    assert_eq!(
        list_sessions(&conn, math).unwrap()[0].topic.as_deref(),
        Some("Limits")
    );
    log_manual_session(&conn, math, today, 5, "", None, today).unwrap();
    assert_eq!(list_sessions(&conn, math).unwrap()[0].topic, None);
}

#[test]
fn subject_problems_win_over_topic_problems() {
    let conn = db::test_conn();
    let math = create_subject(&conn, "Math").unwrap();
    let topic = create_topic(&conn, math, "Limits", None).unwrap();
    let today = day("2026-06-15");
    set_subject_archived(&conn, math, true).unwrap();
    assert!(matches!(
        log_manual_session(&conn, math, today, 10, "", Some(topic), today),
        Err(StudyError::SubjectArchived)
    ));
    assert!(matches!(
        log_manual_session(&conn, SubjectId(99), today, 10, "", Some(topic), today),
        Err(StudyError::SubjectNotFound)
    ));
}

#[test]
fn deleting_a_topic_keeps_its_sessions() {
    let conn = db::test_conn();
    let math = create_subject(&conn, "Math").unwrap();
    let today = day("2026-06-15");
    let topic = create_topic(&conn, math, "Limits", None).unwrap();
    log_manual_session(&conn, math, today, 10, "", Some(topic), today).unwrap();
    crate::tasks::delete(&conn, topic).unwrap();
    let list = list_sessions(&conn, math).unwrap();
    assert_eq!((list.len(), list[0].topic.as_deref()), (1, None));
}

#[test]
fn subject_topics_are_that_subjects_pending_ones() {
    let conn = db::test_conn();
    let math = create_subject(&conn, "Math").unwrap();
    let art = create_subject(&conn, "Art").unwrap();
    create_topic(&conn, math, "undated", None).unwrap();
    create_topic(&conn, math, "dated", Some(day("2026-06-20"))).unwrap();
    create_topic(&conn, art, "color", None).unwrap();
    let done = create_topic(&conn, math, "done", None).unwrap();
    crate::tasks::set_done(&conn, done, true, day("2026-06-15")).unwrap();
    let titles: Vec<_> = subject_topics(&conn, math)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, ["dated", "undated"]);
}

#[test]
fn today_topics_are_only_those_due_today() {
    let conn = db::test_conn();
    let s = create_subject(&conn, "S").unwrap();
    let old = create_subject(&conn, "Old").unwrap();
    // Done now counts as done today, so the whole test runs on the real day.
    let now = Utc::now().fixed_offset();
    let today = now.date_naive();
    let on = |days| Some(today + chrono::TimeDelta::days(days));
    let topic = |sub, title, due| create_topic(&conn, sub, title, due).unwrap();
    topic(s, "overdue", on(-1));
    topic(s, "future", on(1));
    topic(s, "undated", None);
    let done = topic(s, "done", on(0));
    topic(s, "pending", on(0));
    topic(old, "archived", on(0));
    set_subject_archived(&conn, old, true).unwrap();
    crate::tasks::set_done(&conn, done, true, today).unwrap();
    let titles: Vec<_> = today_topics(&conn, now)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, ["pending", "done"]);
}

mod timer {
    use super::*;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn setup() -> (Connection, SubjectId) {
        let conn = db::test_conn();
        let s = create_subject(&conn, "Math").unwrap();
        (conn, s)
    }

    fn elapsed(conn: &Connection, now: &str) -> i64 {
        active_session(conn, at(now)).unwrap().unwrap().elapsed_secs
    }

    #[test]
    fn start_rules() {
        let (conn, s) = setup();
        let now = at("2026-11-08T10:00:00-05:00");
        assert!(matches!(
            start_session(&conn, SubjectId(99), None, now),
            Err(StudyError::SubjectNotFound)
        ));
        let other = create_subject(&conn, "Art").unwrap();
        let foreign = create_topic(&conn, other, "Color", None).unwrap();
        assert!(matches!(
            start_session(&conn, s, Some(foreign), now),
            Err(StudyError::TopicNotFound)
        ));
        let done = create_topic(&conn, s, "Done", None).unwrap();
        crate::tasks::set_done(&conn, done, true, now.date_naive()).unwrap();
        assert!(matches!(
            start_session(&conn, s, Some(done), now),
            Err(StudyError::TopicNotFound)
        ));
        set_subject_archived(&conn, other, true).unwrap();
        assert!(matches!(
            start_session(&conn, other, None, now),
            Err(StudyError::SubjectArchived)
        ));
        assert!(active_session(&conn, now).unwrap().is_none());
        start_session(&conn, s, None, now).unwrap();
        assert!(matches!(
            start_session(&conn, s, None, now),
            Err(StudyError::SessionActive)
        ));
    }

    #[test]
    fn elapsed_counts_small_gaps_only() {
        let (conn, s) = setup();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        assert_eq!(elapsed(&conn, "2026-11-08T10:00:00Z"), 0);
        assert_eq!(elapsed(&conn, "2026-11-08T10:01:30Z"), 90);
        assert_eq!(elapsed(&conn, "2026-11-08T10:02:00Z"), 120);
        assert_eq!(elapsed(&conn, "2026-11-08T10:02:01Z"), 0);
        assert_eq!(elapsed(&conn, "2026-11-08T09:59:00Z"), 0);
    }

    #[test]
    fn heartbeat_banks_and_ignores_long_gaps() {
        let (conn, s) = setup();
        heartbeat(&conn, at("2026-11-08T10:00:00Z")).unwrap();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        heartbeat(&conn, at("2026-11-08T10:01:00Z")).unwrap();
        assert_eq!(elapsed(&conn, "2026-11-08T10:01:00Z"), 60);
        assert_eq!(elapsed(&conn, "2026-11-08T10:01:30Z"), 90);
        heartbeat(&conn, at("2026-11-08T11:00:00Z")).unwrap();
        assert_eq!(elapsed(&conn, "2026-11-08T11:00:10Z"), 60);
    }

    fn paused(conn: &Connection, now: &str) -> bool {
        active_session(conn, at(now)).unwrap().unwrap().paused
    }

    #[test]
    fn pause_banks_and_stops_counting() {
        let (conn, s) = setup();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        pause_session(&conn, at("2026-11-08T10:00:50Z")).unwrap();
        assert!(paused(&conn, "2026-11-08T10:00:50Z"));
        assert_eq!(elapsed(&conn, "2026-11-08T10:01:20Z"), 50);
        heartbeat(&conn, at("2026-11-08T10:01:30Z")).unwrap();
        assert_eq!(elapsed(&conn, "2026-11-08T10:01:30Z"), 50);
        // Pausing again changes nothing.
        pause_session(&conn, at("2026-11-08T10:01:40Z")).unwrap();
        assert_eq!(elapsed(&conn, "2026-11-08T10:01:40Z"), 50);
    }

    #[test]
    fn resume_does_not_count_the_pause() {
        let (conn, s) = setup();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        pause_session(&conn, at("2026-11-08T10:00:50Z")).unwrap();
        resume_session(&conn, at("2026-11-08T10:30:00Z")).unwrap();
        assert!(!paused(&conn, "2026-11-08T10:30:00Z"));
        assert_eq!(elapsed(&conn, "2026-11-08T10:30:10Z"), 60);
        // Resuming a running session keeps the time not yet banked.
        resume_session(&conn, at("2026-11-08T10:30:20Z")).unwrap();
        assert_eq!(elapsed(&conn, "2026-11-08T10:30:30Z"), 80);
    }

    #[test]
    fn long_gap_pauses_and_active_session_only_reads() {
        let (conn, s) = setup();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        heartbeat(&conn, at("2026-11-08T10:01:00Z")).unwrap();
        assert!(paused(&conn, "2026-11-08T11:00:00Z"));
        assert_eq!(elapsed(&conn, "2026-11-08T11:00:00Z"), 60);
        // The read did not write: a read inside the gap still sees it running.
        assert!(!paused(&conn, "2026-11-08T10:01:30Z"));
        heartbeat(&conn, at("2026-11-08T11:00:00Z")).unwrap();
        assert!(paused(&conn, "2026-11-08T11:00:01Z"));
        assert_eq!(elapsed(&conn, "2026-11-08T11:00:01Z"), 60);
    }

    #[test]
    fn backwards_clock_counts_nothing_and_does_not_pause() {
        let (conn, s) = setup();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        heartbeat(&conn, at("2026-11-08T09:00:00Z")).unwrap();
        assert!(!paused(&conn, "2026-11-08T09:00:10Z"));
        assert_eq!(elapsed(&conn, "2026-11-08T09:00:10Z"), 10);
    }

    #[test]
    fn finish_while_paused_saves_what_was_counted() {
        let (conn, s) = setup();
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        heartbeat(&conn, at("2026-11-08T10:01:00Z")).unwrap();
        pause_session(&conn, at("2026-11-08T10:02:10Z")).unwrap();
        assert_eq!(
            finish_session(&conn, at("2026-11-08T12:00:00Z")).unwrap(),
            2
        );
    }

    #[test]
    fn pause_and_resume_need_a_session() {
        let conn = db::test_conn();
        let now = at("2026-11-08T10:00:00Z");
        assert!(matches!(
            pause_session(&conn, now),
            Err(StudyError::NoActiveSession)
        ));
        assert!(matches!(
            resume_session(&conn, now),
            Err(StudyError::NoActiveSession)
        ));
    }

    #[test]
    fn finish_floors_minutes_and_saves_on_start_day() {
        let (conn, s) = setup();
        let topic = create_topic(&conn, s, "Limits", None).unwrap();
        start_session(&conn, s, Some(topic), at("2026-11-08T23:30:00-05:00")).unwrap();
        for m in 1..=3 {
            heartbeat(&conn, at(&format!("2026-11-08T23:{:02}:00-05:00", 30 + m))).unwrap();
        }
        // 3m + 59s, past midnight: floors to 3.
        assert_eq!(
            finish_session(&conn, at("2026-11-09T00:33:59-05:00")).unwrap(),
            3
        );
        let sessions = list_sessions(&conn, s).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].date, day("2026-11-08"));
        assert_eq!(sessions[0].minutes, 3);
        assert_eq!(sessions[0].topic.as_deref(), Some("Limits"));
        assert!(
            active_session(&conn, at("2026-11-09T00:34:00-05:00"))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            finish_session(&conn, at("2026-11-09T00:34:00-05:00")),
            Err(StudyError::NoActiveSession)
        ));
    }

    #[test]
    fn finish_too_short_or_failing_keeps_the_session() {
        let (conn, s) = setup();
        let topic = create_topic(&conn, s, "Limits", None).unwrap();
        start_session(&conn, s, Some(topic), at("2026-11-08T10:00:00Z")).unwrap();
        assert!(matches!(
            finish_session(&conn, at("2026-11-08T10:00:59Z")),
            Err(StudyError::TooShort)
        ));
        // The seconds banked by the failed attempt were rolled back too.
        assert_eq!(elapsed(&conn, "2026-11-08T10:00:00Z"), 0);
        crate::tasks::set_done(&conn, topic, true, day("2026-11-08")).unwrap();
        heartbeat(&conn, at("2026-11-08T10:01:00Z")).unwrap();
        heartbeat(&conn, at("2026-11-08T10:02:00Z")).unwrap();
        assert!(matches!(
            finish_session(&conn, at("2026-11-08T10:02:00Z")),
            Err(StudyError::TopicNotFound)
        ));
        assert_eq!(elapsed(&conn, "2026-11-08T10:02:00Z"), 120);
        assert!(list_sessions(&conn, s).unwrap().is_empty());
        set_subject_archived(&conn, s, true).unwrap();
        assert!(matches!(
            finish_session(&conn, at("2026-11-08T10:02:00Z")),
            Err(StudyError::TopicNotFound | StudyError::SubjectArchived)
        ));
        assert!(
            active_session(&conn, at("2026-11-08T10:02:00Z"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn discard_drops_the_session() {
        let (conn, s) = setup();
        assert!(matches!(
            discard_session(&conn),
            Err(StudyError::NoActiveSession)
        ));
        start_session(&conn, s, None, at("2026-11-08T10:00:00Z")).unwrap();
        discard_session(&conn).unwrap();
        assert!(
            active_session(&conn, at("2026-11-08T10:00:01Z"))
                .unwrap()
                .is_none()
        );
        assert!(list_sessions(&conn, s).unwrap().is_empty());
    }

    #[test]
    fn clock_and_break_formats() {
        for (secs, text) in [
            (0, "00:00"),
            (59, "00:59"),
            (60, "01:00"),
            (3599, "59:59"),
            (3600, "1:00:00"),
            (3661, "1:01:01"),
        ] {
            assert_eq!(format_clock(secs), text);
        }
        for (m, b) in [(5, 1), (25, 5), (50, 10), (3, 1)] {
            assert_eq!(suggested_break(m), b);
        }
    }

    #[test]
    fn deleting_the_topic_keeps_the_session() {
        let (conn, s) = setup();
        let topic = create_topic(&conn, s, "Limits", None).unwrap();
        let now = at("2026-11-08T10:00:00Z");
        start_session(&conn, s, Some(topic), now).unwrap();
        assert_eq!(
            active_session(&conn, now)
                .unwrap()
                .unwrap()
                .topic
                .as_deref(),
            Some("Limits")
        );
        crate::tasks::delete(&conn, topic).unwrap();
        let active = active_session(&conn, now).unwrap().unwrap();
        assert_eq!((active.subject_name.as_str(), active.topic), ("Math", None));
    }
}
