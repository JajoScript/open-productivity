// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for habits.

use super::*;
use crate::db;

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
    let conn = db::test_conn();
    create(&conn, "Read", [true; 7], day(MON), day(MON)).unwrap();
    conn.execute("UPDATE habit SET start_date = 'nope'", [])
        .unwrap();
    assert!(matches!(
        month(&conn, day(MON), day(MON)),
        Err(HabitError::BadDate(d)) if d == "nope"
    ));
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
    let conn = db::test_conn();
    let id = create(&conn, "  Read \n", [true; 7], day(MON), day(MON)).unwrap();
    let (weekdays, start): (i64, String) = conn
        .query_row("SELECT weekdays, start_date FROM habit", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((weekdays, start.as_str()), (127, MON));
    let m = month(&conn, day(MON), day(MON)).unwrap();
    assert_eq!((m.habits[0].id, m.habits[0].name.as_str()), (id, "Read"));
}

#[test]
fn empty_name_is_rejected() {
    let conn = db::test_conn();
    assert!(matches!(
        create(&conn, " \t", [true; 7], day(MON), day(MON)),
        Err(HabitError::EmptyName)
    ));
    assert!(names(&conn, MON).is_empty());
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
    let conn = db::test_conn();
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
}

#[test]
fn selected_weekdays_and_start_decide_when_it_shows() {
    let conn = db::test_conn();
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
}

#[test]
fn month_orders_by_creation_and_hides_not_started_and_unscheduled() {
    let conn = db::test_conn();
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
}

#[test]
fn archived_habits_are_hidden() {
    let conn = db::test_conn();
    create(&conn, "Gone", [true; 7], day(MON), day(MON)).unwrap();
    conn.execute("UPDATE habit SET archived = 1", []).unwrap();
    assert!(names(&conn, MON).is_empty());
}

#[test]
fn set_done_marks_and_unmarks_and_is_idempotent() {
    let conn = db::test_conn();
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
}

#[test]
fn set_done_rejects_future_before_start_and_unscheduled_days() {
    let conn = db::test_conn();
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
}

#[test]
fn today_lists_applicable_active_habits_with_their_mark() {
    let conn = db::test_conn();
    let mon = [true, false, false, false, false, false, false];
    let tue = [false, true, false, false, false, false, false];
    let a = create(&conn, "A", [true; 7], day(MON), day(MON)).unwrap();
    let b = create(&conn, "B", [true; 7], day(MON), day(MON)).unwrap();
    create(&conn, "Tue only", tue, day(MON), day(MON)).unwrap();
    let m = create(&conn, "Mondays", mon, day(MON), day(MON)).unwrap();
    let old = create(&conn, "Old", [true; 7], day("2026-10-01"), day(MON)).unwrap();
    archive(&conn, old).unwrap();
    // Starts after the day asked about.
    let later = day("2026-11-03");
    create(&conn, "Later", [true; 7], later, later).unwrap();
    set_done(&conn, b, day(MON), day(MON), true).unwrap();
    let got: Vec<_> = today(&conn, day(MON))
        .unwrap()
        .into_iter()
        .map(|h| (h.id, h.name, h.done))
        .collect();
    assert_eq!(
        got,
        [
            (a, "A".to_string(), false),
            (b, "B".to_string(), true),
            (m, "Mondays".to_string(), false)
        ]
    );
    assert_eq!(done_count(&today(&conn, day(MON)).unwrap()), 1);
}

#[test]
fn set_done_on_unknown_habit_is_not_found() {
    let conn = db::test_conn();
    assert!(matches!(
        set_done(&conn, HabitId(99), day(MON), day(MON), true),
        Err(HabitError::NotFound)
    ));
}

#[test]
fn month_classifies_every_day() {
    let conn = db::test_conn();
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
}

#[test]
fn stored_mark_on_a_day_that_does_not_apply_is_ignored() {
    let conn = db::test_conn();
    let id = create(&conn, "Read", [true; 7], day(MON), day(MON)).unwrap();
    set_done(&conn, id, day(MON), day(MON), true).unwrap();
    // Schedule changes later: Monday no longer applies, the old mark must not show.
    conn.execute("UPDATE habit SET weekdays = 2", []).unwrap();
    assert_eq!(states(&conn, MON, MON), [DayState::Skipped]);
}

#[test]
fn month_shows_only_its_own_marks_and_active_habits() {
    let conn = db::test_conn();
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
}

#[test]
fn month_reports_today_and_whether_next_is_allowed() {
    let conn = db::test_conn();
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
    assert_eq!(calendar::month_start(day("2026-11-30")), day("2026-11-01"));
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
    let conn = db::test_conn();
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
}

#[test]
fn update_validates_like_create_and_needs_an_existing_habit() {
    let conn = db::test_conn();
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
}

#[test]
fn update_is_retroactive_and_keeps_marks() {
    let conn = db::test_conn();
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
}

#[test]
fn archive_hides_keeps_logs_and_needs_an_existing_habit() {
    let conn = db::test_conn();
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
}

#[test]
fn set_done_works_on_any_past_applicable_day() {
    let conn = db::test_conn();
    let id = create(&conn, "Read", [true; 7], day("2026-10-01"), day(MON)).unwrap();
    set_done(&conn, id, day("2026-10-05"), day(MON), true).unwrap();
    assert_eq!(states(&conn, "2026-10-05", MON), [DayState::Done]);
}

#[test]
fn totals_per_day_and_week_count_future_and_ignore_skipped() {
    let conn = db::test_conn();
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
}

#[test]
fn heat_buckets_by_share_done() {
    let conn = db::test_conn();
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
}

#[test]
fn range_lists_applicable_habits_per_day() {
    let conn = db::test_conn();
    let today = day("2026-11-04");
    // Gym: Mon, Wed, Fri since Wed the 4th. Read: daily since the 1st. Gone: archived.
    let mwf = [true, false, true, false, true, false, false];
    let gym = create(&conn, "Gym", mwf, day("2026-11-04"), today).unwrap();
    let read = create(&conn, "Read", [true; 7], day("2026-11-01"), today).unwrap();
    let gone = create(&conn, "Gone", [true; 7], day("2026-11-01"), today).unwrap();
    archive(&conn, gone).unwrap();
    set_done(&conn, read, day("2026-11-01"), today, true).unwrap();
    set_done(&conn, gym, day("2026-11-04"), today, true).unwrap();
    // Seven days from Sunday the 1st.
    let r = range(&conn, day("2026-11-01"), 7, today).unwrap();
    let v = |d: usize| -> Vec<(&str, bool)> {
        r[d].iter().map(|h| (h.name.as_str(), h.done)).collect()
    };
    assert_eq!(v(0), [("Read", true)]);
    // Monday the 2nd: Gym had not started.
    assert_eq!(v(1), [("Read", false)]);
    // Today: both, in grid order, both done where marked.
    assert_eq!(v(3), [("Gym", true), ("Read", false)]);
    // Thursday: Gym not scheduled. Friday is in the future: pending.
    assert_eq!(v(4), [("Read", false)]);
    assert_eq!(v(5), [("Gym", false), ("Read", false)]);
    // Same days as the grid of the month.
    let m = month(&conn, day("2026-11-01"), today).unwrap();
    for (i, hs) in r.iter().enumerate() {
        let want: Vec<_> = m
            .habits
            .iter()
            .filter(|h| h.days[i] != DayState::Skipped)
            .map(|h| h.id)
            .collect();
        assert_eq!(hs.iter().map(|h| h.id).collect::<Vec<_>>(), want);
    }
}

#[test]
fn restore_brings_back_marks_and_streaks() {
    let conn = db::test_conn();
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
}
