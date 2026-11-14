// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for credit card bills.

use chrono::NaiveDate;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// A CLP card with the given days and a bank account.
fn setup(closing: u32, due: u32) -> (rusqlite::Connection, AccountId, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let card = create_card(&conn, "Visa", "", "CLP", "1000000", closing, due, None).unwrap();
    let bank = account(&conn, "Bank", AccountKind::Checking, "", "500000").unwrap();
    (conn, card, bank)
}

fn spend(conn: &rusqlite::Connection, card: AccountId, amount: &str, date: &str) {
    create_transaction(conn, TxKind::Expense, card, amount, d(date), "", None).unwrap();
}

fn pay(conn: &rusqlite::Connection, bank: AccountId, card: AccountId, amount: &str, date: &str) {
    create_transfer(conn, bank, card, amount, "", d(date), "").unwrap();
}

fn bill(conn: &rusqlite::Connection, card: AccountId, today: &str) -> Option<Bill> {
    card_bill(conn, card, d(today)).unwrap()
}

/// (closing, due) of a card with a purchase long before, as of `today`.
fn dates(closing: u32, due: u32, today: &str) -> (NaiveDate, NaiveDate) {
    let (conn, card, _) = setup(closing, due);
    spend(&conn, card, "1000", "2020-01-01");
    let b = bill(&conn, card, today).unwrap();
    (b.closing, b.due)
}

#[test]
fn closing_before_and_after_today() {
    // closing 5, today 20 Jun: closed this month
    assert_eq!(
        dates(5, 20, "2026-06-20"),
        (d("2026-06-05"), d("2026-06-20"))
    );
    // closing 25, today 20 Jun: closed last month
    assert_eq!(
        dates(25, 10, "2026-06-20"),
        (d("2026-05-25"), d("2026-06-10"))
    );
    // on the closing day itself the cycle is already closed
    assert_eq!(dates(20, 5, "2026-06-20").0, d("2026-06-20"));
    // across the year boundary
    assert_eq!(
        dates(25, 10, "2027-01-03"),
        (d("2026-12-25"), d("2027-01-10"))
    );
}

#[test]
fn closing_day_31_is_capped_in_short_months() {
    assert_eq!(dates(31, 10, "2026-05-15").0, d("2026-04-30"));
    assert_eq!(dates(31, 10, "2026-03-15").0, d("2026-02-28"));
    assert_eq!(dates(31, 10, "2028-03-15").0, d("2028-02-29"));
    assert_eq!(dates(31, 10, "2026-04-30").0, d("2026-04-30"));
    assert_eq!(dates(30, 10, "2026-03-15").0, d("2026-02-28"));
    assert_eq!(dates(29, 10, "2026-03-15").0, d("2026-02-28"));
    assert_eq!(dates(29, 10, "2028-03-15").0, d("2028-02-29"));
}

#[test]
fn due_day_before_closing_day_is_next_month() {
    assert_eq!(dates(15, 5, "2026-06-20").1, d("2026-07-05"));
    assert_eq!(dates(15, 5, "2026-12-20").1, d("2027-01-05"));
    // equal days: strictly after the closing, so next month
    assert_eq!(dates(10, 10, "2026-06-20").1, d("2026-07-10"));
    // due 31 capped to the month's end
    assert_eq!(dates(5, 31, "2026-06-20").1, d("2026-06-30"));
    // closing 28 Feb, due 31: Feb 28 is not after the closing, so March
    assert_eq!(dates(31, 31, "2026-03-15").1, d("2026-03-31"));
}

#[test]
fn purchases_before_and_after_the_closing() {
    let (conn, card, _) = setup(31, 10);
    spend(&conn, card, "10000", "2026-02-20");
    spend(&conn, card, "5000", "2026-03-01"); // after the closing: next bill
    let b = bill(&conn, card, "2026-03-05").unwrap();
    assert_eq!(
        (b.closing, b.due, b.pending),
        (d("2026-02-28"), d("2026-03-10"), 10_000)
    );
    assert_eq!(b.card_name, "Visa");
    // on the closing day itself that day's purchases are in
    spend(&conn, card, "1000", "2026-03-31");
    assert_eq!(bill(&conn, card, "2026-03-31").unwrap().pending, 16_000);
}

#[test]
fn partial_and_full_payment() {
    let (conn, card, bank) = setup(31, 10);
    spend(&conn, card, "10000", "2026-02-20");
    // a payment before the closing is already in the debt
    pay(&conn, bank, card, "3000", "2026-02-25");
    assert_eq!(bill(&conn, card, "2026-03-05").unwrap().pending, 7_000);
    pay(&conn, bank, card, "4000", "2026-03-02");
    assert_eq!(bill(&conn, card, "2026-03-05").unwrap().pending, 3_000);
    pay(&conn, bank, card, "3000", "2026-03-03");
    assert_eq!(bill(&conn, card, "2026-03-05"), None);
}

#[test]
fn overpayment_leaves_no_bill() {
    let (conn, card, bank) = setup(5, 20);
    spend(&conn, card, "10000", "2026-05-20");
    pay(&conn, bank, card, "15000", "2026-06-10");
    assert_eq!(bill(&conn, card, "2026-06-12"), None);
}

#[test]
fn no_debt_no_bill() {
    let (conn, card, _) = setup(5, 20);
    assert_eq!(bill(&conn, card, "2026-06-12"), None);
}

#[test]
fn cross_currency_payment_counts_what_arrives() {
    let (conn, card, _) = setup(5, 20);
    let usd = create_account(
        &conn,
        "Dolares",
        AccountKind::Checking,
        "",
        "USD",
        "1000",
        Some("900"),
    )
    .unwrap();
    spend(&conn, card, "100000", "2026-05-20");
    // 100 USD sent, 90.000 CLP arrive
    create_transfer(&conn, usd, card, "100", "90000", d("2026-06-10"), "").unwrap();
    assert_eq!(bill(&conn, card, "2026-06-12").unwrap().pending, 10_000);
}

#[test]
fn plain_account_has_no_bill() {
    let (conn, _, bank) = setup(5, 20);
    spend(&conn, bank, "1000", "2026-05-20");
    assert_eq!(bill(&conn, bank, "2026-06-12"), None);
}

#[test]
fn list_skips_archived_cards_and_sorts_by_due() {
    let (conn, a, _) = setup(5, 20);
    let b = create_card(&conn, "Amex", "", "CLP", "1000", 5, 10, None).unwrap();
    let c = create_card(&conn, "Old", "", "CLP", "1000", 5, 8, None).unwrap();
    for card in [a, b, c] {
        spend(&conn, card, "100", "2026-05-20");
    }
    set_account_archived(&conn, c, true).unwrap();
    let names: Vec<_> = list_bills(&conn, d("2026-06-12"))
        .unwrap()
        .into_iter()
        .map(|x| x.card_name)
        .collect();
    assert_eq!(names, ["Amex", "Visa"]);
}

#[test]
fn due_in_month_and_overdue() {
    let (conn, card, _) = setup(5, 10);
    spend(&conn, card, "1000", "2026-05-20");
    let today = d("2026-07-03"); // closed 5 Jun, due 10 Jun: overdue
    let n = |m: &str| bills_due_in_month(&conn, d(m), today).unwrap().len();
    assert_eq!(
        (n("2026-06-01"), n("2026-07-15"), n("2026-08-01")),
        (1, 1, 0)
    );
    assert!(bill(&conn, card, "2026-07-03").unwrap().overdue);
    // a bill not yet due is only in its due month
    spend(&conn, card, "2000", "2026-07-02");
    let today = d("2026-07-06"); // closed 5 Jul, due 10 Jul
    let bills = bills_due_in_month(&conn, d("2026-07-01"), today).unwrap();
    assert_eq!(
        (bills.len(), bills[0].pending, bills[0].overdue),
        (1, 3_000, false)
    );
    assert!(
        bills_due_in_month(&conn, d("2026-08-01"), today)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn payment_source_prefers_a_plain_active_account() {
    let (conn, card, bank) = setup(5, 20);
    assert_eq!(bill_payment_source(&conn, card).unwrap(), Some(bank));
    set_account_archived(&conn, bank, true).unwrap();
    assert_eq!(bill_payment_source(&conn, card).unwrap(), None);
    let other = create_card(&conn, "Other", "", "CLP", "1000", 5, 20, None).unwrap();
    assert_eq!(bill_payment_source(&conn, card).unwrap(), Some(other));
}
