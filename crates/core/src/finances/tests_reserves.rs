// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for reserves: sub-balances that lower an account's available, not its balance.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

const D: &str = "2026-10-10";

fn setup(balance: &str) -> (Connection, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "Cta", AccountKind::Checking, "", balance).unwrap();
    (conn, a)
}

fn acc(conn: &Connection, id: AccountId) -> Account {
    get_account(conn, id).unwrap()
}

fn put(conn: &Connection, r: ReserveId, amount: &str) -> Result<(), FinanceError> {
    reserve_money(conn, r, amount, d(D), "")
}

fn take(conn: &Connection, r: ReserveId, amount: &str) -> Result<(), FinanceError> {
    withdraw_from_reserve(conn, r, amount, d(D), "")
}

#[test]
fn create_needs_a_name_and_a_plain_active_account() {
    let (conn, a) = setup("1000");
    assert!(matches!(
        create_reserve(&conn, "  ", a),
        Err(FinanceError::EmptyName)
    ));
    assert!(matches!(
        create_reserve(&conn, "X", AccountId(99)),
        Err(FinanceError::AccountNotFound)
    ));
    let card = create_card(&conn, "Visa", "", "CLP", "1000", 5, 20, None).unwrap();
    let line = create_credit_line(&conn, "Línea", "", "CLP", "0", "1000", None).unwrap();
    for c in [card, line] {
        assert!(matches!(
            create_reserve(&conn, "X", c),
            Err(FinanceError::KindLocked)
        ));
    }
    let other = account(&conn, "Vieja", AccountKind::Cash, "", "0").unwrap();
    set_account_archived(&conn, other, true).unwrap();
    assert!(matches!(
        create_reserve(&conn, "X", other),
        Err(FinanceError::AccountArchived)
    ));
    let id = create_reserve(&conn, " Emergencias ", a).unwrap();
    let list = list_reserves(&conn).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].id, list[0].name.as_str(), list[0].balance),
        (id, "Emergencias", 0)
    );
    assert_eq!(list[0].account_name, "Cta");
}

#[test]
fn reserving_is_limited_by_the_available_and_withdrawing_by_the_balance() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Ahorro", a).unwrap();
    for bad in ["0", "", "abc", "-5"] {
        assert!(matches!(
            put(&conn, r, bad),
            Err(FinanceError::InvalidAmount)
        ));
        assert!(matches!(
            take(&conn, r, bad),
            Err(FinanceError::InvalidAmount)
        ));
    }
    assert!(matches!(
        put(&conn, r, "1001"),
        Err(FinanceError::Insufficient)
    ));
    assert!(matches!(
        put(&conn, ReserveId(99), "10"),
        Err(FinanceError::ReserveNotFound)
    ));
    put(&conn, r, "1000").unwrap();
    assert!(matches!(
        put(&conn, r, "1"),
        Err(FinanceError::Insufficient)
    ));
    assert!(matches!(
        take(&conn, r, "1001"),
        Err(FinanceError::Insufficient)
    ));
    take(&conn, r, "400").unwrap();
    assert_eq!(list_reserves(&conn).unwrap()[0].balance, 600);
    take(&conn, r, "600").unwrap();
    assert_eq!(list_reserves(&conn).unwrap()[0].balance, 0);
}

#[test]
fn available_is_the_balance_minus_the_reserves_and_no_movement_is_made() {
    let (conn, a) = setup("1000");
    let r1 = create_reserve(&conn, "Uno", a).unwrap();
    let r2 = create_reserve(&conn, "Dos", a).unwrap();
    put(&conn, r1, "300").unwrap();
    put(&conn, r2, "200").unwrap();
    let x = acc(&conn, a);
    assert_eq!((x.balance, x.reserved, x.available()), (1000, 500, 500));
    // the limit is the account's available, not the reserve's
    assert!(matches!(
        put(&conn, r1, "501"),
        Err(FinanceError::Insufficient)
    ));
    take(&conn, r1, "100").unwrap();
    let x = acc(&conn, a);
    assert_eq!((x.reserved, x.available()), (400, 600));
    assert!(!x.in_use);
}

#[test]
fn available_goes_negative_when_spending_below_the_reserved() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    put(&conn, r, "800").unwrap();
    create_transaction(&conn, TxKind::Expense, a, "500", d(D), "", None).unwrap();
    let x = acc(&conn, a);
    assert_eq!((x.balance, x.available()), (500, -300));
    take(&conn, r, "800").unwrap();
    assert_eq!(acc(&conn, a).available(), 500);
}

#[test]
fn rename_needs_a_name() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    assert!(matches!(
        rename_reserve(&conn, r, " "),
        Err(FinanceError::EmptyName)
    ));
    assert!(matches!(
        rename_reserve(&conn, ReserveId(99), "X"),
        Err(FinanceError::ReserveNotFound)
    ));
    rename_reserve(&conn, r, " Dos ").unwrap();
    assert_eq!(list_reserves(&conn).unwrap()[0].name, "Dos");
}

#[test]
fn archive_needs_zero_balance_and_restore_brings_it_back() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    put(&conn, r, "300").unwrap();
    assert!(matches!(
        set_reserve_archived(&conn, r, true),
        Err(FinanceError::ReserveNotEmpty)
    ));
    take(&conn, r, "300").unwrap();
    set_reserve_archived(&conn, r, true).unwrap();
    assert!(list_reserves(&conn).unwrap().is_empty());
    assert_eq!(list_archived_reserves(&conn).unwrap()[0].id, r);
    assert!(matches!(
        put(&conn, r, "10"),
        Err(FinanceError::ReserveNotFound)
    ));
    set_reserve_archived(&conn, r, false).unwrap();
    assert!(list_archived_reserves(&conn).unwrap().is_empty());
    put(&conn, r, "10").unwrap();
}

#[test]
fn delete_only_without_moves() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    put(&conn, r, "10").unwrap();
    take(&conn, r, "10").unwrap();
    assert!(matches!(
        delete_reserve(&conn, r),
        Err(FinanceError::ReserveInUse)
    ));
    let empty = create_reserve(&conn, "Dos", a).unwrap();
    delete_reserve(&conn, empty).unwrap();
    assert!(matches!(
        delete_reserve(&conn, empty),
        Err(FinanceError::ReserveNotFound)
    ));
    assert_eq!(list_reserves(&conn).unwrap().len(), 1);
}

#[test]
fn moves_come_most_recent_first() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    assert!(list_moves(&conn, r).unwrap().is_empty());
    reserve_money(&conn, r, "500", d("2026-10-05"), "primero").unwrap();
    reserve_money(&conn, r, "100", d("2026-10-12"), "último").unwrap();
    withdraw_from_reserve(&conn, r, "200", d("2026-10-12"), " mismo día ").unwrap();
    let seen: Vec<_> = list_moves(&conn, r)
        .unwrap()
        .into_iter()
        .map(|m| (m.date.to_string(), m.description, m.withdraw, m.amount))
        .collect();
    let row = |a: &str, b: &str, c, e| (a.to_string(), b.to_string(), c, e);
    assert_eq!(
        seen,
        vec![
            row("2026-10-12", "mismo día", true, 200),
            row("2026-10-12", "último", false, 100),
            row("2026-10-05", "primero", false, 500),
        ]
    );
}

#[test]
fn an_archived_account_keeps_its_reserves_but_takes_no_deposits() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    put(&conn, r, "100").unwrap();
    // reserves with money block archiving the account
    assert!(matches!(
        set_account_archived(&conn, a, true),
        Err(FinanceError::ReserveNotEmpty)
    ));
    take(&conn, r, "100").unwrap();
    set_account_archived(&conn, a, true).unwrap();
    assert_eq!(list_reserves(&conn).unwrap().len(), 1);
    assert!(list_reserves(&conn).unwrap()[0].account_archived);
    assert!(matches!(
        put(&conn, r, "10"),
        Err(FinanceError::AccountArchived)
    ));
}

#[test]
fn an_account_with_reserves_cannot_be_deleted_nor_change_currency() {
    let (conn, a) = setup("1000");
    let r = create_reserve(&conn, "Uno", a).unwrap();
    assert!(matches!(
        delete_account(&conn, a),
        Err(FinanceError::AccountInUse)
    ));
    assert!(matches!(
        update_account(
            &conn,
            a,
            "Cta",
            AccountKind::Checking,
            "",
            "USD",
            "1000",
            Some("900")
        ),
        Err(FinanceError::CurrencyLocked)
    ));
    update_account(
        &conn,
        a,
        "Otra",
        AccountKind::Savings,
        "",
        "CLP",
        "1000",
        None,
    )
    .unwrap();
    delete_reserve(&conn, r).unwrap();
    delete_account(&conn, a).unwrap();
}

#[test]
fn amounts_are_read_in_the_accounts_currency() {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let usd = create_account(
        &conn,
        "USD",
        AccountKind::Cash,
        "",
        "USD",
        "100.50",
        Some("900"),
    )
    .unwrap();
    let r = create_reserve(&conn, "Viaje", usd).unwrap();
    put(&conn, r, "20.25").unwrap();
    assert_eq!(list_reserves(&conn).unwrap()[0].balance, 2025);
    assert_eq!(acc(&conn, usd).available(), 8025);
}
