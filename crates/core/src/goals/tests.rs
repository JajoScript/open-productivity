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
