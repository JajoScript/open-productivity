// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for credit lines.

use chrono::NaiveDate;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup() -> (rusqlite::Connection, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let line = create_credit_line(&conn, " Line ", " Bank ", "CLP", "-200", "1000", None).unwrap();
    (conn, line)
}

fn spend(conn: &rusqlite::Connection, id: AccountId, amount: &str) {
    create_transaction(conn, TxKind::Expense, id, amount, d("2026-03-02"), "", None).unwrap();
}

#[test]
fn create_allows_debt_and_trims() {
    let (conn, id) = setup();
    let a = get_account(&conn, id).unwrap();
    assert_eq!((a.name.as_str(), a.bank.as_str()), ("Line", "Bank"));
    assert_eq!(
        (a.kind, a.balance, a.credit_limit),
        (AccountKind::CreditLine, -200, Some(1000))
    );
    assert_eq!(a.card, None);
    assert!(create_credit_line(&conn, "Z", "", "CLP", "0", "5", None).is_ok());
}

#[test]
fn create_validates() {
    let (conn, _) = setup();
    let mk = |name, bal, limit| create_credit_line(&conn, name, "", "CLP", bal, limit, None);
    assert!(matches!(mk(" ", "0", "10"), Err(FinanceError::EmptyName)));
    assert!(matches!(
        mk("L", "x", "10"),
        Err(FinanceError::InvalidAmount)
    ));
    for l in ["", "0", "abc", "-5"] {
        assert!(
            matches!(mk("L", "0", l), Err(FinanceError::InvalidLimit)),
            "{l}"
        );
    }
    assert_eq!(list_accounts(&conn).unwrap().len(), 1);
}

#[test]
fn kind_is_locked_in_every_direction() {
    let (conn, line) = setup();
    let plain = account(&conn, "A", AccountKind::Cash, "", "0").unwrap();
    let card = create_card(&conn, "V", "", "CLP", "10", 1, 1, None).unwrap();
    assert!(matches!(
        create_account(&conn, "X", AccountKind::CreditLine, "", "CLP", "0", None),
        Err(FinanceError::KindLocked)
    ));
    for (id, kind) in [
        (plain, AccountKind::CreditLine),
        (line, AccountKind::Cash),
        (line, AccountKind::Credit),
    ] {
        assert!(matches!(
            update_account(&conn, id, "A", kind, "", "CLP", "0", None),
            Err(FinanceError::KindLocked)
        ));
    }
    for id in [plain, card] {
        assert!(matches!(
            update_credit_line(&conn, id, "A", "", "CLP", "0", "10", None),
            Err(FinanceError::KindLocked)
        ));
    }
    assert!(matches!(
        update_card(&conn, line, "A", "", "CLP", "10", 1, 1, None),
        Err(FinanceError::KindLocked)
    ));
    update_credit_line(&conn, line, "L2", "B", "CLP", "-50", "2000", None).unwrap();
    let a = get_account(&conn, line).unwrap();
    assert_eq!(
        (a.name.as_str(), a.balance, a.credit_limit),
        ("L2", -50, Some(2000))
    );
}

#[test]
fn usage_figures_and_over_limit() {
    let (conn, line) = setup();
    let usage = || get_account(&conn, line).unwrap().credit_usage().unwrap();
    let u = |used, available, over_limit| CardUsage {
        used,
        available,
        per_mille: used * 1000 / 1000,
        over_limit,
    };
    assert_eq!(usage(), u(200, 800, false));
    spend(&conn, line, "800");
    assert_eq!(usage(), u(1000, 0, false));
    spend(&conn, line, "100");
    assert_eq!(usage(), u(1100, -100, true));
}

#[test]
fn has_no_bill() {
    let (conn, line) = setup();
    spend(&conn, line, "100");
    assert_eq!(card_bill(&conn, line, d("2026-04-01")).unwrap(), None);
    assert!(list_bills(&conn, d("2026-04-01")).unwrap().is_empty());
}

#[test]
fn currency_and_archive_like_accounts() {
    let (conn, line) = setup();
    assert!(matches!(
        update_credit_line(&conn, line, "L", "", "USD", "0", "10", None),
        Err(FinanceError::MissingRate(_))
    ));
    spend(&conn, line, "5");
    assert!(matches!(
        update_credit_line(&conn, line, "L", "", "USD", "0", "10", Some("900")),
        Err(FinanceError::CurrencyLocked)
    ));
    set_account_archived(&conn, line, true).unwrap();
    assert!(matches!(
        create_transaction(&conn, TxKind::Expense, line, "5", d("2026-03-02"), "", None),
        Err(FinanceError::AccountArchived)
    ));
}
