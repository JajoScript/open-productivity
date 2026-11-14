// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The manual exchange rate of each non-main currency, stored once for the whole history.

use rusqlite::Connection;

use super::FinanceError;
use super::accounts::main_currency;
use super::money::{Currency, Rates, currency, parse_rate};

/// A non-main currency some account uses, with its rate (6 decimals) if set.
#[derive(Debug, PartialEq)]
pub struct Rate {
    pub currency: &'static Currency,
    pub rate: Option<i64>,
}

/// The main currency and every stored rate; load it once per operation.
pub fn load_rates(conn: &Connection) -> Result<Rates, FinanceError> {
    let main = main_currency(conn)?.ok_or(FinanceError::NoMainCurrency)?;
    let mut stmt = conn.prepare("SELECT currency, rate FROM finance_rate")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut stored = Vec::new();
    for row in rows {
        let (code, rate) = row?;
        // A stored code the list no longer knows cannot be used by any account.
        if let Some(c) = currency(&code) {
            stored.push((c.code, rate));
        }
    }
    Ok(Rates::new(main, stored))
}

/// The non-main currencies used by some account, in the list's order.
pub fn list_rates(conn: &Connection) -> Result<Vec<Rate>, FinanceError> {
    let rates = load_rates(conn)?;
    let mut stmt = conn.prepare("SELECT DISTINCT currency FROM finance_account")?;
    let used = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(super::money::CURRENCIES
        .iter()
        .filter(|c| c.code != rates.main().code && used.iter().any(|u| u == c.code))
        .map(|c| Rate {
            currency: c,
            rate: rates.stored(c),
        })
        .collect())
}

/// Validates and stores the rate; the main currency has none.
fn save(conn: &Connection, main: &Currency, code: &str, text: &str) -> Result<(), FinanceError> {
    let c = currency(code).ok_or_else(|| FinanceError::UnknownCurrency(code.to_string()))?;
    if c.code == main.code {
        return Err(FinanceError::InvalidRate);
    }
    let rate = parse_rate(text).ok_or(FinanceError::InvalidRate)?;
    conn.execute(
        "INSERT INTO finance_rate (currency, rate) VALUES (?1, ?2)
         ON CONFLICT (currency) DO UPDATE SET rate = excluded.rate",
        (c.code, rate),
    )?;
    Ok(())
}

/// Stores the rate typed for `c` when it is not the main currency and has none yet.
/// Runs inside the caller's transaction.
pub(super) fn ensure_rate(
    conn: &Connection,
    main: &Currency,
    c: &Currency,
    text: Option<&str>,
) -> Result<(), FinanceError> {
    if c.code == main.code || !load_rates(conn)?.needs_rate(c) {
        return Ok(());
    }
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        None => Err(FinanceError::MissingRate(c.code.to_string())),
        Some(t) => save(conn, main, c.code, t),
    }
}

/// Sets several rates in one transaction: all or none.
pub fn set_rates(conn: &Connection, rates: &[(&str, &str)]) -> Result<(), FinanceError> {
    let main = main_currency(conn)?.ok_or(FinanceError::NoMainCurrency)?;
    let tx = conn.unchecked_transaction()?;
    for (code, text) in rates {
        save(&tx, main, code, text)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn set_rate(conn: &Connection, code: &str, text: &str) -> Result<(), FinanceError> {
    set_rates(conn, &[(code, text)])
}
