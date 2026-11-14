// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Expenses and income of an account.

use chrono::{Months, NaiveDate};
use rusqlite::Connection;

use super::FinanceError;
use super::accounts::{AccountId, get_account};
use super::money::{Currency, currency, parse_amount};
use crate::calendar::month_start;

const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransactionId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TxKind {
    Expense,
    Income,
}

impl TxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TxKind::Expense => "expense",
            TxKind::Income => "income",
        }
    }

    pub fn parse(s: &str) -> Result<Self, FinanceError> {
        Ok(match s {
            "expense" => TxKind::Expense,
            "income" => TxKind::Income,
            other => return Err(FinanceError::BadTxKind(other.to_string())),
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Transaction {
    pub id: TransactionId,
    pub kind: TxKind,
    pub account_id: AccountId,
    pub account_name: String,
    pub currency: &'static Currency,
    /// Always > 0; `signed` carries the direction.
    pub amount: i64,
    pub date: NaiveDate,
    pub description: String,
}

impl Transaction {
    /// Negative for an expense.
    pub fn signed(&self) -> i64 {
        match self.kind {
            TxKind::Expense => -self.amount,
            TxKind::Income => self.amount,
        }
    }
}

/// Reads the amount in the account's currency (> 0) and trims the description.
fn validate<'a>(
    conn: &Connection,
    account: AccountId,
    amount: &str,
    description: &'a str,
) -> Result<(i64, &'a str), FinanceError> {
    let c = get_account(conn, account)?.currency;
    match parse_amount(amount, c) {
        Some(a) if a > 0 => Ok((a, description.trim())),
        _ => Err(FinanceError::InvalidAmount),
    }
}

pub fn create_transaction(
    conn: &Connection,
    kind: TxKind,
    account: AccountId,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<TransactionId, FinanceError> {
    let (amount, description) = validate(conn, account, amount, description)?;
    conn.execute(
        "INSERT INTO finance_transaction (kind, account_id, amount, date, description)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            kind.as_str(),
            account.0,
            amount,
            date.format(DATE_FORMAT).to_string(),
            description,
        ),
    )?;
    Ok(TransactionId(conn.last_insert_rowid()))
}

pub fn update_transaction(
    conn: &Connection,
    id: TransactionId,
    kind: TxKind,
    account: AccountId,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<(), FinanceError> {
    get_transaction(conn, id)?;
    let (amount, description) = validate(conn, account, amount, description)?;
    conn.execute(
        "UPDATE finance_transaction
         SET kind = ?1, account_id = ?2, amount = ?3, date = ?4, description = ?5
         WHERE id = ?6",
        (
            kind.as_str(),
            account.0,
            amount,
            date.format(DATE_FORMAT).to_string(),
            description,
            id.0,
        ),
    )?;
    Ok(())
}

pub fn delete_transaction(conn: &Connection, id: TransactionId) -> Result<(), FinanceError> {
    match conn.execute("DELETE FROM finance_transaction WHERE id = ?1", [id.0])? {
        0 => Err(FinanceError::TransactionNotFound),
        _ => Ok(()),
    }
}

const SELECT: &str = "SELECT t.id, t.kind, t.account_id, a.name, a.currency, t.amount, t.date,
    t.description FROM finance_transaction t JOIN finance_account a ON a.id = t.account_id";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Transaction, FinanceError>> {
    let kind: String = r.get(1)?;
    let code: String = r.get(4)?;
    let date: String = r.get(6)?;
    let (id, account_id, account_name, amount, description) =
        (r.get(0)?, r.get(2)?, r.get(3)?, r.get(5)?, r.get(7)?);
    Ok(TxKind::parse(&kind).and_then(|kind| {
        Ok(Transaction {
            id: TransactionId(id),
            kind,
            account_id: AccountId(account_id),
            account_name,
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            amount,
            date: NaiveDate::parse_from_str(&date, DATE_FORMAT)
                .map_err(|_| FinanceError::BadDate(date))?,
            description,
        })
    }))
}

pub fn get_transaction(conn: &Connection, id: TransactionId) -> Result<Transaction, FinanceError> {
    match conn.query_row(&format!("{SELECT} WHERE t.id = ?1"), [id.0], row) {
        Ok(t) => t,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::TransactionNotFound),
        Err(e) => Err(e.into()),
    }
}

/// The movements of the month holding `month` (any day of it works: it is normalized to day 1),
/// newest first.
pub fn list_month(conn: &Connection, month: NaiveDate) -> Result<Vec<Transaction>, FinanceError> {
    let from = month_start(month);
    let Some(to) = from.checked_add_months(Months::new(1)) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE t.date >= ?1 AND t.date < ?2 ORDER BY t.date DESC, t.id DESC"
    ))?;
    let rows = stmt.query_map(
        [
            from.format(DATE_FORMAT).to_string(),
            to.format(DATE_FORMAT).to_string(),
        ],
        row,
    )?;
    rows.map(|r| r?).collect()
}
