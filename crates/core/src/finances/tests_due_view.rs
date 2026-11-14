// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the Scheduled tab's calendar marks and biggest payments.

use chrono::NaiveDate;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn plan(conn: &rusqlite::Connection, a: AccountId, name: &str, amount: &str, date: &str) {
    create_scheduled(
        conn,
        TxKind::Expense,
        name,
        a,
        amount,
        None,
        d(date),
        ScheduleForm::Once,
    )
    .unwrap();
}

fn setup() -> (rusqlite::Connection, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "A", AccountKind::Checking, "", "1000").unwrap();
    (conn, a)
}

#[test]
fn marks_due_and_overdue_days_of_the_month() {
    let (conn, a) = setup();
    plan(&conn, a, "Old", "10", "2026-10-20");
    plan(&conn, a, "Late", "10", "2026-11-05");
    plan(&conn, a, "Soon", "10", "2026-11-20");
    plan(&conn, a, "Soon2", "10", "2026-11-20");
    plan(&conn, a, "Next", "10", "2026-12-01");
    let days = due_days(&conn, d("2026-11-01"), d("2026-11-14")).unwrap();
    assert_eq!(
        days,
        vec![
            (d("2026-11-05"), DayMark::Overdue),
            (d("2026-11-20"), DayMark::Due)
        ]
    );
}

#[test]
fn biggest_are_sorted_limited_and_relative() {
    let (conn, a) = setup();
    plan(&conn, a, "Small", "100", "2026-11-20");
    plan(&conn, a, "Big", "400", "2026-11-21");
    plan(&conn, a, "Mid", "200", "2026-11-22");
    create_scheduled(
        &conn,
        TxKind::Income,
        "Pay",
        a,
        "9000",
        None,
        d("2026-11-23"),
        ScheduleForm::Once,
    )
    .unwrap();
    let top = biggest_due(&conn, d("2026-11-01"), d("2026-11-14"), 2).unwrap();
    let names: Vec<_> = top.iter().map(|b| b.description.as_str()).collect();
    assert_eq!(names, ["Big", "Mid"]);
    assert_eq!(top[0].share, 1.0);
    assert_eq!(top[1].share, 0.5);
}

#[test]
fn nothing_due_gives_empty_lists() {
    let (conn, _) = setup();
    assert!(
        due_days(&conn, d("2026-11-01"), d("2026-11-14"))
            .unwrap()
            .is_empty()
    );
    assert!(
        biggest_due(&conn, d("2026-11-01"), d("2026-11-14"), 4)
            .unwrap()
            .is_empty()
    );
}
