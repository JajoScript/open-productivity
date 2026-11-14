// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the budget: buckets, shares by month and spending against them.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// CLP main, a CLP account and a USD account (1 USD = 1000 CLP).
fn setup() -> (Connection, AccountId, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let clp = account(&conn, "CLP", AccountKind::Cash, "", "0").unwrap();
    let usd = create_account(
        &conn,
        "USD",
        AccountKind::Cash,
        "",
        "USD",
        "0",
        Some("1000"),
    )
    .unwrap();
    (conn, clp, usd)
}

fn tx(conn: &Connection, kind: TxKind, a: AccountId, date: &str, amount: &str) {
    create_transaction(conn, kind, a, amount, d(date), "", None).unwrap();
}

/// Three buckets A, B, C.
fn buckets(conn: &Connection) -> [BucketId; 3] {
    ["A", "B", "C"].map(|n| create_bucket(conn, n).unwrap())
}

fn percents(conn: &Connection, month: &str) -> Vec<i64> {
    budget_summary(conn, d(month))
        .unwrap()
        .buckets
        .iter()
        .map(|b| b.percent)
        .collect()
}

fn share(conn: &Connection, month: &str, b: BucketId, p: i64) {
    set_share(conn, d(month), b, p).unwrap();
}

fn rows(conn: &Connection, month: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM finance_budget_share WHERE month = ?1",
        [month],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn base_sums_income_in_both_currencies_without_transfers() {
    let (conn, clp, usd) = setup();
    tx(&conn, TxKind::Income, clp, "2026-10-03", "500000");
    tx(&conn, TxKind::Income, usd, "2026-10-04", "200");
    tx(&conn, TxKind::Expense, clp, "2026-10-05", "90000");
    tx(&conn, TxKind::Income, clp, "2026-09-30", "7000");
    create_transfer(&conn, clp, usd, "100000", "100", d("2026-10-06"), "").unwrap();
    let s = budget_summary(&conn, d("2026-10-17")).unwrap();
    assert_eq!((s.base, s.base_month), (700_000, d("2026-10-01")));
}

#[test]
fn base_follows_the_setting_and_crosses_the_year() {
    let (conn, clp, _) = setup();
    tx(&conn, TxKind::Income, clp, "2025-12-10", "1000");
    tx(&conn, TxKind::Income, clp, "2026-01-10", "2000");
    tx(&conn, TxKind::Income, clp, "2026-02-10", "4000");
    let [a, ..] = buckets(&conn);
    share(&conn, "2025-01-01", a, 50);
    assert_eq!(budget_base(&conn).unwrap(), BudgetBase::Current);
    let s = budget_summary(&conn, d("2026-01-01")).unwrap();
    assert_eq!((s.base, s.base_month), (2000, d("2026-01-01")));
    set_budget_base(&conn, BudgetBase::Previous).unwrap();
    assert_eq!(budget_base(&conn).unwrap(), BudgetBase::Previous);
    let s = budget_summary(&conn, d("2026-01-01")).unwrap();
    assert_eq!((s.base, s.base_month), (1000, d("2025-12-01")));
    assert_eq!((s.buckets[0].assigned, s.unassigned), (500, 500));
    assert_eq!(budget_summary(&conn, d("2026-02-01")).unwrap().base, 2000);
    set_budget_base(&conn, BudgetBase::Current).unwrap();
    assert_eq!(budget_summary(&conn, d("2026-02-01")).unwrap().base, 4000);
}

#[test]
fn assigned_rounds_down_and_the_rest_is_unassigned() {
    let (conn, clp, _) = setup();
    tx(&conn, TxKind::Income, clp, "2026-10-03", "1001");
    let [a, b, c] = buckets(&conn);
    share(&conn, "2026-10-01", a, 50);
    share(&conn, "2026-10-01", b, 30);
    share(&conn, "2026-10-01", c, 20);
    let s = budget_summary(&conn, d("2026-10-01")).unwrap();
    let got: Vec<_> = s.buckets.iter().map(|b| (b.percent, b.assigned)).collect();
    assert_eq!(got, [(50, 500), (30, 300), (20, 200)]);
    assert_eq!(s.unassigned, 1);
    share(&conn, "2026-10-01", c, 0);
    let s = budget_summary(&conn, d("2026-10-01")).unwrap();
    assert_eq!((s.buckets[2].assigned, s.unassigned), (0, 201));
}

#[test]
fn a_month_inherits_the_latest_earlier_one() {
    let (conn, ..) = setup();
    let [a, b, c] = buckets(&conn);
    assert_eq!(percents(&conn, "2026-10-01"), [0, 0, 0]);
    for (x, p) in [(a, 60), (b, 20), (c, 10)] {
        share(&conn, "2026-08-01", x, p);
    }
    assert_eq!(percents(&conn, "2026-08-15"), [60, 20, 10]);
    assert_eq!(percents(&conn, "2026-10-01"), [60, 20, 10]);
    assert_eq!(percents(&conn, "2026-07-01"), [0, 0, 0]);
}

#[test]
fn editing_a_month_copies_the_inherited_one_and_leaves_earlier_months() {
    let (conn, ..) = setup();
    let [a, b, c] = buckets(&conn);
    for (x, p) in [(a, 60), (b, 20), (c, 10)] {
        share(&conn, "2026-08-01", x, p);
    }
    share(&conn, "2026-10-01", a, 10);
    // the other buckets came along from August
    assert_eq!(percents(&conn, "2026-10-01"), [10, 20, 10]);
    assert_eq!(percents(&conn, "2026-09-01"), [60, 20, 10]);
    assert_eq!(percents(&conn, "2026-08-01"), [60, 20, 10]);
    // later months without rows of their own inherit the edit
    assert_eq!(percents(&conn, "2026-12-01"), [10, 20, 10]);
    assert_eq!(
        (rows(&conn, "2026-10-01"), rows(&conn, "2026-12-01")),
        (3, 0)
    );
    // a second edit in the same month does not copy again
    share(&conn, "2026-10-01", b, 30);
    assert_eq!(percents(&conn, "2026-10-01"), [10, 30, 10]);
}

#[test]
fn editing_a_month_keeps_the_archived_buckets_rows() {
    // Regression: the legacy screen rewrote the month from the active buckets only.
    let (conn, ..) = setup();
    let [a, b, c] = buckets(&conn);
    for (x, p) in [(a, 50), (b, 30), (c, 20)] {
        share(&conn, "2026-08-01", x, p);
    }
    set_bucket_archived(&conn, c, true).unwrap();
    share(&conn, "2026-10-01", a, 40);
    assert_eq!(percents(&conn, "2026-10-01"), [40, 30]);
    let kept: i64 = conn
        .query_row(
            "SELECT percent FROM finance_budget_share WHERE month = '2026-10-01' AND bucket_id = ?1",
            [c.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 20);
    set_bucket_archived(&conn, c, false).unwrap();
    assert_eq!(percents(&conn, "2026-10-01"), [40, 30, 20]);
}

#[test]
fn an_archived_bucket_leaves_the_list_and_its_share_is_unassigned() {
    let (conn, clp, _) = setup();
    tx(&conn, TxKind::Income, clp, "2026-10-03", "1000");
    let [a, b, _] = buckets(&conn);
    share(&conn, "2026-10-01", a, 50);
    share(&conn, "2026-10-01", b, 30);
    set_bucket_archived(&conn, b, true).unwrap();
    let s = budget_summary(&conn, d("2026-10-01")).unwrap();
    assert_eq!(s.buckets.len(), 2);
    assert_eq!(s.unassigned, 500);
    // archived buckets do not count toward the 100
    share(&conn, "2026-10-01", a, 100);
}

#[test]
fn invalid_shares_are_rejected_and_nothing_changes() {
    let (conn, ..) = setup();
    let [a, b, c] = buckets(&conn);
    share(&conn, "2026-08-01", a, 60);
    share(&conn, "2026-08-01", b, 30);
    for p in [-1, 101] {
        assert!(matches!(
            set_share(&conn, d("2026-10-01"), a, p),
            Err(FinanceError::InvalidShares)
        ));
    }
    // 60 + 30 + 11 > 100, in a month that would first copy August
    assert!(matches!(
        set_share(&conn, d("2026-10-01"), c, 11),
        Err(FinanceError::InvalidShares)
    ));
    assert_eq!(rows(&conn, "2026-10-01"), 0);
    assert_eq!(percents(&conn, "2026-10-01"), [60, 30, 0]);
    assert!(matches!(
        set_share(&conn, d("2026-10-01"), BucketId(99), 1),
        Err(FinanceError::BucketNotFound)
    ));
    share(&conn, "2026-10-01", c, 10);
}

#[test]
fn spent_counts_only_the_months_expenses_in_a_bucket() {
    let (conn, clp, usd) = setup();
    tx(&conn, TxKind::Income, clp, "2026-10-01", "1000000");
    let [food_b, _, fun_b] = buckets(&conn);
    share(&conn, "2026-10-01", food_b, 50);
    share(&conn, "2026-10-01", fun_b, 20);
    let cat = |n| create_category(&conn, TxKind::Expense, n).unwrap();
    let (food, fun, loose) = (cat("Food"), cat("Fun"), cat("Loose"));
    set_category_bucket(&conn, food, Some(food_b)).unwrap();
    set_category_bucket(&conn, fun, Some(fun_b)).unwrap();
    let spend = |a, date, amount, c| {
        create_transaction(&conn, TxKind::Expense, a, amount, d(date), "", c).unwrap();
    };
    spend(clp, "2026-10-05", "100000", Some(food));
    spend(usd, "2026-10-06", "200", Some(food)); // 200.000
    spend(clp, "2026-10-07", "50000", Some(loose)); // category without bucket
    spend(clp, "2026-10-07", "40000", None); // no category
    spend(clp, "2026-09-30", "30000", Some(food)); // other month
    spend(clp, "2026-10-08", "250000", Some(fun)); // over the 200.000 assigned
    let s = budget_summary(&conn, d("2026-10-01")).unwrap();
    let got: Vec<_> = s
        .buckets
        .iter()
        .map(|b| (b.spent, b.remaining, b.overspent, b.spent_per_mille))
        .collect();
    assert_eq!(
        got,
        [
            (300_000, 200_000, false, 600),
            (0, 0, false, 0),
            (250_000, -50_000, true, 1250)
        ]
    );
    assert_eq!(
        budget_summary(&conn, d("2026-09-01")).unwrap().buckets[0].spent,
        30_000
    );
}

#[test]
fn spending_in_a_bucket_with_nothing_assigned_is_overspent() {
    let (conn, clp, _) = setup();
    let [a, ..] = buckets(&conn);
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    set_category_bucket(&conn, food, Some(a)).unwrap();
    create_transaction(
        &conn,
        TxKind::Expense,
        clp,
        "10",
        d("2026-10-05"),
        "",
        Some(food),
    )
    .unwrap();
    let b = &budget_summary(&conn, d("2026-10-01")).unwrap().buckets[0];
    assert_eq!(
        (b.assigned, b.remaining, b.overspent, b.spent_per_mille),
        (0, -10, true, 0)
    );
}

#[test]
fn category_bucket_only_for_expense_categories() {
    let (conn, ..) = setup();
    let [a, b, _] = buckets(&conn);
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    let pay = create_category(&conn, TxKind::Income, "Pay").unwrap();
    let bucket_of = |c| get_category(&conn, c).unwrap().bucket;
    assert_eq!(bucket_of(food), None);
    set_category_bucket(&conn, food, Some(a)).unwrap();
    assert_eq!(bucket_of(food), Some(a));
    set_category_bucket(&conn, food, Some(b)).unwrap(); // moves
    assert_eq!(bucket_of(food), Some(b));
    set_category_bucket(&conn, food, None).unwrap();
    assert_eq!(bucket_of(food), None);
    assert!(matches!(
        set_category_bucket(&conn, pay, Some(a)),
        Err(FinanceError::CategoryKindMismatch)
    ));
    assert!(matches!(
        set_category_bucket(&conn, food, Some(BucketId(99))),
        Err(FinanceError::BucketNotFound)
    ));
    assert!(matches!(
        set_category_bucket(&conn, CategoryId(99), None),
        Err(FinanceError::CategoryNotFound)
    ));
}

#[test]
fn buckets_are_created_renamed_and_listed_active_first() {
    let (conn, ..) = setup();
    assert!(matches!(
        create_bucket(&conn, "  "),
        Err(FinanceError::EmptyName)
    ));
    let a = create_bucket(&conn, " Needs ").unwrap();
    let b = create_bucket(&conn, "Wants").unwrap();
    rename_bucket(&conn, a, "Basics").unwrap();
    assert!(matches!(
        rename_bucket(&conn, a, " "),
        Err(FinanceError::EmptyName)
    ));
    assert!(matches!(
        rename_bucket(&conn, BucketId(99), "x"),
        Err(FinanceError::BucketNotFound)
    ));
    set_bucket_archived(&conn, a, true).unwrap();
    let got: Vec<_> = list_buckets(&conn)
        .unwrap()
        .into_iter()
        .map(|x| (x.id, x.name, x.archived))
        .collect();
    assert_eq!(
        got,
        [(b, "Wants".into(), false), (a, "Basics".into(), true)]
    );
    assert!(matches!(
        set_bucket_archived(&conn, BucketId(99), true),
        Err(FinanceError::BucketNotFound)
    ));
}

#[test]
fn delete_needs_a_bucket_without_categories_and_takes_its_shares() {
    let (conn, ..) = setup();
    let [a, b, _] = buckets(&conn);
    share(&conn, "2026-10-01", a, 10);
    share(&conn, "2026-10-01", b, 10);
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    set_category_bucket(&conn, food, Some(a)).unwrap();
    assert!(list_buckets(&conn).unwrap()[0].in_use);
    assert!(matches!(
        delete_bucket(&conn, a),
        Err(FinanceError::BucketInUse)
    ));
    delete_bucket(&conn, b).unwrap();
    assert_eq!(rows(&conn, "2026-10-01"), 1);
    set_category_bucket(&conn, food, None).unwrap();
    delete_bucket(&conn, a).unwrap();
    assert_eq!(rows(&conn, "2026-10-01"), 0);
    assert!(matches!(
        delete_bucket(&conn, a),
        Err(FinanceError::BucketNotFound)
    ));
}

#[test]
fn suggested_budget_is_50_30_20_and_only_for_an_empty_budget() {
    let (conn, clp, _) = setup();
    tx(&conn, TxKind::Income, clp, "2026-10-03", "1000");
    create_suggested_budget(&conn, d("2026-10-17"), ["Needs", "Wants", "Savings"]).unwrap();
    let s = budget_summary(&conn, d("2026-10-01")).unwrap();
    let got: Vec<_> = s
        .buckets
        .iter()
        .map(|b| (b.name.as_str(), b.percent, b.assigned))
        .collect();
    assert_eq!(
        got,
        [("Needs", 50, 500), ("Wants", 30, 300), ("Savings", 20, 200)]
    );
    assert_eq!(percents(&conn, "2026-09-01"), [0, 0, 0]);
    assert!(matches!(
        create_suggested_budget(&conn, d("2026-10-01"), ["a", "b", "c"]),
        Err(FinanceError::BucketsExist)
    ));
    // a failure creates nothing
    let conn2 = db::test_conn();
    assert!(create_suggested_budget(&conn2, d("2026-10-01"), ["a", " ", "c"]).is_err());
    assert!(list_buckets(&conn2).unwrap().is_empty());
}
