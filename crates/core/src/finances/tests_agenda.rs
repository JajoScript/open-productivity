// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the pending payments the Calendar and Today show.

use chrono::NaiveDate;

use super::tests::account;
use super::*;
use crate::db;
use crate::recurrence::Frequency;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup() -> (rusqlite::Connection, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "A", AccountKind::Checking, "", "1000").unwrap();
    (conn, a)
}

fn plan(
    conn: &rusqlite::Connection,
    a: AccountId,
    name: &str,
    date: &str,
    form: ScheduleForm,
) -> ScheduledId {
    create_scheduled(conn, TxKind::Expense, name, a, "10", None, d(date), form).unwrap()
}

fn names(p: &[Payment]) -> Vec<&str> {
    p.iter().map(|p| p.description.as_str()).collect()
}

#[test]
fn a_range_across_months_keeps_each_occurrence_once() {
    let (conn, a) = setup();
    let today = d("2026-11-15");
    plan(&conn, a, "Old", "2026-09-10", ScheduleForm::Once);
    plan(
        &conn,
        a,
        "Rent",
        "2026-11-20",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    plan(&conn, a, "Out", "2027-02-01", ScheduleForm::Once);
    // 2026-10-26 to 2026-12-31: the overdue "Old" is repeated by November, outside the range.
    let got = payments_between(&conn, d("2026-10-26"), d("2026-12-31"), today).unwrap();
    assert_eq!(names(&got), ["Rent", "Rent"]);
    assert_eq!(
        (got[0].date, got[1].date),
        (d("2026-11-20"), d("2026-12-20"))
    );
}

#[test]
fn overdue_is_marked_and_not_duplicated_across_months() {
    let (conn, a) = setup();
    let today = d("2026-11-15");
    plan(&conn, a, "Late", "2026-09-10", ScheduleForm::Once);
    let got = payments_between(&conn, d("2026-08-31"), d("2026-12-06"), today).unwrap();
    assert_eq!(names(&got), ["Late"]);
    assert!(got[0].overdue);
    assert_eq!(got[0].date, d("2026-09-10"));
}

#[test]
fn a_bill_is_included_as_an_expense() {
    let (conn, a) = setup();
    let card = create_card(&conn, "Visa", "", "CLP", "1000000", 5, 20, None).unwrap();
    create_transaction(
        &conn,
        TxKind::Expense,
        card,
        "500",
        d("2026-10-01"),
        "",
        None,
    )
    .unwrap();
    plan(&conn, a, "Gym", "2026-11-10", ScheduleForm::Once);
    let got = payments_between(&conn, d("2026-11-01"), d("2026-11-30"), d("2026-11-15")).unwrap();
    assert_eq!(names(&got), ["Gym", "Visa"]);
    let visa = &got[1];
    assert!(visa.bill && visa.kind == TxKind::Expense && !visa.overdue);
    assert_eq!((visa.date, visa.amount), (d("2026-11-20"), 500));
}

#[test]
fn paid_and_skipped_are_left_out() {
    let (conn, a) = setup();
    let today = d("2026-11-15");
    let paid = plan(&conn, a, "Paid", "2026-11-10", ScheduleForm::Once);
    let skipped = plan(&conn, a, "Skipped", "2026-11-11", ScheduleForm::Once);
    plan(&conn, a, "Pending", "2026-11-12", ScheduleForm::Once);
    confirm_scheduled(&conn, paid, d("2026-11-10"), "10").unwrap();
    skip_scheduled(&conn, skipped).unwrap();
    let got = payments_between(&conn, d("2026-11-01"), d("2026-11-30"), today).unwrap();
    assert_eq!(names(&got), ["Pending"]);
}

#[test]
fn today_has_todays_and_the_overdue_by_date() {
    let (conn, a) = setup();
    let today = d("2026-11-15");
    plan(&conn, a, "Old", "2026-08-02", ScheduleForm::Once);
    plan(&conn, a, "B", "2026-11-15", ScheduleForm::Once);
    plan(&conn, a, "A", "2026-11-15", ScheduleForm::Once);
    plan(&conn, a, "Soon", "2026-11-16", ScheduleForm::Once);
    let got = payments_due(&conn, today).unwrap();
    assert_eq!(names(&got), ["Old", "A", "B"]);
    assert_eq!(
        got.iter().map(|p| p.overdue).collect::<Vec<_>>(),
        [true, false, false]
    );
}

#[test]
fn an_instance_without_finances_has_no_payments() {
    let conn = db::test_conn();
    let today = d("2026-11-15");
    assert!(payments_due(&conn, today).unwrap().is_empty());
    assert!(
        payments_between(&conn, d("2026-11-01"), d("2026-11-30"), today)
            .unwrap()
            .is_empty()
    );
}
