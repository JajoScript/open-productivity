// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Expenses and income of an account.

use chrono::{Months, NaiveDate};
use rusqlite::Connection;

use super::FinanceError;
use super::accounts::{AccountId, check_account, get_account};
use super::categories::{CategoryId, get_category};
use super::money::{Currency, currency, parse_amount};
use super::scheduled::ScheduledId;
use super::transfers::{Transfer, transfer_row};
use crate::calendar::month_start;

pub(super) const DATE_FORMAT: &str = "%Y-%m-%d";

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
    pub category: Option<CategoryId>,
    pub category_name: Option<String>,
    /// The scheduled payment that created it, if it still exists.
    pub scheduled: Option<ScheduledLink>,
}

/// What a movement that came from a scheduled payment tells the UI.
#[derive(Debug, PartialEq)]
pub struct ScheduledLink {
    pub id: ScheduledId,
    /// (number, total) when it paid an installment.
    pub installment: Option<(i64, i64)>,
    /// Deleting it can return the payment to the schedule (`delete_transaction`'s `restore`).
    pub restorable: bool,
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

/// The category must exist, match the movement's kind and, unless the movement already had it,
/// not be archived.
pub(super) fn check_category(
    conn: &Connection,
    category: Option<CategoryId>,
    kind: TxKind,
    current: Option<CategoryId>,
) -> Result<(), FinanceError> {
    let Some(id) = category else { return Ok(()) };
    let c = get_category(conn, id)?;
    if c.kind != kind {
        return Err(FinanceError::CategoryKindMismatch);
    }
    if c.archived && current != category {
        return Err(FinanceError::CategoryArchived);
    }
    Ok(())
}

pub fn create_transaction(
    conn: &Connection,
    kind: TxKind,
    account: AccountId,
    amount: &str,
    date: NaiveDate,
    description: &str,
    category: Option<CategoryId>,
) -> Result<TransactionId, FinanceError> {
    let (amount, description) = validate(conn, account, amount, description)?;
    check_account(conn, account, None)?;
    check_category(conn, category, kind, None)?;
    insert(
        conn,
        kind,
        account,
        amount,
        date,
        description,
        category,
        None,
    )
}

/// Inserts an already validated movement; `from` is the scheduled payment and installment number
/// that created it.
#[allow(clippy::too_many_arguments)]
pub(super) fn insert(
    conn: &Connection,
    kind: TxKind,
    account: AccountId,
    amount: i64,
    date: NaiveDate,
    description: &str,
    category: Option<CategoryId>,
    from: Option<(ScheduledId, Option<i64>)>,
) -> Result<TransactionId, FinanceError> {
    conn.execute(
        "INSERT INTO finance_transaction
             (kind, account_id, amount, date, description, category_id, scheduled_id, installment)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        (
            kind.as_str(),
            account.0,
            amount,
            date.format(DATE_FORMAT).to_string(),
            description,
            category.map(|c| c.0),
            from.map(|f| f.0.0),
            from.and_then(|f| f.1),
        ),
    )?;
    Ok(TransactionId(conn.last_insert_rowid()))
}

// One argument per field the form edits; a struct would only move the list.
#[allow(clippy::too_many_arguments)]
pub fn update_transaction(
    conn: &Connection,
    id: TransactionId,
    kind: TxKind,
    account: AccountId,
    amount: &str,
    date: NaiveDate,
    description: &str,
    category: Option<CategoryId>,
) -> Result<(), FinanceError> {
    let current = get_transaction(conn, id)?;
    let (amount, description) = validate(conn, account, amount, description)?;
    check_account(conn, account, Some(current.account_id))?;
    check_category(conn, category, kind, current.category)?;
    conn.execute(
        "UPDATE finance_transaction
         SET kind = ?1, account_id = ?2, amount = ?3, date = ?4, description = ?5,
             category_id = ?6
         WHERE id = ?7",
        (
            kind.as_str(),
            account.0,
            amount,
            date.format(DATE_FORMAT).to_string(),
            description,
            category.map(|c| c.0),
            id.0,
        ),
    )?;
    Ok(())
}

/// With `restore`, a movement that came from a one-time or installments schedule returns its
/// payment: the schedule is pending again (an installment goes back to the unpaid ones, at the
/// end of the series). A recurring schedule has nothing to return.
pub fn delete_transaction(
    conn: &Connection,
    id: TransactionId,
    restore: bool,
) -> Result<(), FinanceError> {
    let tx = conn.unchecked_transaction()?;
    let scheduled: Option<i64> = match tx.query_row(
        "SELECT scheduled_id FROM finance_transaction WHERE id = ?1",
        [id.0],
        |r| r.get(0),
    ) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(FinanceError::TransactionNotFound);
        }
        Err(e) => return Err(e.into()),
    };
    tx.execute("DELETE FROM finance_transaction WHERE id = ?1", [id.0])?;
    if let (true, Some(sid)) = (restore, scheduled) {
        tx.execute(
            "UPDATE finance_scheduled
             SET done = 0, installments_paid = CASE form
                 WHEN 'installments' THEN max(installments_paid - 1, 0) ELSE installments_paid END
             WHERE id = ?1 AND form IN ('once', 'installments')",
            [sid],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) const SELECT: &str =
    "SELECT t.id, t.kind, t.account_id, a.name, a.currency, t.amount, t.date,
    t.description, t.category_id, c.name, t.to_account_id, b.name, t.to_amount, b.currency,
    t.scheduled_id, t.installment, s.form, s.installments_total, s.installments_paid, s.done
    FROM finance_transaction t
    JOIN finance_account a ON a.id = t.account_id
    LEFT JOIN finance_account b ON b.id = t.to_account_id
    LEFT JOIN finance_category c ON c.id = t.category_id
    LEFT JOIN finance_scheduled s ON s.id = t.scheduled_id";

/// A movement of the month list: a transaction or a transfer between accounts.
#[derive(Debug, PartialEq)]
pub enum Movement {
    Tx(Transaction),
    Transfer(Transfer),
}

pub(super) fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Movement, FinanceError>> {
    let kind: String = r.get(1)?;
    if kind == "transfer" {
        return Ok(transfer_row(r)?.map(Movement::Transfer));
    }
    let code: String = r.get(4)?;
    let date: String = r.get(6)?;
    let (id, account_id, account_name, amount, description) =
        (r.get(0)?, r.get(2)?, r.get(3)?, r.get(5)?, r.get(7)?);
    let (category, category_name): (Option<i64>, Option<String>) = (r.get(8)?, r.get(9)?);
    let scheduled: Option<i64> = r.get(14)?;
    let number: Option<i64> = r.get(15)?;
    let form: Option<String> = r.get(16)?;
    let total: Option<i64> = r.get(17)?;
    let (paid, done): (Option<i64>, Option<bool>) = (r.get(18)?, r.get(19)?);
    let scheduled = scheduled.map(|sid| ScheduledLink {
        id: ScheduledId(sid),
        installment: number.zip(total),
        restorable: match form.as_deref() {
            Some("once") => done == Some(true),
            Some("installments") => paid.is_some_and(|p| p > 0),
            _ => false,
        },
    });
    Ok(TxKind::parse(&kind).and_then(|kind| {
        Ok(Movement::Tx(Transaction {
            id: TransactionId(id),
            kind,
            account_id: AccountId(account_id),
            account_name,
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            amount,
            date: NaiveDate::parse_from_str(&date, DATE_FORMAT)
                .map_err(|_| FinanceError::BadDate(date))?,
            description,
            category: category.map(CategoryId),
            category_name,
            scheduled,
        }))
    }))
}

pub fn get_transaction(conn: &Connection, id: TransactionId) -> Result<Transaction, FinanceError> {
    match conn.query_row(
        &format!("{SELECT} WHERE t.id = ?1 AND t.kind <> 'transfer'"),
        [id.0],
        row,
    ) {
        Ok(Ok(Movement::Tx(t))) => Ok(t),
        Ok(Ok(Movement::Transfer(_))) => Err(FinanceError::TransactionNotFound),
        Ok(Err(e)) => Err(e),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::TransactionNotFound),
        Err(e) => Err(e.into()),
    }
}

/// The movements of the month holding `month` (any day of it works: it is normalized to day 1),
/// newest first.
pub fn list_month(conn: &Connection, month: NaiveDate) -> Result<Vec<Movement>, FinanceError> {
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
