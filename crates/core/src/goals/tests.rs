// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

use chrono::NaiveDate;

use super::*;
use crate::db::test_conn;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn today() -> NaiveDate {
    d(2026, 6, 1)
}

fn draft(title: &str, deadline: Option<NaiveDate>) -> GoalDraft {
    GoalDraft {
        title: title.into(),
        deadline,
        ..GoalDraft::default()
    }
}

#[test]
fn create_trims_and_stores() {
    let conn = test_conn();
    let id = create(
        &conn,
        GoalKind::Project,
        &GoalDraft {
            title: "  Learn Rust ".into(),
            emoji: " 🦀 ".into(),
            description: " daily ".into(),
            deadline: Some(d(2026, 12, 31)),
            numeric: None,
        },
    )
    .unwrap();
    let g = get(&conn, id, today()).unwrap();
    assert_eq!(g.title, "Learn Rust");
    assert_eq!(g.emoji, "🦀");
    assert_eq!(g.description, "daily");
    assert_eq!(g.kind, GoalKind::Project);
    assert_eq!(g.deadline, Some(d(2026, 12, 31)));
}

#[test]
fn create_rejects_blank_title() {
    let conn = test_conn();
    assert!(matches!(
        create(&conn, GoalKind::Goal, &draft("   ", None)),
        Err(GoalError::EmptyTitle)
    ));
    assert!(list(&conn, today()).unwrap().is_empty());
}

#[test]
fn order_is_deadline_then_undated_last() {
    let conn = test_conn();
    create(&conn, GoalKind::Goal, &draft("none", None)).unwrap();
    create(&conn, GoalKind::Goal, &draft("late", Some(d(2026, 12, 1)))).unwrap();
    create(&conn, GoalKind::Goal, &draft("soon", Some(d(2026, 7, 1)))).unwrap();
    create(&conn, GoalKind::Goal, &draft("none2", None)).unwrap();
    let titles: Vec<_> = list(&conn, today())
        .unwrap()
        .into_iter()
        .map(|g| g.title)
        .collect();
    assert_eq!(titles, ["soon", "late", "none", "none2"]);
}

#[test]
fn update_edits_fields_keeps_kind_and_can_clear_deadline() {
    let conn = test_conn();
    let id = create(&conn, GoalKind::Project, &draft("a", Some(d(2026, 7, 1)))).unwrap();
    update(
        &conn,
        id,
        &GoalDraft {
            title: " b ".into(),
            emoji: "🎯".into(),
            description: "x".into(),
            deadline: Some(d(2026, 8, 1)),
            numeric: None,
        },
    )
    .unwrap();
    let g = get(&conn, id, today()).unwrap();
    assert_eq!((g.title.as_str(), g.emoji.as_str()), ("b", "🎯"));
    assert_eq!(g.kind, GoalKind::Project);
    assert_eq!(g.deadline, Some(d(2026, 8, 1)));
    update(&conn, id, &draft("b", None)).unwrap();
    assert_eq!(get(&conn, id, today()).unwrap().deadline, None);
}

#[test]
fn update_rejects_blank_title() {
    let conn = test_conn();
    let id = create(&conn, GoalKind::Goal, &draft("a", None)).unwrap();
    assert!(matches!(
        update(&conn, id, &draft(" ", None)),
        Err(GoalError::EmptyTitle)
    ));
    assert_eq!(get(&conn, id, today()).unwrap().title, "a");
}

#[test]
fn delete_removes_goal() {
    let conn = test_conn();
    let id = create(&conn, GoalKind::Goal, &draft("a", None)).unwrap();
    delete(&conn, id).unwrap();
    assert!(list(&conn, today()).unwrap().is_empty());
}

#[test]
fn missing_id_is_not_found() {
    let conn = test_conn();
    let id = GoalId(99);
    assert!(matches!(get(&conn, id, today()), Err(GoalError::NotFound)));
    assert!(matches!(
        update(&conn, id, &draft("a", None)),
        Err(GoalError::NotFound)
    ));
    assert!(matches!(delete(&conn, id), Err(GoalError::NotFound)));
}

#[test]
fn days_left_counts_from_today() {
    assert_eq!(days_left(d(2026, 6, 11), today()), 10);
    assert_eq!(days_left(today(), today()), 0);
    assert_eq!(days_left(d(2026, 5, 30), today()), -2);
}

#[test]
fn goal_carries_days_left() {
    let conn = test_conn();
    let got: Vec<_> = [
        Some(d(2026, 6, 11)),
        Some(today()),
        Some(d(2026, 5, 30)),
        None,
    ]
    .into_iter()
    .map(|dl| {
        let id = create(&conn, GoalKind::Goal, &draft("g", dl)).unwrap();
        get(&conn, id, today()).unwrap().days_left
    })
    .collect();
    assert_eq!(got, [Some(10), Some(0), Some(-2), None]);
}

fn goal(conn: &Connection) -> GoalId {
    create(conn, GoalKind::Goal, &draft("g", None)).unwrap()
}

#[test]
fn create_task_links_it_with_optional_date_and_validates() {
    let conn = test_conn();
    let g = goal(&conn);
    let a = create_task(&conn, g, "  a ", Some(d(2026, 6, 5))).unwrap();
    let b = create_task(&conn, g, "b", None).unwrap();
    let tasks = list_tasks(&conn, g).unwrap();
    assert_eq!(tasks.iter().map(|t| t.id).collect::<Vec<_>>(), [a, b]);
    assert_eq!(tasks[0].title, "a");
    assert_eq!(tasks[0].due, Some(d(2026, 6, 5)));
    assert_eq!(tasks[1].due, None);
    assert!(matches!(
        create_task(&conn, g, "  ", None),
        Err(GoalError::EmptyTitle)
    ));
    assert!(matches!(
        create_task(&conn, GoalId(99), "x", None),
        Err(GoalError::NotFound)
    ));
}

#[test]
fn list_tasks_puts_pending_before_done() {
    let conn = test_conn();
    let g = goal(&conn);
    let a = create_task(&conn, g, "a", None).unwrap();
    let b = create_task(&conn, g, "b", None).unwrap();
    crate::tasks::set_done(&conn, a, true, today()).unwrap();
    let ids: Vec<_> = list_tasks(&conn, g).unwrap().iter().map(|t| t.id).collect();
    assert_eq!(ids, [b, a]);
}

#[test]
fn task_progress_counts_done_over_total() {
    let conn = test_conn();
    let g = goal(&conn);
    let got = get(&conn, g, today()).unwrap();
    assert_eq!(
        (got.tasks_done, got.tasks_total, got.tasks_percent),
        (0, 0, 0)
    );
    let a = create_task(&conn, g, "a", None).unwrap();
    create_task(&conn, g, "b", None).unwrap();
    create_task(&conn, g, "c", None).unwrap();
    crate::tasks::set_done(&conn, a, true, today()).unwrap();
    let got = get(&conn, g, today()).unwrap();
    assert_eq!(
        (got.tasks_done, got.tasks_total, got.tasks_percent),
        (1, 3, 33)
    );
    assert_eq!(list(&conn, today()).unwrap()[0].tasks_percent, 33);
}

#[test]
fn percent_truncates_and_handles_zero() {
    assert_eq!(percent(0, 0), 0);
    assert_eq!(percent(1, 3), 33);
    assert_eq!(percent(2, 3), 66);
    assert_eq!(percent(3, 3), 100);
}

#[test]
fn delete_unlinks_tasks_without_deleting_them() {
    let conn = test_conn();
    let g = goal(&conn);
    let t = create_task(&conn, g, "a", None).unwrap();
    delete(&conn, g).unwrap();
    let kept: i64 = conn
        .query_row(
            "SELECT count(*) FROM task WHERE id = ?1 AND goal_id IS NULL",
            [t.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1);
}

fn numeric(title: &str, target: i64, unit: &str) -> GoalDraft {
    GoalDraft {
        title: title.into(),
        numeric: Some(Numeric {
            target,
            unit: unit.into(),
        }),
        ..GoalDraft::default()
    }
}

fn numeric_goal(conn: &Connection, target: i64) -> GoalId {
    create(conn, GoalKind::Goal, &numeric("n", target, "km")).unwrap()
}

#[test]
fn create_numeric_validates_and_stores() {
    let conn = test_conn();
    let id = create(&conn, GoalKind::Goal, &numeric("run", 100, " km ")).unwrap();
    let n = get(&conn, id, today()).unwrap().numeric.unwrap();
    assert_eq!(
        (n.target, n.unit.as_str(), n.total, n.percent),
        (100, "km", 0, 0)
    );
    let empty_unit = create(&conn, GoalKind::Goal, &numeric("a", 5, "")).unwrap();
    assert_eq!(
        get(&conn, empty_unit, today())
            .unwrap()
            .numeric
            .unwrap()
            .unit,
        ""
    );
    for bad in [0, -3] {
        assert!(matches!(
            create(&conn, GoalKind::Goal, &numeric("b", bad, "")),
            Err(GoalError::BadTarget)
        ));
    }
    assert_eq!(list(&conn, today()).unwrap().len(), 2);
    let plain = create(&conn, GoalKind::Goal, &draft("p", None)).unwrap();
    assert_eq!(get(&conn, plain, today()).unwrap().numeric, None);
}

#[test]
fn project_takes_no_measure() {
    let conn = test_conn();
    assert!(matches!(
        create(&conn, GoalKind::Project, &numeric("p", 5, "")),
        Err(GoalError::ProjectMeasure)
    ));
    assert!(list(&conn, today()).unwrap().is_empty());
}

#[test]
fn update_target_changes_count_and_unit_only() {
    let conn = test_conn();
    let id = numeric_goal(&conn, 10);
    update(&conn, id, &numeric(" t ", 20, " pages ")).unwrap();
    let g = get(&conn, id, today()).unwrap();
    assert_eq!(g.title, "t");
    let n = g.numeric.unwrap();
    assert_eq!((n.target, n.unit.as_str()), (20, "pages"));
    assert!(matches!(
        update(&conn, id, &numeric("t", 0, "")),
        Err(GoalError::BadTarget)
    ));
}

#[test]
fn update_never_changes_the_measure() {
    let conn = test_conn();
    let plain = goal(&conn);
    let num = numeric_goal(&conn, 10);
    assert!(matches!(
        update(&conn, plain, &numeric("g", 5, "")),
        Err(GoalError::MeasureChanged)
    ));
    assert!(matches!(
        update(&conn, num, &draft("n", None)),
        Err(GoalError::MeasureChanged)
    ));
    assert_eq!(get(&conn, plain, today()).unwrap().numeric, None);
    assert_eq!(
        get(&conn, num, today()).unwrap().numeric.unwrap().target,
        10
    );
}

#[test]
fn add_entry_validates() {
    let conn = test_conn();
    let n = numeric_goal(&conn, 10);
    for bad in [0, -1] {
        assert!(matches!(
            add_entry(&conn, n, today(), bad),
            Err(GoalError::BadValue)
        ));
    }
    assert!(matches!(
        add_entry(&conn, goal(&conn), today(), 1),
        Err(GoalError::NotNumeric)
    ));
    assert!(matches!(
        add_entry(&conn, GoalId(99), today(), 1),
        Err(GoalError::NotNumeric)
    ));
    assert!(list_entries(&conn, n).unwrap().is_empty());
}

#[test]
fn total_may_exceed_target_and_delete_entry_lowers_it() {
    let conn = test_conn();
    let g = numeric_goal(&conn, 10);
    add_entry(&conn, g, d(2026, 6, 1), 8).unwrap();
    let b = add_entry(&conn, g, d(2026, 6, 2), 7).unwrap();
    let n = get(&conn, g, today()).unwrap().numeric.unwrap();
    assert_eq!((n.total, n.percent), (15, 100));
    delete_entry(&conn, b).unwrap();
    let n = get(&conn, g, today()).unwrap().numeric.unwrap();
    assert_eq!((n.total, n.percent), (8, 80));
    assert!(matches!(delete_entry(&conn, b), Err(GoalError::NotFound)));
}

#[test]
fn numeric_percent_truncates() {
    let conn = test_conn();
    let g = numeric_goal(&conn, 3);
    add_entry(&conn, g, today(), 1).unwrap();
    let got = list(&conn, today()).unwrap();
    assert_eq!(got[0].numeric.as_ref().unwrap().percent, 33);
}

#[test]
fn entries_list_newest_first_and_stay_with_their_goal() {
    let conn = test_conn();
    let (a, b) = (numeric_goal(&conn, 10), numeric_goal(&conn, 10));
    let old = add_entry(&conn, a, d(2026, 5, 1), 1).unwrap();
    let new = add_entry(&conn, a, d(2026, 6, 1), 2).unwrap();
    let same_day = add_entry(&conn, a, d(2026, 6, 1), 3).unwrap();
    add_entry(&conn, b, d(2026, 6, 1), 9).unwrap();
    let entries = list_entries(&conn, a).unwrap();
    let ids: Vec<_> = entries.iter().map(|e| e.id).collect();
    assert_eq!(ids, [same_day, new, old]);
    assert_eq!(entries[0].date, d(2026, 6, 1));
    assert_eq!(get(&conn, a, today()).unwrap().numeric.unwrap().total, 6);
    assert_eq!(get(&conn, b, today()).unwrap().numeric.unwrap().total, 9);
}

#[test]
fn delete_removes_entries() {
    let conn = test_conn();
    let g = numeric_goal(&conn, 10);
    add_entry(&conn, g, today(), 1).unwrap();
    delete(&conn, g).unwrap();
    let left: i64 = conn
        .query_row("SELECT count(*) FROM goal_entry", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn parse_count_reads_positive_whole_numbers() {
    for (text, want) in [
        ("5", 5),
        (" 42 ", 42),
        ("1.000", 1000),
        ("12.345.678", 12_345_678),
    ] {
        assert_eq!(parse_count(text).unwrap(), want, "{text}");
    }
    for bad in [
        "",
        "  ",
        "0",
        "-3",
        "1.5",
        "1,5",
        "1.00",
        "1234.567",
        "1..000",
        "abc",
        "1e3",
        "99999999999999999999",
        "1.000.000.001",
    ] {
        assert!(
            matches!(parse_count(bad), Err(GoalError::BadNumber)),
            "{bad}"
        );
    }
}

fn created(conn: &Connection, id: GoalId, day: &str) {
    // Midday UTC keeps the local date the same in almost every time zone.
    conn.execute(
        "UPDATE goal SET created_at = ?1 WHERE id = ?2",
        params![format!("{day}T12:00:00.000Z"), id.0],
    )
    .unwrap();
}

#[test]
fn expected_pace_is_the_elapsed_share() {
    let (c, dl) = (d(2026, 1, 1), d(2026, 1, 11));
    assert_eq!(expected_pace(c, dl, d(2026, 1, 1)), 0);
    assert_eq!(expected_pace(c, dl, d(2026, 1, 6)), 50);
    assert_eq!(expected_pace(c, dl, d(2026, 1, 4)), 30);
    assert_eq!(expected_pace(c, dl, d(2026, 2, 1)), 100);
    assert_eq!(expected_pace(c, dl, d(2025, 12, 1)), 0);
    assert_eq!(expected_pace(c, c, c), 100);
}

#[test]
fn pace_label_compares_progress_with_pace() {
    assert_eq!(pace_label(60, 50), PaceLabel::Ahead);
    assert_eq!(pace_label(40, 50), PaceLabel::Behind);
    assert_eq!(pace_label(50, 50), PaceLabel::OnPace);
}

#[test]
fn progress_depends_on_the_class() {
    let conn = test_conn();
    let project = create(&conn, GoalKind::Project, &draft("p", None)).unwrap();
    assert_eq!(get(&conn, project, today()).unwrap().progress, Some(0));
    create_task(&conn, project, "t", None).unwrap();
    assert_eq!(get(&conn, project, today()).unwrap().progress, Some(0));
    let n = numeric_goal(&conn, 10);
    add_entry(&conn, n, today(), 5).unwrap();
    assert_eq!(get(&conn, n, today()).unwrap().progress, Some(50));
    let plain = goal(&conn);
    create_task(&conn, plain, "t", None).unwrap();
    assert_eq!(get(&conn, plain, today()).unwrap().progress, None);
}

#[test]
fn pace_needs_progress_and_a_deadline() {
    let conn = test_conn();
    let dl = Some(d(2026, 1, 11));
    let project = create(&conn, GoalKind::Project, &draft("p", dl)).unwrap();
    created(&conn, project, "2026-01-01");
    let on = |id| get(&conn, id, d(2026, 1, 6)).unwrap().pace;
    assert_eq!(
        on(project),
        Some(Pace {
            expected: 50,
            label: PaceLabel::Behind
        })
    );
    let undated = create(&conn, GoalKind::Project, &draft("u", None)).unwrap();
    assert_eq!(on(undated), None);
    let plain = create(&conn, GoalKind::Goal, &draft("g", dl)).unwrap();
    assert_eq!(on(plain), None);
}

#[test]
fn unreadable_creation_date_is_an_error() {
    let conn = test_conn();
    let id = create(&conn, GoalKind::Project, &draft("p", Some(today()))).unwrap();
    conn.execute("UPDATE goal SET created_at = 'x'", [])
        .unwrap();
    assert!(matches!(
        get(&conn, id, today()),
        Err(GoalError::BadDate(_))
    ));
}

#[test]
fn portfolio_averages_only_goals_with_pace() {
    let conn = test_conn();
    assert_eq!(portfolio(&[]), Portfolio::default());
    let dl = Some(d(2026, 1, 11));
    let a = create(&conn, GoalKind::Project, &draft("a", dl)).unwrap();
    let b = numeric_goal(&conn, 10);
    conn.execute(
        "UPDATE goal SET deadline = '2026-01-11' WHERE id = ?1",
        [b.0],
    )
    .unwrap();
    add_entry(&conn, b, today(), 9).unwrap();
    create(&conn, GoalKind::Project, &draft("no deadline", None)).unwrap();
    created(&conn, a, "2026-01-01");
    created(&conn, b, "2026-01-01");
    let goals = list(&conn, d(2026, 1, 4)).unwrap();
    assert_eq!(
        portfolio(&goals),
        Portfolio {
            count: 2,
            progress: 45,
            pace: 30
        }
    );
    let none = create(&conn, GoalKind::Goal, &draft("g", dl)).unwrap();
    assert_eq!(get(&conn, none, today()).unwrap().pace, None);
    assert_eq!(
        portfolio(&[get(&conn, none, today()).unwrap()]),
        Portfolio::default()
    );
}

#[test]
fn timeline_has_only_dated_goals_by_date_then_title() {
    let conn = test_conn();
    create(&conn, GoalKind::Goal, &draft("none", None)).unwrap();
    create(&conn, GoalKind::Goal, &draft("b", Some(d(2026, 7, 1)))).unwrap();
    create(&conn, GoalKind::Goal, &draft("late", Some(d(2026, 12, 1)))).unwrap();
    create(&conn, GoalKind::Goal, &draft("a", Some(d(2026, 7, 1)))).unwrap();
    let goals = list(&conn, today()).unwrap();
    let titles: Vec<_> = timeline(&goals).iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["a", "b", "late"]);
}

fn status(conn: &Connection, id: GoalId) -> (bool, bool, Option<String>) {
    let g = get(conn, id, today()).unwrap();
    let at = conn
        .query_row("SELECT completed_at FROM goal WHERE id = ?1", [id.0], |r| {
            r.get(0)
        })
        .unwrap();
    (g.completed, g.archived, at)
}

#[test]
fn tabs_filter_by_kind_completion_and_archive() {
    let conn = test_conn();
    let g = create(&conn, GoalKind::Goal, &draft("g", None)).unwrap();
    let p = create(&conn, GoalKind::Project, &draft("p", None)).unwrap();
    let done = create(&conn, GoalKind::Goal, &draft("done", None)).unwrap();
    let old = create(&conn, GoalKind::Project, &draft("old", None)).unwrap();
    complete(&conn, done).unwrap();
    set_archived(&conn, old, true).unwrap();
    let goals = list(&conn, today()).unwrap();
    let ids = |tab| -> Vec<GoalId> {
        goals
            .iter()
            .filter(|x| in_tab(x, tab))
            .map(|x| x.id)
            .collect()
    };
    assert_eq!(ids(Tab::All), [g, p, done]);
    assert_eq!(ids(Tab::Goals), [g, done]);
    assert_eq!(ids(Tab::Projects), [p]);
    assert_eq!(ids(Tab::Completed), [done]);
    assert_eq!(ids(Tab::Archived), [old]);
}

#[test]
fn complete_is_final_and_keeps_first_date() {
    let conn = test_conn();
    let id = goal(&conn);
    assert_eq!(status(&conn, id), (false, false, None));
    complete(&conn, id).unwrap();
    conn.execute(
        "UPDATE goal SET completed_at = '2026-01-01T00:00:00.000Z'",
        [],
    )
    .unwrap();
    complete(&conn, id).unwrap();
    update(&conn, id, &draft("edited", None)).unwrap();
    set_archived(&conn, id, true).unwrap();
    assert_eq!(
        status(&conn, id),
        (true, true, Some("2026-01-01T00:00:00.000Z".into()))
    );
    assert!(matches!(
        complete(&conn, GoalId(99)),
        Err(GoalError::NotFound)
    ));
}

#[test]
fn restore_keeps_completed_and_missing_is_not_found() {
    let conn = test_conn();
    let id = goal(&conn);
    complete(&conn, id).unwrap();
    set_archived(&conn, id, true).unwrap();
    set_archived(&conn, id, false).unwrap();
    let (completed, archived, _) = status(&conn, id);
    assert!(completed && !archived);
    assert!(matches!(
        set_archived(&conn, GoalId(99), true),
        Err(GoalError::NotFound)
    ));
}

#[test]
fn complete_reached_follows_the_measure() {
    let conn = test_conn();
    assert_eq!(complete_reached(&conn).unwrap(), 0);
    let exact = numeric_goal(&conn, 10);
    let over = numeric_goal(&conn, 10);
    let under = numeric_goal(&conn, 10);
    add_entry(&conn, exact, today(), 10).unwrap();
    add_entry(&conn, over, today(), 12).unwrap();
    add_entry(&conn, under, today(), 9).unwrap();
    let full = create(&conn, GoalKind::Project, &draft("full", None)).unwrap();
    let t = create_task(&conn, full, "t", None).unwrap();
    crate::tasks::set_done(&conn, t, true, today()).unwrap();
    let half = create(&conn, GoalKind::Project, &draft("half", None)).unwrap();
    let t = create_task(&conn, half, "t", None).unwrap();
    create_task(&conn, half, "u", None).unwrap();
    crate::tasks::set_done(&conn, t, true, today()).unwrap();
    create(&conn, GoalKind::Project, &draft("empty", None)).unwrap();
    let plain = goal(&conn);
    let t = create_task(&conn, plain, "t", None).unwrap();
    crate::tasks::set_done(&conn, t, true, today()).unwrap();
    set_archived(&conn, over, true).unwrap();

    assert_eq!(complete_reached(&conn).unwrap(), 3);
    for id in [exact, over, full] {
        assert!(status(&conn, id).0);
    }
    for id in [under, half, plain] {
        assert!(!status(&conn, id).0);
    }
    assert_eq!(complete_reached(&conn).unwrap(), 0);
}

#[test]
fn completed_stays_completed_when_progress_drops() {
    let conn = test_conn();
    let n = numeric_goal(&conn, 10);
    let e = add_entry(&conn, n, today(), 10).unwrap();
    let p = create(&conn, GoalKind::Project, &draft("p", None)).unwrap();
    let t = create_task(&conn, p, "t", None).unwrap();
    crate::tasks::set_done(&conn, t, true, today()).unwrap();
    assert_eq!(complete_reached(&conn).unwrap(), 2);
    delete_entry(&conn, e).unwrap();
    crate::tasks::set_done(&conn, t, false, today()).unwrap();
    assert_eq!(complete_reached(&conn).unwrap(), 0);
    assert!(status(&conn, n).0 && status(&conn, p).0);
}

#[test]
fn a_repeating_task_never_completes_its_project() {
    let conn = test_conn();
    let p = create(&conn, GoalKind::Project, &draft("p", None)).unwrap();
    let t = create_task(&conn, p, "t", Some(d(2026, 6, 1))).unwrap();
    crate::tasks::set_recurrence(&conn, t, Some((crate::recurrence::Frequency::Weekly, "1")))
        .unwrap();
    crate::tasks::set_done(&conn, t, true, today()).unwrap();
    // The copy born pending keeps the project below 100 %; the user completes it by hand.
    assert_eq!(get(&conn, p, today()).unwrap().tasks_total, 2);
    assert_eq!(complete_reached(&conn).unwrap(), 0);
}

#[test]
fn completed_has_no_pace_and_hidden_goals_leave_portfolio_and_timeline() {
    let conn = test_conn();
    let dl = Some(d(2026, 1, 11));
    let mk = |t| create(&conn, GoalKind::Project, &draft(t, dl)).unwrap();
    let (live, done, old) = (mk("live"), mk("done"), mk("old"));
    for id in [live, done, old] {
        created(&conn, id, "2026-01-01");
    }
    complete(&conn, done).unwrap();
    set_archived(&conn, old, true).unwrap();
    let goals = list(&conn, d(2026, 1, 6)).unwrap();
    let get_ = |id| goals.iter().find(|g| g.id == id).unwrap();
    assert_eq!(get_(done).pace, None);
    assert!(get_(live).pace.is_some() && get_(old).pace.is_some());
    assert_eq!(get_(done).progress, Some(0));
    assert_eq!(
        portfolio(&goals),
        Portfolio {
            count: 1,
            progress: 0,
            pace: 50
        }
    );
    let titles: Vec<_> = timeline(&goals).iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["live"]);
}
