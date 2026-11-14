// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for changing the main currency.

use rusqlite::Connection;

use super::tests::account;
use super::*;
use crate::db;

/// CLP main with a CLP account (10.000) and a USD account (100,00) at 1 USD = 987,24 CLP.
fn setup() -> Connection {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    account(&conn, "A", AccountKind::Cash, "", "10000").unwrap();
    create_account(
        &conn,
        "B",
        AccountKind::Cash,
        "",
        "USD",
        "100",
        Some("987,24"),
    )
    .unwrap();
    conn
}

fn total(conn: &Connection) -> i64 {
    month_summary(conn, "2026-03-01".parse().unwrap())
        .unwrap()
        .accounts_total
        .value
}

fn balances(conn: &Connection) -> Vec<i64> {
    list_accounts(conn)
        .unwrap()
        .iter()
        .map(|a| a.balance)
        .collect()
}

#[test]
fn suggests_rates_through_the_current_main() {
    let conn = setup();
    let r = main_change_rates(&conn, "USD").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].currency.code, "CLP");
    assert_eq!(r[0].rate, Some(1_013)); // 1 / 987,24 = 0,001013
    assert!(matches!(
        main_change_rates(&conn, "XXX"),
        Err(FinanceError::UnknownCurrency(_))
    ));
    // Back to the current main: only the other currency, at its stored rate.
    let r = main_change_rates(&conn, "CLP").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].currency.code, r[0].rate), ("USD", Some(987_240_000)));
}

#[test]
fn suggestion_is_none_without_a_rate_or_when_it_rounds_to_zero() {
    let conn = setup();
    conn.execute("DELETE FROM finance_rate", []).unwrap();
    assert_eq!(main_change_rates(&conn, "USD").unwrap()[0].rate, None);
    conn.execute(
        "INSERT INTO finance_rate (currency, rate) VALUES ('USD', 10000000000000)",
        [],
    )
    .unwrap();
    assert_eq!(main_change_rates(&conn, "USD").unwrap()[0].rate, None);
}

#[test]
fn changes_main_and_rates_in_one_go() {
    let conn = setup();
    assert_eq!(total(&conn), 10_000 + 98_724);
    let before = balances(&conn);
    change_main_currency(&conn, "USD", &[("CLP", "0,001013")]).unwrap();
    assert_eq!(main_currency(&conn).unwrap().unwrap().code, "USD");
    assert_eq!(balances(&conn), before);
    assert_eq!(total(&conn), 1_013 + 10_000);
    let rates = list_rates(&conn).unwrap();
    assert_eq!(
        (rates[0].currency.code, rates[0].rate),
        ("CLP", Some(1_013))
    );
}

#[test]
fn failures_change_nothing() {
    let conn = setup();
    for rates in [
        vec![],
        vec![("CLP", "  ")],
        vec![("CLP", "abc")],
        vec![("CLP", "0,001"), ("USD", "1")],
        vec![("CLP", "0,001"), ("XXX", "1")],
    ] {
        assert!(change_main_currency(&conn, "USD", &rates).is_err());
    }
    assert!(matches!(
        change_main_currency(&conn, "USD", &[]),
        Err(FinanceError::MissingRate(c)) if c == "CLP"
    ));
    assert!(matches!(
        change_main_currency(&conn, "USD", &[("CLP", "abc")]),
        Err(FinanceError::InvalidRate)
    ));
    assert!(matches!(
        change_main_currency(&conn, "XXX", &[]),
        Err(FinanceError::UnknownCurrency(_))
    ));
    assert_eq!(main_currency(&conn).unwrap().unwrap().code, "CLP");
    assert_eq!(total(&conn), 10_000 + 98_724);
}

#[test]
fn same_main_is_a_no_op() {
    let conn = setup();
    change_main_currency(&conn, "CLP", &[("USD", "1")]).unwrap();
    assert_eq!(total(&conn), 10_000 + 98_724);
}

#[test]
fn without_accounts_it_just_sets_the_main() {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    change_main_currency(&conn, "USD", &[]).unwrap();
    assert_eq!(main_currency(&conn).unwrap().unwrap().code, "USD");
    assert!(matches!(
        change_main_currency(&conn, "CLP", &[("USD", "1")]),
        Err(FinanceError::InvalidRate)
    ));
}
