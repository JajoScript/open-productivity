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
