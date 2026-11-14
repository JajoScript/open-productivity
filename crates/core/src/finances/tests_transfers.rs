// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for transfers and account management.

use chrono::NaiveDate;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup() -> (rusqlite::Connection, AccountId, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "A", AccountKind::Cash, "", "1000").unwrap();
    let b = account(&conn, "B", AccountKind::Cash, "", "50").unwrap();
    (conn, a, b)
}

fn balance(conn: &rusqlite::Connection, a: AccountId) -> i64 {
    get_account(conn, a).unwrap().balance
}

#[test]
fn transfer_moves_balances_both_ways() {
    let (conn, a, b) = setup();
    let t = create_transfer(&conn, a, b, "300", "", d("2026-03-05"), " rent ").unwrap();
    assert_eq!((balance(&conn, a), balance(&conn, b)), (700, 350));
    let got = get_transfer(&conn, t).unwrap();
    assert_eq!((got.from, got.to, got.amount), (a, b, 300));
    assert_eq!(
        (
            got.from_name.as_str(),
            got.to_name.as_str(),
            got.description.as_str()
        ),
        ("A", "B", "rent")
    );
    update_transfer(&conn, t, b, a, "100", "", d("2026-03-06"), "").unwrap();
    assert_eq!((balance(&conn, a), balance(&conn, b)), (1100, -50));
    delete_transaction(&conn, t, false).unwrap();
    assert_eq!((balance(&conn, a), balance(&conn, b)), (1000, 50));
}

#[test]
fn transfer_validation() {
    let (conn, a, b) = setup();
    assert!(matches!(
        create_transfer(&conn, a, a, "5", "", d("2026-03-05"), ""),
        Err(FinanceError::SameAccount)
    ));
    for bad in ["", "0", "-5", "abc"] {
        assert!(matches!(
            create_transfer(&conn, a, b, bad, "", d("2026-03-05"), ""),
            Err(FinanceError::InvalidAmount)
        ));
    }
    assert!(matches!(
        create_transfer(&conn, a, AccountId(99), "5", "", d("2026-03-05"), ""),
        Err(FinanceError::AccountNotFound)
    ));
    let t = create_transfer(&conn, a, b, "5", "", d("2026-03-05"), "").unwrap();
    assert!(matches!(
        update_transfer(&conn, t, b, b, "5", "", d("2026-03-05"), ""),
        Err(FinanceError::SameAccount)
    ));
    assert!(matches!(
        get_transfer(&conn, TransactionId(99)),
        Err(FinanceError::TransactionNotFound)
    ));
}

#[test]
fn transactions_and_transfers_do_not_mix() {
    let (conn, a, b) = setup();
    let t = create_transfer(&conn, a, b, "5", "", d("2026-03-05"), "").unwrap();
    let e = create_transaction(&conn, TxKind::Expense, a, "1", d("2026-03-05"), "", None).unwrap();
    assert!(matches!(
        get_transaction(&conn, t),
        Err(FinanceError::TransactionNotFound)
    ));
    assert!(matches!(
        update_transaction(&conn, t, TxKind::Expense, a, "1", d("2026-03-05"), "", None),
        Err(FinanceError::TransactionNotFound)
    ));
    assert!(matches!(
        get_transfer(&conn, e),
        Err(FinanceError::TransactionNotFound)
    ));
}

#[test]
fn month_list_filters_by_kind() {
    let (conn, a, b) = setup();
    let m = d("2026-03-01");
    create_transaction(&conn, TxKind::Expense, a, "10", m, "", None).unwrap();
    create_transaction(&conn, TxKind::Income, a, "20", m, "", None).unwrap();
    create_transfer(&conn, a, b, "300", "", m, "").unwrap();
    let n = |f| list_month(&conn, m, f).unwrap().len();
    assert_eq!(n(MovementFilter::All), 3);
    assert_eq!(n(MovementFilter::Expense), 1);
    assert_eq!(n(MovementFilter::Income), 1);
    assert_eq!(n(MovementFilter::Transfer), 1);
}

#[test]
fn month_list_mixes_transfers_in_order_and_totals_ignore_them() {
    let (conn, a, b) = setup();
    let c = create_category(&conn, TxKind::Expense, "Food").unwrap();
    let e = create_transaction(
        &conn,
        TxKind::Expense,
        a,
        "10",
        d("2026-03-05"),
        "",
        Some(c),
    )
    .unwrap();
    let t = create_transfer(&conn, a, b, "300", "", d("2026-03-05"), "").unwrap();
    let list = list_month(&conn, d("2026-03-01"), MovementFilter::All).unwrap();
    assert!(matches!(&list[0], Movement::Transfer(x) if x.id == t));
    assert!(matches!(&list[1], Movement::Tx(x) if x.id == e));
    for (kind, want) in [(TxKind::Expense, 10), (TxKind::Income, 0)] {
        let total: i64 = category_totals(&conn, d("2026-03-01"), kind)
            .unwrap()
            .iter()
            .map(|c| c.total)
            .sum();
        assert_eq!(total, want);
    }
}

#[test]
fn archive_and_delete_accounts() {
    let (conn, a, b) = setup();
    let lone = account(&conn, "C", AccountKind::Cash, "", "0").unwrap();
    assert!(!get_account(&conn, a).unwrap().archived);
    set_account_archived(&conn, lone, true).unwrap();
    assert!(get_account(&conn, lone).unwrap().archived);
    set_account_archived(&conn, lone, false).unwrap();
    assert!(matches!(
        set_account_archived(&conn, AccountId(99), true),
        Err(FinanceError::AccountNotFound)
    ));
    // Used as source, as destination, or by an income: cannot be deleted.
    create_transfer(&conn, a, b, "5", "", d("2026-03-05"), "").unwrap();
    for used in [a, b] {
        assert!(matches!(
            delete_account(&conn, used),
            Err(FinanceError::AccountInUse)
        ));
    }
    create_transaction(&conn, TxKind::Income, lone, "1", d("2026-03-05"), "", None).unwrap();
    assert!(matches!(
        delete_account(&conn, lone),
        Err(FinanceError::AccountInUse)
    ));
    let free = account(&conn, "D", AccountKind::Cash, "", "0").unwrap();
    delete_account(&conn, free).unwrap();
    assert!(matches!(
        delete_account(&conn, free),
        Err(FinanceError::AccountNotFound)
    ));
    assert_eq!(list_accounts(&conn).unwrap().len(), 3);
}

#[test]
fn archived_accounts_reject_new_movements_only() {
    let (conn, a, b) = setup();
    let t = create_transfer(&conn, a, b, "5", "", d("2026-03-05"), "").unwrap();
    let e = create_transaction(&conn, TxKind::Expense, a, "1", d("2026-03-05"), "", None).unwrap();
    set_account_archived(&conn, a, true).unwrap();
    assert!(matches!(
        create_transaction(&conn, TxKind::Expense, a, "1", d("2026-03-05"), "", None),
        Err(FinanceError::AccountArchived)
    ));
    assert!(matches!(
        create_transfer(&conn, b, a, "1", "", d("2026-03-05"), ""),
        Err(FinanceError::AccountArchived)
    ));
    assert!(matches!(
        create_transfer(&conn, a, b, "1", "", d("2026-03-05"), ""),
        Err(FinanceError::AccountArchived)
    ));
    // Editing keeps the account the movement already had...
    update_transaction(&conn, e, TxKind::Expense, a, "2", d("2026-03-05"), "", None).unwrap();
    update_transfer(&conn, t, a, b, "6", "", d("2026-03-05"), "").unwrap();
    // ...but cannot move to another archived one.
    let c = account(&conn, "C", AccountKind::Cash, "", "0").unwrap();
    set_account_archived(&conn, c, true).unwrap();
    assert!(matches!(
        update_transaction(&conn, e, TxKind::Expense, c, "2", d("2026-03-05"), "", None),
        Err(FinanceError::AccountArchived)
    ));
    assert!(matches!(
        update_transfer(&conn, t, a, c, "6", "", d("2026-03-05"), ""),
        Err(FinanceError::AccountArchived)
    ));
    // The order of list_accounts does not depend on archiving.
    let names: Vec<_> = list_accounts(&conn)
        .unwrap()
        .into_iter()
        .map(|x| x.name)
        .collect();
    assert_eq!(names, ["A", "B", "C"]);
}
