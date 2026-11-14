// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for finances.

use super::*;
use crate::{db, instance_settings};

fn conn_with(code: &str) -> rusqlite::Connection {
    let conn = db::test_conn();
    set_main_currency(&conn, code).unwrap();
    conn
}

#[test]
fn migration_creates_table() {
    let conn = db::test_conn();
    assert!(list_accounts(&conn).unwrap().is_empty());
}

#[test]
fn main_currency_set_and_unknown() {
    let conn = db::test_conn();
    assert_eq!(main_currency(&conn).unwrap(), None);
    assert!(matches!(
        set_main_currency(&conn, "XXX"),
        Err(FinanceError::UnknownCurrency(_))
    ));
    set_main_currency(&conn, "CLP").unwrap();
    assert_eq!(main_currency(&conn).unwrap().unwrap().code, "CLP");
    instance_settings::set(&conn, "finance.main_currency", "ZZZ").unwrap();
    assert!(matches!(
        main_currency(&conn),
        Err(FinanceError::UnknownCurrency(c)) if c == "ZZZ"
    ));
}

#[test]
fn create_requires_main_currency_name_and_amount() {
    let conn = db::test_conn();
    assert!(matches!(
        create_account(&conn, "A", AccountKind::Cash, "", "0"),
        Err(FinanceError::NoMainCurrency)
    ));
    set_main_currency(&conn, "USD").unwrap();
    assert!(matches!(
        create_account(&conn, "  ", AccountKind::Cash, "", "0"),
        Err(FinanceError::EmptyName)
    ));
    for bad in ["", "x", "-1", "1.2.3"] {
        assert!(matches!(
            create_account(&conn, "A", AccountKind::Cash, "", bad),
            Err(FinanceError::InvalidAmount)
        ));
    }
    assert!(list_accounts(&conn).unwrap().is_empty());
}

#[test]
fn create_trims_and_uses_main_currency() {
    let conn = conn_with("USD");
    let id = create_account(
        &conn,
        "  Main ",
        AccountKind::Checking,
        " Bank ",
        "1.234,50",
    )
    .unwrap();
    let a = get_account(&conn, id).unwrap();
    assert_eq!(a.name, "Main");
    assert_eq!(a.bank, "Bank");
    assert_eq!(a.kind, AccountKind::Checking);
    assert_eq!(a.currency.code, "USD");
    assert_eq!((a.initial_balance, a.balance), (123450, 123450));
    let z = create_account(&conn, "Zero", AccountKind::Cash, "", "0").unwrap();
    assert_eq!(get_account(&conn, z).unwrap().balance, 0);
}

#[test]
fn list_in_creation_order() {
    let conn = conn_with("CLP");
    let b = create_account(&conn, "B", AccountKind::Savings, "", "1").unwrap();
    let a = create_account(&conn, "A", AccountKind::Demand, "", "2").unwrap();
    let ids: Vec<_> = list_accounts(&conn).unwrap().iter().map(|x| x.id).collect();
    assert_eq!(ids, [b, a]);
}

#[test]
fn update_keeps_currency_and_validates() {
    let conn = conn_with("CLP");
    let id = create_account(&conn, "A", AccountKind::Cash, "", "100").unwrap();
    set_main_currency(&conn, "USD").unwrap();
    update_account(&conn, id, " New ", AccountKind::Savings, "B", "17.062").unwrap();
    let a = get_account(&conn, id).unwrap();
    assert_eq!(
        (a.name.as_str(), a.kind, a.bank.as_str()),
        ("New", AccountKind::Savings, "B")
    );
    assert_eq!((a.currency.code, a.initial_balance), ("CLP", 17062));
    assert!(matches!(
        update_account(&conn, id, "", AccountKind::Cash, "", "1"),
        Err(FinanceError::EmptyName)
    ));
    assert!(matches!(
        update_account(&conn, id, "A", AccountKind::Cash, "", "1,5"),
        Err(FinanceError::InvalidAmount)
    ));
    assert!(matches!(
        update_account(&conn, AccountId(99), "A", AccountKind::Cash, "", "1"),
        Err(FinanceError::AccountNotFound)
    ));
    assert!(matches!(
        get_account(&conn, AccountId(99)),
        Err(FinanceError::AccountNotFound)
    ));
}

#[test]
fn unknown_stored_values_are_errors() {
    let conn = conn_with("CLP");
    let id = create_account(&conn, "A", AccountKind::Cash, "", "1").unwrap();
    conn.execute("UPDATE finance_account SET currency = 'ZZZ'", [])
        .unwrap();
    assert!(matches!(
        get_account(&conn, id),
        Err(FinanceError::UnknownCurrency(_))
    ));
    assert!(matches!(
        AccountKind::parse("x"),
        Err(FinanceError::BadKind(_))
    ));
    for k in [
        AccountKind::Checking,
        AccountKind::Demand,
        AccountKind::Savings,
        AccountKind::Cash,
    ] {
        assert_eq!(AccountKind::parse(k.as_str()).unwrap(), k);
    }
}
