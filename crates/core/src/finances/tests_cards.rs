// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for credit cards.

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
    let card = create_card(&conn, " Visa ", " Bank ", "CLP", "1000", 5, 20, None).unwrap();
    (conn, card)
}

fn spend(conn: &rusqlite::Connection, card: AccountId, amount: &str) {
    create_transaction(
        conn,
        TxKind::Expense,
        card,
        amount,
        d("2026-03-02"),
        "",
        None,
    )
    .unwrap();
}

#[test]
fn create_trims_and_starts_without_debt() {
    let (conn, card) = setup();
    let a = get_account(&conn, card).unwrap();
    assert_eq!((a.name.as_str(), a.bank.as_str()), ("Visa", "Bank"));
    assert_eq!(
        (a.kind, a.balance, a.initial_balance),
        (AccountKind::Credit, 0, 0)
    );
    assert_eq!(
        a.card,
        Some(CardTerms {
            limit: 1000,
            closing_day: 5,
            due_day: 20
        })
    );
}

#[test]
fn create_validates() {
    let (conn, _) = setup();
    let mk = |name, limit, c, d| create_card(&conn, name, "", "CLP", limit, c, d, None);
    assert!(matches!(mk(" ", "10", 1, 1), Err(FinanceError::EmptyName)));
    for l in ["", "0", "abc", "-5"] {
        assert!(
            matches!(mk("C", l, 1, 1), Err(FinanceError::InvalidLimit)),
            "{l}"
        );
    }
    for (c, d) in [(0, 1), (32, 1), (1, 0), (1, 32)] {
        assert!(matches!(mk("C", "10", c, d), Err(FinanceError::InvalidDay)));
    }
    assert!(mk("C", "10", 31, 1).is_ok());
    assert_eq!(list_accounts(&conn).unwrap().len(), 2);
}

#[test]
fn kind_is_locked_both_ways() {
    let (conn, card) = setup();
    let plain = account(&conn, "A", AccountKind::Cash, "", "0").unwrap();
    assert!(matches!(
        create_account(&conn, "X", AccountKind::Credit, "", "CLP", "0", None),
        Err(FinanceError::KindLocked)
    ));
    assert!(matches!(
        update_account(&conn, plain, "A", AccountKind::Credit, "", "CLP", "0", None),
        Err(FinanceError::KindLocked)
    ));
    assert!(matches!(
        update_account(&conn, card, "V", AccountKind::Cash, "", "CLP", "0", None),
        Err(FinanceError::KindLocked)
    ));
    assert!(matches!(
        update_card(&conn, plain, "A", "", "CLP", "10", 1, 1, None),
        Err(FinanceError::KindLocked)
    ));
    update_card(&conn, card, "Mastercard", "B2", "CLP", "2000", 10, 25, None).unwrap();
    let a = get_account(&conn, card).unwrap();
    assert_eq!(
        (a.name.as_str(), a.kind),
        ("Mastercard", AccountKind::Credit)
    );
    assert_eq!(
        a.card.map(|t| (t.limit, t.closing_day, t.due_day)),
        Some((2000, 10, 25))
    );
}

#[test]
fn usage_figures() {
    let (conn, card) = setup();
    let usage = || get_account(&conn, card).unwrap().card_usage().unwrap();
    assert_eq!(
        usage(),
        CardUsage {
            used: 0,
            available: 1000,
            per_mille: 0,
            over_limit: false
        }
    );
    spend(&conn, card, "250");
    assert_eq!(
        usage(),
        CardUsage {
            used: 250,
            available: 750,
            per_mille: 250,
            over_limit: false
        }
    );
    spend(&conn, card, "750");
    assert_eq!(
        usage(),
        CardUsage {
            used: 1000,
            available: 0,
            per_mille: 1000,
            over_limit: false
        }
    );
    spend(&conn, card, "500");
    assert_eq!(
        usage(),
        CardUsage {
            used: 1500,
            available: -500,
            per_mille: 1500,
            over_limit: true
        }
    );
    // A payment is a transfer into the card.
    let bank = account(&conn, "A", AccountKind::Checking, "", "5000").unwrap();
    create_transfer(&conn, bank, card, "600", "", d("2026-03-10"), "").unwrap();
    assert_eq!(
        usage(),
        CardUsage {
            used: 900,
            available: 100,
            per_mille: 900,
            over_limit: false
        }
    );
    // Overpaying leaves credit in favor, not negative use.
    create_transfer(&conn, bank, card, "1000", "", d("2026-03-11"), "").unwrap();
    assert_eq!(usage().used, 0);
    assert_eq!(get_account(&conn, card).unwrap().balance, 100);
    assert_eq!(
        account(&conn, "P", AccountKind::Cash, "", "0")
            .map(|id| { get_account(&conn, id).unwrap().card_usage() })
            .unwrap(),
        None
    );
}

#[test]
fn summary_subtracts_the_debt() {
    let (conn, card) = setup();
    account(&conn, "A", AccountKind::Cash, "", "5000").unwrap();
    spend(&conn, card, "300");
    let s = month_summary(&conn, d("2026-03-17")).unwrap();
    assert_eq!(s.accounts_total.value, 4700);
}

#[test]
fn currency_and_rate_like_accounts() {
    let (conn, card) = setup();
    // No movements: the currency can change, and the new one needs its rate.
    assert!(matches!(
        update_card(&conn, card, "V", "", "USD", "10", 1, 1, None),
        Err(FinanceError::MissingRate(_))
    ));
    update_card(&conn, card, "V", "", "USD", "10.50", 1, 1, Some("900")).unwrap();
    let a = get_account(&conn, card).unwrap();
    assert_eq!((a.currency.code, a.card.unwrap().limit), ("USD", 1050));
    spend(&conn, card, "5");
    assert!(matches!(
        update_card(&conn, card, "V", "", "CLP", "10", 1, 1, None),
        Err(FinanceError::CurrencyLocked)
    ));
    // A new card in a foreign currency saves its rate with it.
    let eur_less = create_card(&conn, "U", "", "USD", "1", 1, 1, None).unwrap();
    assert!(get_account(&conn, eur_less).is_ok());
}

#[test]
fn archive_and_delete_like_accounts() {
    let (conn, card) = setup();
    set_account_archived(&conn, card, true).unwrap();
    assert!(matches!(
        create_transaction(&conn, TxKind::Expense, card, "5", d("2026-03-02"), "", None),
        Err(FinanceError::AccountArchived)
    ));
    assert!(get_account(&conn, card).unwrap().archived);
    set_account_archived(&conn, card, false).unwrap();
    spend(&conn, card, "5");
    assert!(matches!(
        delete_account(&conn, card),
        Err(FinanceError::AccountInUse)
    ));
}
