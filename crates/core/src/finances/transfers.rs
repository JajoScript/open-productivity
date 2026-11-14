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
    /// Always > 0.
    pub amount: i64,
    pub date: NaiveDate,
    pub description: String,
}

/// Reads a `SELECT` row of kind transfer.
pub(super) fn transfer_row(r: &rusqlite::Row) -> rusqlite::Result<Result<Transfer, FinanceError>> {
    let code: String = r.get(4)?;
    let date: String = r.get(6)?;
    let to: Option<i64> = r.get(10)?;
    let to_name: Option<String> = r.get(11)?;
    let (id, from, from_name, amount, description) =
        (r.get(0)?, r.get(2)?, r.get(3)?, r.get(5)?, r.get(7)?);
    Ok((|| {
        Ok(Transfer {
            id: TransactionId(id),
            from: AccountId(from),
            from_name,
            to: AccountId(to.ok_or(FinanceError::AccountNotFound)?),
            to_name: to_name.unwrap_or_default(),
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            amount,
            date: NaiveDate::parse_from_str(&date, DATE_FORMAT)
                .map_err(|_| FinanceError::BadDate(date))?,
            description,
        })
    })())
}

/// Different, existing accounts; the amount (> 0) is read in the source's currency.
fn validate<'a>(
    conn: &Connection,
    from: AccountId,
    to: AccountId,
    amount: &str,
    description: &'a str,
) -> Result<(i64, &'a str), FinanceError> {
    let c = get_account(conn, from)?.currency;
    get_account(conn, to)?;
    if from == to {
        return Err(FinanceError::SameAccount);
    }
    match parse_amount(amount, c) {
        Some(a) if a > 0 => Ok((a, description.trim())),
        _ => Err(FinanceError::InvalidAmount),
    }
}

pub fn create_transfer(
    conn: &Connection,
    from: AccountId,
    to: AccountId,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<TransactionId, FinanceError> {
    let (amount, description) = validate(conn, from, to, amount, description)?;
    check_account(conn, from, None)?;
    check_account(conn, to, None)?;
    conn.execute(
        "INSERT INTO finance_transaction (kind, account_id, to_account_id, amount, date, description)
         VALUES ('transfer', ?1, ?2, ?3, ?4, ?5)",
        (
            from.0,
            to.0,
            amount,
            date.format(DATE_FORMAT).to_string(),
            description,
        ),
    )?;
    Ok(TransactionId(conn.last_insert_rowid()))
}

pub fn update_transfer(
    conn: &Connection,
    id: TransactionId,
    from: AccountId,
    to: AccountId,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<(), FinanceError> {
    let current = get_transfer(conn, id)?;
    let (amount, description) = validate(conn, from, to, amount, description)?;
    check_account(conn, from, Some(current.from))?;
    check_account(conn, to, Some(current.to))?;
    conn.execute(
        "UPDATE finance_transaction
         SET account_id = ?1, to_account_id = ?2, amount = ?3, date = ?4, description = ?5
         WHERE id = ?6",
        (
            from.0,
            to.0,
            amount,
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
