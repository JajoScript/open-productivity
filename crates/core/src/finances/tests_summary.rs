// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the month summary.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn fig(value: i64, previous: i64) -> Figure {
    Figure { value, previous }
}

fn setup() -> (Connection, AccountId, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = create_account(&conn, "A", AccountKind::Cash, "", "1000").unwrap();
    let b = create_account(&conn, "B", AccountKind::Cash, "", "500").unwrap();
    (conn, a, b)
}

fn tx(conn: &Connection, kind: TxKind, a: AccountId, date: &str, amount: &str) {
    create_transaction(conn, kind, a, amount, d(date), "", None).unwrap();
}

#[test]
fn figures_and_changes() {
    let (conn, a, _) = setup();
    tx(&conn, TxKind::Income, a, "2026-03-02", "300");
    tx(&conn, TxKind::Expense, a, "2026-03-10", "100");
    tx(&conn, TxKind::Income, a, "2026-02-02", "50");
    tx(&conn, TxKind::Expense, a, "2026-02-20", "80");
    let s = month_summary(&conn, d("2026-03-17")).unwrap();
    assert_eq!(s.income, fig(300, 50));
    assert_eq!(s.expense, fig(100, 80));
    assert_eq!(s.balance, fig(200, -30));
    assert_eq!((s.income.change(), s.balance.change()), (250, 230));
    // 1500 opening + 50 - 80 at the end of February, then + 300 - 100.
    assert_eq!(s.accounts_total, fig(1670, 1470));
}

#[test]
fn transfers_move_money_between_accounts_without_counting() {
    let (conn, a, b) = setup();
    create_transfer(&conn, a, b, "300", d("2026-03-05"), "").unwrap();
    let s = month_summary(&conn, d("2026-03-01")).unwrap();
    assert_eq!((s.income.value, s.expense.value), (0, 0));
    assert_eq!(s.accounts_total, fig(1500, 1500));
    assert!(s.by_category.is_empty());
    assert_eq!(s.recent.len(), 1);
}

#[test]
fn archived_accounts_are_left_out_of_the_total() {
    let (conn, a, b) = setup();
    create_transfer(&conn, a, b, "300", d("2026-03-05"), "").unwrap();
    tx(&conn, TxKind::Expense, b, "2026-03-06", "50");
    set_account_archived(&conn, b, true).unwrap();
    let s = month_summary(&conn, d("2026-03-01")).unwrap();
    // Only A: 1000 - 300 sent to the archived B.
    assert_eq!(s.accounts_total, fig(700, 1000));
    // The expense of the archived account is still an expense.
    assert_eq!(s.expense.value, 50);
}

#[test]
fn end_of_month_cutoff() {
    let (conn, a, _) = setup();
    tx(&conn, TxKind::Expense, a, "2026-03-31", "100");
    tx(&conn, TxKind::Expense, a, "2026-04-01", "7");
    let s = month_summary(&conn, d("2026-03-01")).unwrap();
    assert_eq!(s.accounts_total, fig(1400, 1500));
    assert_eq!(s.expense, fig(100, 0));
}

#[test]
fn bars_are_relative_to_the_biggest() {
    let (conn, a, _) = setup();
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    let fun = create_category(&conn, TxKind::Expense, "Fun").unwrap();
    let spend = |date, amount, c| {
        create_transaction(&conn, TxKind::Expense, a, amount, d(date), "", c).unwrap();
    };
    spend("2026-03-02", "40", Some(food));
    spend("2026-03-03", "10", Some(fun));
    spend("2026-03-04", "20", None);
    let bars = month_summary(&conn, d("2026-03-01")).unwrap().by_category;
    let seen: Vec<_> = bars
        .iter()
        .map(|b| (b.category, b.name.as_deref(), b.total, b.width))
        .collect();
    assert_eq!(
        seen,
        [
            (Some(food), Some("Food"), 40, 1.0),
            (None, None, 20, 0.5),
            (Some(fun), Some("Fun"), 10, 0.25),
        ]
    );
}

#[test]
fn recent_is_five_newest_first() {
    let (conn, a, _) = setup();
    for day in 1..=7 {
        tx(
            &conn,
            TxKind::Expense,
            a,
            &format!("2026-03-{day:02}"),
            &day.to_string(),
        );
    }
    tx(&conn, TxKind::Expense, a, "2026-04-01", "9");
    let s = month_summary(&conn, d("2026-03-01")).unwrap();
    let days: Vec<_> = s
        .recent
        .iter()
        .map(|m| match m {
            Movement::Tx(t) => t.date.format("%d").to_string(),
            Movement::Transfer(_) => unreachable!(),
        })
        .collect();
    assert_eq!(days, ["07", "06", "05", "04", "03"]);
}

#[test]
fn empty_month_gives_zeros() {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let s = month_summary(&conn, d("2026-03-01")).unwrap();
    let zero = fig(0, 0);
    assert_eq!(
        s,
        MonthSummary {
            income: zero,
            expense: zero,
            balance: zero,
            accounts_total: zero,
            by_category: Vec::new(),
            recent: Vec::new(),
        }
    );
    assert_eq!(zero.change(), 0);
}

#[test]
fn sums_saturate() {
    let (conn, a, _) = setup();
    for _ in 0..2 {
        conn.execute(
            "INSERT INTO finance_transaction (kind, account_id, amount, date)
             VALUES ('income', ?1, ?2, '2026-03-05')",
            (a.0, i64::MAX),
        )
        .unwrap();
    }
    let s = month_summary(&conn, d("2026-03-01")).unwrap();
    assert_eq!(s.income.value, i64::MAX);
    assert_eq!(s.accounts_total.value, i64::MAX);
    assert_eq!(s.accounts_total.change(), i64::MAX - 1500);
    assert_eq!(fig(i64::MIN, i64::MAX).change(), i64::MIN);
}
