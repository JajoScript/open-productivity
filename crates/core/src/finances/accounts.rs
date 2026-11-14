// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Accounts and the instance's main currency.

use rusqlite::Connection;

use super::FinanceError;
use super::money::{Currency, currency, parse_amount};
use super::rates::ensure_rate;
use crate::instance_settings;

const MAIN_CURRENCY_KEY: &str = "finance.main_currency";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccountId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccountKind {
    Checking,
    Demand,
    Savings,
    Cash,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Checking => "checking",
            AccountKind::Demand => "demand",
            AccountKind::Savings => "savings",
            AccountKind::Cash => "cash",
        }
    }

    pub fn parse(s: &str) -> Result<Self, FinanceError> {
        Ok(match s {
            "checking" => AccountKind::Checking,
            "demand" => AccountKind::Demand,
            "savings" => AccountKind::Savings,
            "cash" => AccountKind::Cash,
            other => return Err(FinanceError::BadKind(other.to_string())),
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub kind: AccountKind,
    pub bank: String,
    pub currency: &'static Currency,
    pub initial_balance: i64,
    pub balance: i64,
    pub archived: bool,
    /// Some movement uses it, as source or destination.
    pub in_use: bool,
}

pub fn main_currency(conn: &Connection) -> Result<Option<&'static Currency>, FinanceError> {
    match instance_settings::get(conn, MAIN_CURRENCY_KEY)? {
        None => Ok(None),
        Some(code) => currency(&code)
            .map(Some)
            .ok_or(FinanceError::UnknownCurrency(code)),
    }
}

pub fn set_main_currency(conn: &Connection, code: &str) -> Result<(), FinanceError> {
    let c = currency(code).ok_or_else(|| FinanceError::UnknownCurrency(code.to_string()))?;
    instance_settings::set(conn, MAIN_CURRENCY_KEY, c.code)?;
    Ok(())
}

/// Trims name and bank, rejects an empty name and reads the amount in `c`.
fn validate<'a>(
    name: &'a str,
    bank: &'a str,
    amount: &str,
    c: &Currency,
) -> Result<(&'a str, &'a str, i64), FinanceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FinanceError::EmptyName);
    }
    let amount = parse_amount(amount, c).ok_or(FinanceError::InvalidAmount)?;
    Ok((name, bank.trim(), amount))
}

fn pick_currency(code: &str) -> Result<&'static Currency, FinanceError> {
    currency(code).ok_or_else(|| FinanceError::UnknownCurrency(code.to_string()))
}

/// `currency` is any code of the list. A non-main currency without a rate yet needs `rate`,
/// which is saved in the same transaction as the account.
pub fn create_account(
    conn: &Connection,
    name: &str,
    kind: AccountKind,
    bank: &str,
    currency: &str,
    initial_balance: &str,
    rate: Option<&str>,
) -> Result<AccountId, FinanceError> {
    let main = main_currency(conn)?.ok_or(FinanceError::NoMainCurrency)?;
    let c = pick_currency(currency)?;
    let (name, bank, amount) = validate(name, bank, initial_balance, c)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO finance_account (name, kind, bank, currency, initial_balance)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (name, kind.as_str(), bank, c.code, amount),
    )?;
    let id = AccountId(tx.last_insert_rowid());
    ensure_rate(&tx, main, c, rate)?;
    tx.commit()?;
    Ok(id)
}

/// The currency can change only while the account has no movements; the amount is read in it.
// One argument per field the form edits; a struct would only move the list.
#[allow(clippy::too_many_arguments)]
pub fn update_account(
    conn: &Connection,
    id: AccountId,
    name: &str,
    kind: AccountKind,
    bank: &str,
    currency: &str,
    initial_balance: &str,
    rate: Option<&str>,
) -> Result<(), FinanceError> {
    let main = main_currency(conn)?.ok_or(FinanceError::NoMainCurrency)?;
    let current = get_account(conn, id)?;
    let c = pick_currency(currency)?;
    if c.code != current.currency.code && (current.in_use || pending_schedules(conn, id)? > 0) {
        return Err(FinanceError::CurrencyLocked);
    }
    let (name, bank, amount) = validate(name, bank, initial_balance, c)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE finance_account
         SET name = ?1, kind = ?2, bank = ?3, currency = ?4, initial_balance = ?5
         WHERE id = ?6",
        (name, kind.as_str(), bank, c.code, amount, id.0),
    )?;
    if c.code != current.currency.code {
        ensure_rate(&tx, main, c, rate)?;
    }
    tx.commit()?;
    Ok(())
}

/// The balance is summed in SQL; an integer overflow there is an error, not a panic.
const SELECT: &str = "SELECT a.id, a.name, a.kind, a.bank, a.currency, a.initial_balance,
    a.initial_balance + coalesce((SELECT sum(CASE WHEN t.account_id = a.id
            THEN CASE t.kind WHEN 'income' THEN t.amount ELSE -t.amount END
            ELSE coalesce(t.to_amount, t.amount) END)
        FROM finance_transaction t WHERE t.account_id = a.id OR t.to_account_id = a.id), 0)
        AS balance, a.archived,
    EXISTS (SELECT 1 FROM finance_transaction t WHERE t.account_id = a.id OR t.to_account_id = a.id)
    FROM finance_account a";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Account, FinanceError>> {
    let kind: String = r.get(2)?;
    let code: String = r.get(4)?;
    let (id, name, bank, initial_balance, balance) =
        (r.get(0)?, r.get(1)?, r.get(3)?, r.get(5)?, r.get(6)?);
    let (archived, in_use): (bool, bool) = (r.get(7)?, r.get(8)?);
    Ok(AccountKind::parse(&kind).and_then(|kind| {
        Ok(Account {
            id: AccountId(id),
            name,
            kind,
            bank,
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            initial_balance,
            balance,
            archived,
            in_use,
        })
    }))
}

/// In creation order.
pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>, FinanceError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY a.created_at, a.id"))?;
    let rows = stmt.query_map([], row)?;
    rows.map(|r| r?).collect()
}

pub fn get_account(conn: &Connection, id: AccountId) -> Result<Account, FinanceError> {
    match conn.query_row(&format!("{SELECT} WHERE a.id = ?1"), [id.0], row) {
        Ok(account) => account,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::AccountNotFound),
        Err(e) => Err(e.into()),
    }
}

pub fn set_account_archived(
    conn: &Connection,
    id: AccountId,
    archived: bool,
) -> Result<(), FinanceError> {
    if archived {
        match pending_schedules(conn, id)? {
            0 => {}
            n => return Err(FinanceError::AccountHasSchedules(n)),
        }
    }
    match conn.execute(
        "UPDATE finance_account SET archived = ?1 WHERE id = ?2",
        (archived, id.0),
    )? {
        0 => Err(FinanceError::AccountNotFound),
        _ => Ok(()),
    }
}

/// Only an account without movements (as source or destination) can be deleted.
pub fn delete_account(conn: &Connection, id: AccountId) -> Result<(), FinanceError> {
    if get_account(conn, id)?.in_use {
        return Err(FinanceError::AccountInUse);
    }
    // Finished payments still reference the account.
    let all: i64 = conn.query_row(
        "SELECT count(*) FROM finance_scheduled WHERE account_id = ?1",
        [id.0],
        |r| r.get(0),
    )?;
    if all > 0 {
        return Err(FinanceError::AccountHasSchedules(all));
    }
    conn.execute("DELETE FROM finance_account WHERE id = ?1", [id.0])?;
    Ok(())
}

/// The account must exist and, unless the movement already had it, not be archived.
pub(super) fn check_account(
    conn: &Connection,
    id: AccountId,
    current: Option<AccountId>,
) -> Result<(), FinanceError> {
    if get_account(conn, id)?.archived && current != Some(id) {
        return Err(FinanceError::AccountArchived);
    }
    Ok(())
}

/// How many scheduled payments of the account are not finished.
fn pending_schedules(conn: &Connection, id: AccountId) -> Result<i64, FinanceError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM finance_scheduled WHERE account_id = ?1 AND done = 0",
        [id.0],
        |r| r.get(0),
    )?)
}
