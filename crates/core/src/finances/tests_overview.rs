// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for what the Summary tab draws.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::tests::account;
use super::*;
use crate::db;
use crate::recurrence::Frequency;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup() -> (Connection, AccountId, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "A", AccountKind::Cash, "", "1000").unwrap();
    let b = account(&conn, "B", AccountKind::Cash, "", "500").unwrap();
    (conn, a, b)
}

fn tx(conn: &Connection, kind: TxKind, a: AccountId, date: &str, amount: &str) {
    create_transaction(conn, kind, a, amount, d(date), "", None).unwrap();
}

#[test]
fn percent_rounds_half_away_from_zero_on_the_absolute_base() {
    assert_eq!(percent_change(150, 100), Some(50));
    assert_eq!(percent_change(1, 3), Some(-67));
    assert_eq!(percent_change(50, -100), Some(150));
    assert_eq!(percent_change(-50, 100), Some(-150));
    assert_eq!(percent_change(5, 0), None);
    assert_eq!(percent_change(0, 0), None);
}

#[test]
fn cards_carry_the_running_value_per_day_up_to_today() {
    let (conn, a, b) = setup();
    tx(&conn, TxKind::Income, a, "2026-11-02", "300");
    tx(&conn, TxKind::Expense, a, "2026-11-04", "100");
    create_transfer(&conn, a, b, "50", "", d("2026-11-03"), "").unwrap();
    tx(&conn, TxKind::Income, a, "2026-10-20", "100");
    tx(&conn, TxKind::Expense, a, "2026-10-21", "40");
    let c = month_cards(&conn, d("2026-11-09"), d("2026-11-05")).unwrap();
    assert_eq!((c.shown, c.span), (5, 31));
    assert_eq!(c.income.series, [0, 300, 300, 300, 300]);
    assert_eq!(c.expense.series, [0, 0, 0, 100, 100]);
    assert_eq!(c.balance.series, [0, 300, 300, 200, 200]);
    // The whole month counts in the value, whatever today is.
    assert_eq!(
        (c.income.value, c.expense.value, c.balance.value),
        (300, 100, 200)
    );
    assert_eq!(c.income.percent, Some(200));
    assert_eq!(c.expense.percent, Some(150));
    assert_eq!(c.balance.percent, Some(233));
    // Opening 1500 + 100 - 40 in October; the transfer between active accounts nets zero.
    assert_eq!(c.total.series, [1560, 1860, 1860, 1760, 1760]);
    assert_eq!(c.total.value, 1760);
    assert_eq!(c.total.percent, None);
    // The month before: all its days, cumulative.
    assert_eq!(c.previous.series.len(), 31);
    assert_eq!((c.previous.series[19], c.previous.series[30]), (100, 60));
    assert_eq!(c.balance_chart().span, 31);
}

#[test]
fn a_past_month_shows_all_its_days_and_no_percent_without_a_base() {
    let (conn, a, _) = setup();
    tx(&conn, TxKind::Income, a, "2026-02-10", "70");
    let c = month_cards(&conn, d("2026-02-01"), d("2026-11-05")).unwrap();
    assert_eq!((c.shown, c.span), (28, 31));
    assert_eq!(c.income.series.len(), 28);
    assert_eq!(c.income.percent, None);
}

#[test]
fn archived_accounts_leave_the_total_series() {
    let (conn, a, b) = setup();
    create_transfer(&conn, a, b, "300", "", d("2026-11-02"), "").unwrap();
    set_account_archived(&conn, b, true).unwrap();
    let c = month_cards(&conn, d("2026-11-01"), d("2026-11-03")).unwrap();
    assert_eq!(c.total.series, [1000, 700, 700]);
}

#[test]
fn donut_keeps_five_slices_and_sums_the_rest() {
    let (conn, a, _) = setup();
    for (i, amount) in [60, 50, 40, 30, 20, 10, 5].into_iter().enumerate() {
        let cat = create_category(&conn, TxKind::Expense, &format!("C{i}")).unwrap();
        create_transaction(
            &conn,
            TxKind::Expense,
            a,
            &amount.to_string(),
            d("2026-11-02"),
            "",
            Some(cat),
        )
        .unwrap();
    }
    let b = category_breakdown(&conn, d("2026-11-10"), TxKind::Expense).unwrap();
    assert_eq!(b.total, 215);
    assert_eq!(b.slices.len(), 6);
    let last = &b.slices[5];
    assert!(last.others && last.name.is_none());
    assert_eq!((last.value, last.percent), (15, 7));
    assert_eq!(b.slices[0].percent, 28);
    let share: f32 = b.slices.iter().map(|s| s.share).sum();
    assert!((share - 1.0).abs() < 1e-4);
    // Income has its own breakdown, empty here.
    let none = category_breakdown(&conn, d("2026-11-10"), TxKind::Income).unwrap();
    assert_eq!((none.total, none.slices.len()), (0, 0));
}

#[test]
fn recent_movements_keep_the_newest_of_the_filter() {
    let (conn, a, _) = setup();
    for day in 1..=7 {
        tx(&conn, TxKind::Expense, a, &format!("2026-11-{day:02}"), "1");
    }
    tx(&conn, TxKind::Income, a, "2026-11-01", "9");
    let all = recent_movements(&conn, d("2026-11-01"), MovementFilter::All, 5).unwrap();
    assert_eq!(all.len(), 5);
    let income = recent_movements(&conn, d("2026-11-01"), MovementFilter::Income, 5).unwrap();
    assert_eq!(income.len(), 1);
}

fn schedule(conn: &Connection, a: AccountId, first: &str, amount: &str, form: ScheduleForm) {
    create_scheduled(
        conn,
        TxKind::Expense,
        "Pago",
        a,
        amount,
        None,
        d(first),
        form,
    )
    .unwrap();
}

#[test]
fn upcoming_totals_four_months_and_lists_pending_by_date() {
    let (conn, a, _) = setup();
    schedule(&conn, a, "2026-11-20", "100", ScheduleForm::Once);
    schedule(
        &conn,
        a,
        "2026-11-05",
        "10",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    schedule(&conn, a, "2026-12-01", "50", ScheduleForm::Installments(2));
    create_scheduled(
        &conn,
        TxKind::Income,
        "Sueldo",
        a,
        "999",
        None,
        d("2026-11-25"),
        ScheduleForm::Once,
    )
    .unwrap();
    let u = upcoming_payments(&conn, d("2026-11-14"), UpcomingFilter::All, 5).unwrap();
    let totals: Vec<_> = u.months.iter().map(|m| m.total).collect();
    // November: 100 + 10 of the 5th (still pending); December and January: 10 + 50 installment.
    assert_eq!(totals, [110, 60, 60, 10]);
    assert_eq!(u.months[0].month, d("2026-11-01"));
    assert_eq!(u.months[0].share, 1.0);
    assert!((u.months[1].share - 60.0 / 110.0).abs() < 1e-6);
    let dates: Vec<_> = u.items.iter().map(|i| i.date).collect();
    assert_eq!(dates.len(), 5);
    assert!(dates.windows(2).all(|w| w[0] <= w[1]));
    assert!(u.items.iter().all(|i| !i.bill));

    let rec = upcoming_payments(&conn, d("2026-11-14"), UpcomingFilter::Recurring, 5).unwrap();
    assert_eq!(rec.months[0].total, 10);
    let inst = upcoming_payments(&conn, d("2026-11-14"), UpcomingFilter::Installments, 5).unwrap();
    assert_eq!(inst.items.len(), 2);
    let bills = upcoming_payments(&conn, d("2026-11-14"), UpcomingFilter::Bills, 5).unwrap();
    assert!(bills.items.is_empty());
    assert!(bills.months.iter().all(|m| m.total == 0 && m.share == 0.0));
}

#[test]
fn account_filter_splits_credit_from_the_rest() {
    use AccountKind::*;
    for kind in [Checking, Demand, Savings, Cash] {
        assert!(AccountFilter::All.keeps(kind) && AccountFilter::Accounts.keeps(kind));
        assert!(!AccountFilter::Credit.keeps(kind));
    }
    for kind in [Credit, CreditLine] {
        assert!(AccountFilter::All.keeps(kind) && AccountFilter::Credit.keeps(kind));
        assert!(!AccountFilter::Accounts.keeps(kind));
    }
}
