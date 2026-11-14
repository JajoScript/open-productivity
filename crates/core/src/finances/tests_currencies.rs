// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for other currencies, exchange rates and totals in the main currency.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// Test-only third currency: the module supports just CLP and USD.
static EUR: Currency = Currency {
    code: "EUR",
    decimals: 2,
    symbol: "€",
};

fn cur(code: &str) -> &'static Currency {
    currency(code).unwrap()
}

/// CLP main; 1 USD = 987.24 CLP; 1 EUR = 1,100 CLP.
fn rates() -> Rates {
    Rates::new(cur("CLP"), [("USD", 987_240_000), ("EUR", 1_100_000_000)])
}

fn setup() -> Connection {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    conn
}

fn usd_account(conn: &Connection, name: &str, balance: &str) -> AccountId {
    create_account(
        conn,
        name,
        AccountKind::Cash,
        "",
        "USD",
        balance,
        Some("987,24"),
    )
    .unwrap()
}

#[test]
fn converts_through_the_main_currency() {
    let r = rates();
    let (clp, usd, eur) = (cur("CLP"), cur("USD"), &EUR);
    assert_eq!(r.convert(100, usd, clp).unwrap(), 987);
    assert_eq!(r.convert(987_240, clp, usd).unwrap(), 100_000);
    assert_eq!(r.convert(1234, usd, usd).unwrap(), 1234);
    assert_eq!(r.convert(5000, clp, clp).unwrap(), 5000);
    let back = r
        .convert(r.convert(25_000, usd, clp).unwrap(), clp, usd)
        .unwrap();
    assert!((back - 25_000).abs() <= 1, "{back}");
    // 1 USD = 987.24 / 1100 EUR = 0.8976 EUR; 100 EUR = 111.42 USD.
    assert_eq!(r.convert(100, usd, eur).unwrap(), 90);
    assert_eq!(r.convert(10_000, eur, usd).unwrap(), 11_142);
    assert_eq!(r.to_main(100, usd).unwrap(), 987);
    assert_eq!(r.from_main(1100, eur).unwrap(), 100);
}

#[test]
fn conversion_rounds_halves_away_from_zero_and_keeps_sign() {
    // 1 USD = 0.5 CLP.
    let r = Rates::new(cur("CLP"), [("USD", 500_000)]);
    // 100 cents = 0.5 CLP -> 1; 99 cents -> 0.495 -> 0; negative mirrors.
    assert_eq!(r.to_main(100, cur("USD")).unwrap(), 1);
    assert_eq!(r.to_main(-100, cur("USD")).unwrap(), -1);
    assert_eq!(r.to_main(99, cur("USD")).unwrap(), 0);
    assert_eq!(r.to_main(0, cur("USD")).unwrap(), 0);
    // 1 CLP = 2 USD = 200 cents.
    assert_eq!(r.from_main(1, cur("USD")).unwrap(), 200);
}

#[test]
fn conversion_saturates() {
    let r = rates();
    assert_eq!(r.to_main(i64::MAX, cur("USD")).unwrap(), i64::MAX);
    assert_eq!(r.to_main(i64::MIN + 1, cur("USD")).unwrap(), -i64::MAX);
    let huge = Rates::new(cur("CLP"), [("USD", i64::MAX)]);
    assert_eq!(huge.to_main(i64::MAX, cur("USD")).unwrap(), i64::MAX);
    let tiny = Rates::new(cur("CLP"), [("USD", 1)]);
    assert_eq!(tiny.from_main(i64::MAX, cur("USD")).unwrap(), i64::MAX);
}

#[test]
fn missing_rate_is_an_error_never_one() {
    let r = Rates::new(cur("CLP"), []);
    assert!(matches!(
        r.to_main(100, cur("USD")),
        Err(FinanceError::MissingRate(c)) if c == "USD"
    ));
    assert!(matches!(
        r.from_main(100, &EUR),
        Err(FinanceError::MissingRate(c)) if c == "EUR"
    ));
    assert!(matches!(
        r.convert(1, cur("USD"), &EUR),
        Err(FinanceError::MissingRate(_))
    ));
    assert_eq!(r.to_main(100, cur("CLP")).unwrap(), 100);
    assert!(r.needs_rate(cur("USD")) && !r.needs_rate(cur("CLP")));
}

#[test]
fn rates_parse_and_format() {
    for (t, v) in [
        ("987,24", Some(987_240_000)),
        ("987.24", Some(987_240_000)),
        ("987.240", Some(987_240_000_000)),
        ("1.050", Some(1_050_000_000)),
        ("1,2340", Some(1_234_000)),
        ("0,001", Some(1_000)),
        ("1.234,567", Some(1_234_567_000)),
        ("1234", Some(1_234_000_000)),
        ("1.234,5", Some(1_234_500_000)),
        ("1,234.5", Some(1_234_500_000)),
        ("1.234.567", Some(1_234_567_000_000)),
        ("0,001", Some(1_000)),
        ("0.000001", Some(1)),
        (" 2 ", Some(2_000_000)),
        ("0", None),
        ("0,0000001", None),
        ("1,2345678", None),
        ("-1", None),
        ("", None),
        ("abc", None),
        ("1,", None),
        (",5", None),
        ("1.2.3", None),
        ("12.34.5678", None),
        ("10000000,000001", None),
        ("10000000", Some(MAX_RATE)),
    ] {
        assert_eq!(parse_rate(t), v, "{t}");
    }
    for style in [AmountStyle::Spanish, AmountStyle::English] {
        for v in [
            1,
            1_000,
            1_234_000,
            500_000,
            987_240_000,
            1_234_567_890_123,
            MAX_RATE,
        ] {
            assert_eq!(parse_rate(&format_rate(v, style)), Some(v), "{v}");
        }
    }
    assert_eq!(format_rate(987_240_000, AmountStyle::Spanish), "987,24");
    assert_eq!(format_rate(1_000_000, AmountStyle::English), "1");
}

#[test]
fn formats_other_currencies_with_their_code() {
    use AmountStyle::*;
    let (clp, usd) = (cur("CLP"), cur("USD"));
    assert_eq!(format_in(17062, clp, clp, Spanish), "$ 17.062");
    assert_eq!(format_in(1706200, usd, clp, Spanish), "17.062,00 USD");
    assert_eq!(format_in(1706200, usd, clp, English), "17,062.00 USD");
    assert_eq!(format_in(-5, usd, clp, English), "\u{2212}0.05 USD");
    assert_eq!(format_in(10, clp, usd, English), "10 CLP");
    assert_eq!(format_in(-5, clp, clp, English), "\u{2212}$5");
}

#[test]
fn rate_is_asked_once_with_the_account() {
    let conn = setup();
    let new = |code, rate| create_account(&conn, "U", AccountKind::Cash, "", code, "10", rate);
    assert!(matches!(new("USD", None), Err(FinanceError::MissingRate(c)) if c == "USD"));
    assert!(matches!(
        new("USD", Some("  ")),
        Err(FinanceError::MissingRate(_))
    ));
    assert!(matches!(
        new("USD", Some("0")),
        Err(FinanceError::InvalidRate)
    ));
    assert!(matches!(
        new("XXX", None),
        Err(FinanceError::UnknownCurrency(_))
    ));
    assert!(list_accounts(&conn).unwrap().is_empty());
    new("USD", Some("987,24")).unwrap();
    // The second account of the currency does not ask again, and a rate given is ignored.
    new("USD", None).unwrap();
    new("USD", Some("1")).unwrap();
    assert_eq!(
        list_rates(&conn).unwrap(),
        [Rate {
            currency: cur("USD"),
            rate: Some(987_240_000)
        }]
    );
    // The main currency needs none.
    new("CLP", None).unwrap();
    assert_eq!(list_rates(&conn).unwrap().len(), 1);
}

#[test]
fn rate_and_account_are_saved_together() {
    let conn = setup();
    // The account is inserted first: a rate that cannot be read undoes it.
    let r = create_account(&conn, "U", AccountKind::Cash, "", "USD", "10", Some("x"));
    assert!(matches!(r, Err(FinanceError::InvalidRate)));
    assert!(list_accounts(&conn).unwrap().is_empty());
    // A failing account leaves no rate.
    let r = create_account(&conn, " ", AccountKind::Cash, "", "USD", "10", Some("5"));
    assert!(matches!(r, Err(FinanceError::EmptyName)));
    let rows = |conn: &Connection| -> i64 {
        conn.query_row("SELECT count(*) FROM finance_rate", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(rows(&conn), 0);
    // The same on update.
    let id = account(&conn, "A", AccountKind::Cash, "", "1").unwrap();
    let r = update_account(&conn, id, "A", AccountKind::Cash, "", "USD", "1", Some("x"));
    assert!(matches!(r, Err(FinanceError::InvalidRate)));
    assert_eq!(get_account(&conn, id).unwrap().currency.code, "CLP");
    assert_eq!(rows(&conn), 0);
    let r = update_account(&conn, id, "A", AccountKind::Cash, "", "USD", "1", None);
    assert!(matches!(r, Err(FinanceError::MissingRate(_))));
    let r = update_account(
        &conn,
        id,
        "A",
        AccountKind::Cash,
        "",
        "USD",
        "1",
        Some("987,24"),
    );
    r.unwrap();
    assert_eq!(get_account(&conn, id).unwrap().currency.code, "USD");
    assert_eq!(
        load_rates(&conn).unwrap().stored(cur("USD")),
        Some(987_240_000)
    );
}

#[test]
fn currency_locks_with_movements() {
    let conn = setup();
    let a = usd_account(&conn, "U", "10");
    let b = account(&conn, "B", AccountKind::Cash, "", "0").unwrap();
    // Free while unused.
    update_account(&conn, b, "B", AccountKind::Cash, "", "USD", "1", None).unwrap();
    update_account(&conn, b, "B", AccountKind::Cash, "", "CLP", "1", None).unwrap();
    create_transaction(&conn, TxKind::Expense, a, "1", d("2026-03-01"), "", None).unwrap();
    let r = update_account(&conn, a, "U", AccountKind::Cash, "", "CLP", "10", None);
    assert!(matches!(r, Err(FinanceError::CurrencyLocked)));
    assert_eq!(get_account(&conn, a).unwrap().currency.code, "USD");
    // The same currency still edits.
    update_account(&conn, a, "V", AccountKind::Cash, "", "USD", "10", None).unwrap();
    assert!(matches!(
        update_account(&conn, a, "V", AccountKind::Cash, "", "XXX", "10", None),
        Err(FinanceError::UnknownCurrency(_))
    ));
}

#[test]
fn destination_account_currency_is_locked_too() {
    let conn = setup();
    let usd = usd_account(&conn, "U", "10");
    let clp = account(&conn, "C", AccountKind::Cash, "", "1000").unwrap();
    create_transfer(&conn, clp, usd, "987", "1", d("2026-03-01"), "").unwrap();
    let r = update_account(&conn, usd, "U", AccountKind::Cash, "", "CLP", "10", None);
    assert!(matches!(r, Err(FinanceError::CurrencyLocked)));
}

#[test]
fn set_rates_validates_and_is_atomic() {
    let conn = setup();
    usd_account(&conn, "U", "1");
    set_rate(&conn, "USD", "1.000,5").unwrap();
    assert_eq!(list_rates(&conn).unwrap()[0].rate, Some(1_000_500_000));
    assert!(matches!(
        set_rate(&conn, "CLP", "1"),
        Err(FinanceError::InvalidRate)
    ));
    assert!(matches!(
        set_rate(&conn, "XXX", "1"),
        Err(FinanceError::UnknownCurrency(_))
    ));
    assert!(matches!(
        set_rate(&conn, "USD", "0"),
        Err(FinanceError::InvalidRate)
    ));
    let r = set_rates(&conn, &[("USD", "900"), ("USD", "oops")]);
    assert!(matches!(r, Err(FinanceError::InvalidRate)));
    assert_eq!(list_rates(&conn).unwrap()[0].rate, Some(1_000_500_000));
    let r = set_rates(&conn, &[("USD", "900"), ("EUR", "1100")]);
    assert!(matches!(r, Err(FinanceError::UnknownCurrency(_))));
    assert_eq!(list_rates(&conn).unwrap()[0].rate, Some(1_000_500_000));
    set_rates(&conn, &[("USD", "900")]).unwrap();
    assert_eq!(list_rates(&conn).unwrap()[0].rate, Some(900_000_000));
    // Only the currencies some account uses are listed.
    assert_eq!(list_rates(&conn).unwrap().len(), 1);
    assert!(matches!(
        set_rate(&db::test_conn(), "USD", "1"),
        Err(FinanceError::NoMainCurrency)
    ));
}

#[test]
fn transfer_between_currencies_uses_the_amount_that_arrives() {
    let conn = setup();
    let usd = usd_account(&conn, "U", "10");
    let clp = account(&conn, "C", AccountKind::Cash, "", "5000").unwrap();
    let date = d("2026-03-05");
    for bad in ["", "0", "x", "-1", "1,2345"] {
        let r = create_transfer(&conn, clp, usd, "1000", bad, date, "");
        assert!(matches!(r, Err(FinanceError::InvalidAmount)), "{bad}");
    }
    let t = create_transfer(&conn, clp, usd, "987", "1,00", date, "").unwrap();
    let tr = get_transfer(&conn, t).unwrap();
    assert_eq!((tr.currency.code, tr.amount), ("CLP", 987));
    assert_eq!((tr.to_currency.code, tr.to_amount), ("USD", 100));
    assert_eq!(get_account(&conn, clp).unwrap().balance, 5000 - 987);
    assert_eq!(get_account(&conn, usd).unwrap().balance, 1000 + 100);
    // Back: the amount is read in USD and what arrives in CLP.
    update_transfer(&conn, t, usd, clp, "2", "1000", date, "").unwrap();
    let tr = get_transfer(&conn, t).unwrap();
    assert_eq!((tr.amount, tr.to_amount), (200, 1000));
    assert_eq!(get_account(&conn, usd).unwrap().balance, 1000 - 200);
    assert_eq!(get_account(&conn, clp).unwrap().balance, 5000 + 1000);
    // The same currency ignores it and stores nothing.
    let other = account(&conn, "D", AccountKind::Cash, "", "0").unwrap();
    let t2 = create_transfer(&conn, clp, other, "10", "999", date, "").unwrap();
    let tr = get_transfer(&conn, t2).unwrap();
    assert_eq!((tr.amount, tr.to_amount), (10, 10));
    let stored = |t: TransactionId| -> Option<i64> {
        conn.query_row(
            "SELECT to_amount FROM finance_transaction WHERE id = ?1",
            [t.0],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(stored(t2), None);
    assert_eq!(get_account(&conn, other).unwrap().balance, 10);
    // Editing into the same currency clears it.
    update_transfer(&conn, t, clp, other, "7", "", date, "").unwrap();
    assert_eq!(get_transfer(&conn, t).unwrap().to_amount, 7);
    assert_eq!(stored(t), None);
}

#[test]
fn summary_and_categories_are_in_the_main_currency() {
    let conn = setup();
    let usd = usd_account(&conn, "U", "100");
    let clp = account(&conn, "C", AccountKind::Cash, "", "1000").unwrap();
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    let date = d("2026-03-05");
    create_transaction(&conn, TxKind::Expense, usd, "10", date, "", Some(food)).unwrap();
    create_transaction(&conn, TxKind::Expense, clp, "500", date, "", Some(food)).unwrap();
    create_transaction(&conn, TxKind::Expense, usd, "1", date, "", None).unwrap();
    create_transaction(&conn, TxKind::Income, usd, "20", date, "", None).unwrap();
    create_transaction(&conn, TxKind::Expense, usd, "5", d("2026-02-10"), "", None).unwrap();
    // 10 USD = 9,872.4 -> 9,872 CLP (rounded each); 1 USD -> 987; 20 USD -> 19,745; 5 USD -> 4,936.
    let totals = category_totals(&conn, date, TxKind::Expense).unwrap();
    assert_eq!(
        totals
            .iter()
            .map(|t| (t.category, t.total))
            .collect::<Vec<_>>(),
        [(Some(food), 9_872 + 500), (None, 987)]
    );
    let s = month_summary(&conn, date).unwrap();
    assert!(s.uses_rates);
    assert_eq!(s.income.value, 19_745);
    assert_eq!(
        s.expense,
        Figure {
            value: 9_872 + 500 + 987,
            previous: 4_936
        }
    );
    assert_eq!(s.by_category[0].total, 10_372);
    assert_eq!(s.by_category[0].width, 1.0);
    // Opening: 100 USD = 98,724 CLP + 1,000; at the end of February less 5 USD.
    assert_eq!(s.accounts_total.previous, 98_724 + 1_000 - 4_936);
    let march = 98_724 + 1_000 - 4_936 + 19_745 - 9_872 - 987 - 500;
    assert_eq!(s.accounts_total.value, march);
}

#[test]
fn transfers_across_currencies_in_the_accounts_total() {
    let conn = setup();
    let usd = usd_account(&conn, "U", "100");
    let clp = account(&conn, "C", AccountKind::Cash, "", "1000").unwrap();
    let date = d("2026-03-05");
    let before = month_summary(&conn, date).unwrap().accounts_total.value;
    // 987 CLP leave and 1.00 USD (987 CLP) arrives: both active, so the total does not move.
    create_transfer(&conn, clp, usd, "987", "1", date, "").unwrap();
    let after = month_summary(&conn, date).unwrap().accounts_total.value;
    assert_eq!(after, before);
    // An archived destination: only the money leaving counts.
    set_account_archived(&conn, usd, true).unwrap();
    let archived = month_summary(&conn, date).unwrap().accounts_total.value;
    assert_eq!(archived, 1000 - 987);
}

#[test]
fn single_currency_instances_do_not_use_rates() {
    let conn = setup();
    account(&conn, "A", AccountKind::Cash, "", "1").unwrap();
    assert!(!month_summary(&conn, d("2026-03-01")).unwrap().uses_rates);
    assert!(list_rates(&conn).unwrap().is_empty());
}

#[test]
fn a_lost_rate_fails_the_totals() {
    let conn = setup();
    let usd = usd_account(&conn, "U", "100");
    create_transaction(&conn, TxKind::Expense, usd, "1", d("2026-03-01"), "", None).unwrap();
    conn.execute("DELETE FROM finance_rate", []).unwrap();
    assert!(matches!(
        month_summary(&conn, d("2026-03-01")),
        Err(FinanceError::MissingRate(c)) if c == "USD"
    ));
    assert!(matches!(
        category_totals(&conn, d("2026-03-01"), TxKind::Expense),
        Err(FinanceError::MissingRate(_))
    ));
}

#[test]
fn category_totals_saturate_and_order() {
    let conn = setup();
    let usd = usd_account(&conn, "U", "1");
    let (a, b) = (
        create_category(&conn, TxKind::Expense, "b").unwrap(),
        create_category(&conn, TxKind::Expense, "A").unwrap(),
    );
    let date = d("2026-03-05");
    for c in [a, b] {
        create_transaction(&conn, TxKind::Expense, usd, "1", date, "", Some(c)).unwrap();
    }
    let names: Vec<_> = category_totals(&conn, date, TxKind::Expense)
        .unwrap()
        .into_iter()
        .map(|t| t.name.unwrap())
        .collect();
    assert_eq!(names, ["A", "b"]);
    for _ in 0..2 {
        conn.execute(
            "INSERT INTO finance_transaction (kind, account_id, amount, date, category_id)
             VALUES ('expense', ?1, ?2, '2026-03-05', ?3)",
            (usd.0, i64::MAX, a.0),
        )
        .unwrap();
    }
    let totals = category_totals(&conn, date, TxKind::Expense).unwrap();
    assert_eq!(totals[0].total, i64::MAX);
}
