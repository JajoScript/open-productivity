// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Reserves: named sub-balances inside a plain account. Reserving moves no money and creates no
//! movement; it only lowers what the account has available.

use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::FinanceError;
use super::accounts::{AccountId, AccountKind, get_account};
use super::money::{Currency, currency, parse_amount};
use super::transactions::DATE_FORMAT;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReserveId(pub i64);

#[derive(Debug, PartialEq)]
pub struct Reserve {
    pub id: ReserveId,
    pub name: String,
    pub account: AccountId,
    pub account_name: String,
    pub account_archived: bool,
    /// The account's currency, also of `balance`.
    pub currency: &'static Currency,
    /// Deposits minus withdrawals.
    pub balance: i64,
    pub archived: bool,
    /// Has moves, so it cannot be deleted.
    pub in_use: bool,
}

/// One deposit or withdrawal, in the account's currency.
#[derive(Debug, PartialEq)]
pub struct ReserveMove {
    pub date: NaiveDate,
    pub description: String,
    pub withdraw: bool,
    pub amount: i64,
}

const SELECT: &str = "SELECT r.id, r.name, r.account_id, a.name, a.archived, a.currency,
    coalesce((SELECT sum(CASE m.kind WHEN 'deposit' THEN m.amount ELSE -m.amount END)
        FROM finance_reserve_move m WHERE m.reserve_id = r.id), 0),
    r.archived,
    EXISTS (SELECT 1 FROM finance_reserve_move m WHERE m.reserve_id = r.id)
    FROM finance_reserve r JOIN finance_account a ON a.id = r.account_id";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Reserve, FinanceError>> {
    let code: String = r.get(5)?;
    let (id, name, account, account_name) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (account_archived, balance, archived, in_use) =
        (r.get(4)?, r.get(6)?, r.get(7)?, r.get(8)?);
    Ok(match currency(&code) {
        Some(currency) => Ok(Reserve {
            id: ReserveId(id),
            name,
            account: AccountId(account),
            account_name,
            account_archived,
            currency,
            balance,
            archived,
            in_use,
        }),
        None => Err(FinanceError::UnknownCurrency(code)),
    })
}

fn query(conn: &Connection, archived: bool) -> Result<Vec<Reserve>, FinanceError> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE r.archived = ?1 ORDER BY r.created_at, r.id"
    ))?;
    let rows = stmt.query_map([archived], row)?;
    rows.map(|r| r?).collect()
}

/// Active reserves in creation order.
pub fn list_reserves(conn: &Connection) -> Result<Vec<Reserve>, FinanceError> {
    query(conn, false)
}

/// Archived reserves in creation order; their balance is 0.
pub fn list_archived_reserves(conn: &Connection) -> Result<Vec<Reserve>, FinanceError> {
    query(conn, true)
}

pub fn get_reserve(conn: &Connection, id: ReserveId) -> Result<Reserve, FinanceError> {
    match conn.query_row(&format!("{SELECT} WHERE r.id = ?1"), [id.0], row) {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::ReserveNotFound),
        Err(e) => Err(e.into()),
    }
}

fn trimmed(name: &str) -> Result<&str, FinanceError> {
    match name.trim() {
        "" => Err(FinanceError::EmptyName),
        n => Ok(n),
    }
}

/// An empty reserve in an active plain account (not a card nor a credit line).
pub fn create_reserve(
    conn: &Connection,
    name: &str,
    account: AccountId,
) -> Result<ReserveId, FinanceError> {
    let name = trimmed(name)?;
    let a = get_account(conn, account)?;
    if matches!(a.kind, AccountKind::Credit | AccountKind::CreditLine) {
        return Err(FinanceError::KindLocked);
    }
    if a.archived {
        return Err(FinanceError::AccountArchived);
    }
    conn.execute(
        "INSERT INTO finance_reserve (name, account_id) VALUES (?1, ?2)",
        (name, account.0),
    )?;
    Ok(ReserveId(conn.last_insert_rowid()))
}

pub fn rename_reserve(conn: &Connection, id: ReserveId, name: &str) -> Result<(), FinanceError> {
    match conn.execute(
        "UPDATE finance_reserve SET name = ?1 WHERE id = ?2",
        (trimmed(name)?, id.0),
    )? {
        0 => Err(FinanceError::ReserveNotFound),
        _ => Ok(()),
    }
}

/// Archiving needs a balance of 0.
pub fn set_reserve_archived(
    conn: &Connection,
    id: ReserveId,
    archived: bool,
) -> Result<(), FinanceError> {
    if archived && get_reserve(conn, id)?.balance != 0 {
        return Err(FinanceError::ReserveNotEmpty);
    }
    match conn.execute(
        "UPDATE finance_reserve SET archived = ?1 WHERE id = ?2",
        (archived, id.0),
    )? {
        0 => Err(FinanceError::ReserveNotFound),
        _ => Ok(()),
    }
}

/// Only a reserve without moves can be deleted.
pub fn delete_reserve(conn: &Connection, id: ReserveId) -> Result<(), FinanceError> {
    if get_reserve(conn, id)?.in_use {
        return Err(FinanceError::ReserveInUse);
    }
    conn.execute("DELETE FROM finance_reserve WHERE id = ?1", [id.0])?;
    Ok(())
}

/// Newest first.
pub fn list_moves(conn: &Connection, id: ReserveId) -> Result<Vec<ReserveMove>, FinanceError> {
    let mut stmt = conn.prepare(
        "SELECT date, description, kind, amount FROM finance_reserve_move
         WHERE reserve_id = ?1 ORDER BY date DESC, id DESC",
    )?;
    let rows = stmt.query_map([id.0], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get(1)?,
            r.get::<_, String>(2)?,
            r.get(3)?,
        ))
    })?;
    rows.map(|r| {
        let (date, description, kind, amount) = r?;
        Ok(ReserveMove {
            date: NaiveDate::parse_from_str(&date, DATE_FORMAT)
                .map_err(|_| FinanceError::BadDate(date))?,
            description,
            withdraw: kind == "withdraw",
            amount,
        })
    })
    .collect()
}

/// One move in one transaction: the limit is checked against the state it is written to.
fn add_move(
    conn: &Connection,
    id: ReserveId,
    withdraw: bool,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<(), FinanceError> {
    let tx = conn.unchecked_transaction()?;
    let reserve = get_reserve(&tx, id)?;
    let amount = parse_amount(amount, reserve.currency)
        .filter(|a| *a > 0)
        .ok_or(FinanceError::InvalidAmount)?;
    let limit = if withdraw {
        reserve.balance
    } else {
        if reserve.archived {
            return Err(FinanceError::ReserveNotFound);
        }
        if reserve.account_archived {
            return Err(FinanceError::AccountArchived);
        }
        get_account(&tx, reserve.account)?.available()
    };
    if amount > limit {
        return Err(FinanceError::Insufficient);
    }
    tx.execute(
        "INSERT INTO finance_reserve_move (reserve_id, kind, amount, date, description)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id.0,
            if withdraw { "withdraw" } else { "deposit" },
            amount,
            date.format(DATE_FORMAT).to_string(),
            description.trim()
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Reserve: sets aside `amount` (in the account's currency) of the account's available.
pub fn reserve_money(
    conn: &Connection,
    id: ReserveId,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<(), FinanceError> {
    add_move(conn, id, false, amount, date, description)
}

/// Withdraw: gives `amount` of the reserve back to the account's available.
pub fn withdraw_from_reserve(
    conn: &Connection,
    id: ReserveId,
    amount: &str,
    date: NaiveDate,
    description: &str,
) -> Result<(), FinanceError> {
    add_move(conn, id, true, amount, date, description)
}
