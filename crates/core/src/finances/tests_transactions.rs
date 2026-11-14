// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for expenses and income.

use chrono::NaiveDate;

use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup(code: &str) -> (rusqlite::Connection, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, code).unwrap();
    let a = create_account(&conn, "A", AccountKind::Cash, "", "1000").unwrap();
    (conn, a)
}

fn balance(conn: &rusqlite::Connection, a: AccountId) -> i64 {
    get_account(conn, a).unwrap().balance
}

#[test]
fn expense_and_income_move_the_balance() {
    let (conn, a) = setup("CLP");
    let e = create_transaction(
        &conn,
        TxKind::Expense,
        a,
        "300",
        d("2026-03-05"),
        " Lunch ",
        None,
    )
    .unwrap();
    assert_eq!(balance(&conn, a), 700);
    create_transaction(&conn, TxKind::Income, a, "1.500", d("2026-03-06"), "", None).unwrap();
    assert_eq!(balance(&conn, a), 2200);
    assert_eq!(list_accounts(&conn).unwrap()[0].balance, 2200);
    let t = get_transaction(&conn, e).unwrap();
    assert_eq!(t.description, "Lunch");
    assert_eq!((t.amount, t.signed()), (300, -300));
    assert_eq!((t.account_name.as_str(), t.currency.code), ("A", "CLP"));
}

#[test]
fn invalid_amounts_and_missing_account() {
    let (conn, a) = setup("CLP");
    for bad in ["", "  ", "0", "-5", "abc", "1.2.3"] {
        assert!(matches!(
            create_transaction(&conn, TxKind::Expense, a, bad, d("2026-03-05"), "", None),
            Err(FinanceError::InvalidAmount)
        ));
    }
    assert!(matches!(
        create_transaction(
            &conn,
            TxKind::Expense,
            AccountId(99),
            "5",
            d("2026-03-05"),
            "",
            None
        ),
        Err(FinanceError::AccountNotFound)
    ));
    assert!(list_month(&conn, d("2026-03-01")).unwrap().is_empty());
}

#[test]
fn usd_amounts_and_future_dates() {
    let (conn, a) = setup("USD");
    create_transaction(
        &conn,
        TxKind::Expense,
        a,
        "12,34",
        d("2999-01-01"),
        "",
        None,
    )
    .unwrap();
    assert_eq!(balance(&conn, a), 100_000 - 1234);
    assert_eq!(list_month(&conn, d("2999-01-01")).unwrap().len(), 1);
}

#[test]
fn update_recalculates_both_accounts() {
    let (conn, a) = setup("CLP");
    let b = create_account(&conn, "B", AccountKind::Cash, "", "50").unwrap();
    let id =
        create_transaction(&conn, TxKind::Expense, a, "100", d("2026-03-05"), "x", None).unwrap();
    update_transaction(
        &conn,
        id,
        TxKind::Income,
        b,
        "30",
        d("2026-04-01"),
        " y ",
        None,
    )
    .unwrap();
    assert_eq!((balance(&conn, a), balance(&conn, b)), (1000, 80));
    let t = get_transaction(&conn, id).unwrap();
    assert_eq!(
        (
            t.kind,
            t.account_id,
            t.amount,
            t.date,
            t.description.as_str()
        ),
        (TxKind::Income, b, 30, d("2026-04-01"), "y")
    );
    assert!(matches!(
        update_transaction(&conn, id, TxKind::Income, b, "0", d("2026-04-01"), "", None),
        Err(FinanceError::InvalidAmount)
    ));
    assert!(matches!(
        update_transaction(
            &conn,
            id,
            TxKind::Income,
            AccountId(99),
            "1",
            d("2026-04-01"),
            "",
            None
        ),
        Err(FinanceError::AccountNotFound)
    ));
    assert!(matches!(
        update_transaction(
            &conn,
            TransactionId(99),
            TxKind::Income,
            b,
            "1",
            d("2026-04-01"),
            "",
            None
        ),
        Err(FinanceError::TransactionNotFound)
    ));
    assert_eq!(get_transaction(&conn, id).unwrap().amount, 30);
}

#[test]
fn delete_returns_the_balance_and_missing_fails() {
    let (conn, a) = setup("CLP");
    let id =
        create_transaction(&conn, TxKind::Expense, a, "100", d("2026-03-05"), "", None).unwrap();
    delete_transaction(&conn, id).unwrap();
    assert_eq!(balance(&conn, a), 1000);
    assert!(matches!(
        delete_transaction(&conn, id),
        Err(FinanceError::TransactionNotFound)
    ));
    assert!(matches!(
        get_transaction(&conn, id),
        Err(FinanceError::TransactionNotFound)
    ));
}

#[test]
fn list_month_bounds_and_order() {
    let (conn, a) = setup("CLP");
    let mk = |date: &str| {
        create_transaction(&conn, TxKind::Expense, a, "1", d(date), "", None)
            .unwrap()
            .0
    };
    mk("2026-11-30");
    let first = mk("2026-12-01");
    let mid = mk("2026-12-15");
    let mid2 = mk("2026-12-15");
    let last = mk("2026-12-31");
    mk("2027-01-01");
    // Any day of the month selects the whole month.
    let ids = |day: &str| -> Vec<i64> {
        list_month(&conn, d(day))
            .unwrap()
            .iter()
            .map(|m| match m {
                Movement::Tx(t) => t.id.0,
                Movement::Transfer(t) => t.id.0,
            })
            .collect()
    };
    assert_eq!(ids("2026-12-01"), [last, mid2, mid, first]);
    assert_eq!(ids("2026-12-20"), ids("2026-12-01"));
    assert_eq!(list_month(&conn, d("2027-01-01")).unwrap().len(), 1);
}

#[test]
fn unreadable_stored_values_are_errors() {
    let (conn, a) = setup("CLP");
    let id = create_transaction(&conn, TxKind::Expense, a, "1", d("2026-03-05"), "", None).unwrap();
    assert!(matches!(
        TxKind::parse("x"),
        Err(FinanceError::BadTxKind(_))
    ));
    for k in [TxKind::Expense, TxKind::Income] {
        assert_eq!(TxKind::parse(k.as_str()).unwrap(), k);
    }
    conn.execute_batch(
        "PRAGMA ignore_check_constraints = ON; UPDATE finance_transaction SET date = 'garbage'",
    )
    .unwrap();
    assert!(matches!(
        get_transaction(&conn, id),
        Err(FinanceError::BadDate(_))
    ));
}
