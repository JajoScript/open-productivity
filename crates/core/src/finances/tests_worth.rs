// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the net worth.

use super::tests::account;
use super::*;
use crate::db;

/// A cash account that an expense leaves in debt.
fn debt(conn: &rusqlite::Connection, name: &str, amount: &str) {
    let id = account(conn, name, AccountKind::Cash, "", "0").unwrap();
    create_transaction(
        conn,
        TxKind::Expense,
        id,
        amount,
        "2026-03-01".parse().unwrap(),
        "",
        None,
    )
    .unwrap();
}

fn setup() -> rusqlite::Connection {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    conn
}

#[test]
fn converts_sums_and_keeps_positives_as_slices() {
    let conn = setup();
    account(&conn, "Banco", AccountKind::Checking, "", "100000").unwrap();
    create_account(
        &conn,
        "Dólares",
        AccountKind::Cash,
        "",
        "USD",
        "100",
        Some("950"),
    )
    .unwrap();
    debt(&conn, "Deuda", "30000");
    let old = account(&conn, "Vieja", AccountKind::Cash, "", "999999").unwrap();
    set_account_archived(&conn, old, true).unwrap();

    let w = net_worth(&conn).unwrap();
    assert_eq!((w.assets, w.debts, w.total), (195_000, 30_000, 165_000));
    let slices: Vec<_> = w
        .slices
        .iter()
        .map(|s| (s.name.as_str(), s.value))
        .collect();
    assert_eq!(slices, [("Banco", 100_000), ("Dólares", 95_000)]);
}

#[test]
fn without_positive_accounts_there_are_no_slices() {
    let conn = setup();
    let w = net_worth(&conn).unwrap();
    assert_eq!((w.total, w.assets, w.debts, w.slices.len()), (0, 0, 0, 0));
    debt(&conn, "Deuda", "500");
    let w = net_worth(&conn).unwrap();
    assert_eq!(
        (w.total, w.assets, w.debts, w.slices.len()),
        (-500, 0, 500, 0)
    );
}

#[test]
fn cards_and_lines_count_as_debt() {
    let conn = setup();
    account(&conn, "Banco", AccountKind::Checking, "", "1000").unwrap();
    let card = create_card(&conn, "Visa", "", "CLP", "5000", 5, 20, None).unwrap();
    create_transaction(
        &conn,
        TxKind::Expense,
        card,
        "300",
        "2026-03-01".parse().unwrap(),
        "",
        None,
    )
    .unwrap();
    create_credit_line(&conn, "Línea", "", "CLP", "-200", "1000", None).unwrap();
    let w = net_worth(&conn).unwrap();
    assert_eq!((w.assets, w.debts, w.total), (1000, 500, 500));
    assert_eq!(w.slices.len(), 1);
}

#[test]
fn slices_are_ordered_with_relative_widths() {
    let conn = setup();
    account(&conn, "B", AccountKind::Cash, "", "50").unwrap();
    account(&conn, "A", AccountKind::Cash, "", "50").unwrap();
    account(&conn, "C", AccountKind::Cash, "", "200").unwrap();
    let w = net_worth(&conn).unwrap();
    let got: Vec<_> = w
        .slices
        .iter()
        .map(|s| (s.name.as_str(), s.width))
        .collect();
    assert_eq!(got, [("C", 1.0), ("A", 0.25), ("B", 0.25)]);
}

#[test]
fn a_missing_rate_is_an_error() {
    let conn = setup();
    create_account(&conn, "USD", AccountKind::Cash, "", "USD", "1", Some("900")).unwrap();
    conn.execute("DELETE FROM finance_rate", []).unwrap();
    assert!(matches!(
        net_worth(&conn),
        Err(FinanceError::MissingRate(_))
    ));
}

#[test]
fn slices_carry_their_share_and_rounded_percent() {
    let conn = setup();
    account(&conn, "Big", AccountKind::Cash, "", "9990").unwrap();
    account(&conn, "Mid", AccountKind::Cash, "", "5").unwrap();
    account(&conn, "Tiny", AccountKind::Cash, "", "1").unwrap();
    let w = net_worth(&conn).unwrap();
    let got: Vec<_> = w.slices.iter().map(|s| s.percent).collect();
    assert_eq!(got, [100, 0, 0]);
    assert!((w.slices[0].share - 0.9994).abs() < 1e-3);
    account(&conn, "Half", AccountKind::Cash, "", "10000").unwrap();
    let w = net_worth(&conn).unwrap();
    let got: Vec<_> = w.slices.iter().map(|s| s.percent).collect();
    assert_eq!(got, [50, 50, 0, 0]);
}
