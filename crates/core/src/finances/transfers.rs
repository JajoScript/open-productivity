// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Transfers of money between two accounts. They are not income nor expense.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::FinanceError;
use super::accounts::{AccountId, check_account, get_account};
use super::money::{Currency, currency, parse_amount};
use super::transactions::{DATE_FORMAT, Movement, SELECT, TransactionId, row};

#[derive(Debug, PartialEq)]
pub struct Transfer {
    pub id: TransactionId,
    pub from: AccountId,
    pub from_name: String,
    pub to: AccountId,
    pub to_name: String,
    /// The source account's currency.
    pub currency: &'static Currency,
    /// Always > 0, in the source's currency.
    pub amount: i64,
    pub to_currency: &'static Currency,
    /// What arrives, in the destination's currency; equals `amount` between the same currency.
    pub to_amount: i64,
    pub date: NaiveDate,
    pub description: String,
}

/// Reads a `SELECT` row of kind transfer.
pub(super) fn transfer_row(r: &rusqlite::Row) -> rusqlite::Result<Result<Transfer, FinanceError>> {
    let code: String = r.get(4)?;
    let date: String = r.get(6)?;
    let to: Option<i64> = r.get(10)?;
    let to_name: Option<String> = r.get(11)?;
    let to_amount: Option<i64> = r.get(12)?;
    let to_code: Option<String> = r.get(13)?;
    let (id, from, from_name, amount, description) =
        (r.get(0)?, r.get(2)?, r.get(3)?, r.get(5)?, r.get(7)?);
    Ok((|| {
        let to_code = to_code.ok_or(FinanceError::AccountNotFound)?;
        Ok(Transfer {
            id: TransactionId(id),
            from: AccountId(from),
            from_name,
            to: AccountId(to.ok_or(FinanceError::AccountNotFound)?),
            to_name: to_name.unwrap_or_default(),
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            amount,
            to_amount: to_amount.unwrap_or(amount),
            to_currency: currency(&to_code).ok_or(FinanceError::UnknownCurrency(to_code))?,
            date: NaiveDate::parse_from_str(&date, DATE_FORMAT)
                .map_err(|_| FinanceError::BadDate(date))?,
            description,
        })
    })())
}

/// Different, existing accounts; the amount (> 0) is read in the source's currency. Between
/// different currencies `to_amount` (> 0, in the destination's) is required; otherwise it is
/// ignored.
fn validate<'a>(
    conn: &Connection,
    from: AccountId,
    to: AccountId,
    amount: &str,
    to_amount: &str,
    description: &'a str,
) -> Result<(i64, Option<i64>, &'a str), FinanceError> {
    let c = get_account(conn, from)?.currency;
    let to_c = get_account(conn, to)?.currency;
    if from == to {
        return Err(FinanceError::SameAccount);
    }
    let positive = |text, c| parse_amount(text, c).filter(|a| *a > 0);
    let amount = positive(amount, c).ok_or(FinanceError::InvalidAmount)?;
    let to_amount = if c.code == to_c.code {
        None
    } else {
        Some(positive(to_amount, to_c).ok_or(FinanceError::InvalidAmount)?)
    };
    Ok((amount, to_amount, description.trim()))
}

pub fn create_transfer(
    conn: &Connection,
    from: AccountId,
    to: AccountId,
    amount: &str,
    to_amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<TransactionId, FinanceError> {
    let (amount, to_amount, description) =
        validate(conn, from, to, amount, to_amount, description)?;
    check_account(conn, from, None)?;
    check_account(conn, to, None)?;
    conn.execute(
        "INSERT INTO finance_transaction
             (kind, account_id, to_account_id, amount, to_amount, date, description)
         VALUES ('transfer', ?1, ?2, ?3, ?4, ?5, ?6)",
        (
            from.0,
            to.0,
            amount,
            to_amount,
            date.format(DATE_FORMAT).to_string(),
            description,
        ),
    )?;
    Ok(TransactionId(conn.last_insert_rowid()))
}

// One argument per field the form edits; a struct would only move the list.
#[allow(clippy::too_many_arguments)]
pub fn update_transfer(
    conn: &Connection,
    id: TransactionId,
    from: AccountId,
    to: AccountId,
    amount: &str,
    to_amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<(), FinanceError> {
    let current = get_transfer(conn, id)?;
    let (amount, to_amount, description) =
        validate(conn, from, to, amount, to_amount, description)?;
    check_account(conn, from, Some(current.from))?;
    check_account(conn, to, Some(current.to))?;
    conn.execute(
        "UPDATE finance_transaction
         SET account_id = ?1, to_account_id = ?2, amount = ?3, to_amount = ?4, date = ?5,
             description = ?6
         WHERE id = ?7",
        (
            from.0,
            to.0,
            amount,
            to_amount,
            date.format(DATE_FORMAT).to_string(),
            description,
            id.0,
        ),
    )?;
    Ok(())
}

pub fn get_transfer(conn: &Connection, id: TransactionId) -> Result<Transfer, FinanceError> {
    match conn.query_row(
        &format!("{SELECT} WHERE t.id = ?1 AND t.kind = 'transfer'"),
        [id.0],
        row,
    ) {
        Ok(Ok(Movement::Transfer(t))) => Ok(t),
        Ok(Ok(Movement::Tx(_))) => Err(FinanceError::TransactionNotFound),
        Ok(Err(e)) => Err(e),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::TransactionNotFound),
        Err(e) => Err(e.into()),
    }
}
