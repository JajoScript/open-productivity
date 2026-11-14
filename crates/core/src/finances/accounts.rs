// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Accounts and the instance's main currency.

use rusqlite::Connection;

use super::FinanceError;
use super::money::{Currency, currency, parse_amount};
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

/// The account is born in the main currency.
pub fn create_account(
    conn: &Connection,
    name: &str,
    kind: AccountKind,
    bank: &str,
    initial_balance: &str,
) -> Result<AccountId, FinanceError> {
    let c = main_currency(conn)?.ok_or(FinanceError::NoMainCurrency)?;
    let (name, bank, amount) = validate(name, bank, initial_balance, c)?;
    conn.execute(
        "INSERT INTO finance_account (name, kind, bank, currency, initial_balance)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (name, kind.as_str(), bank, c.code, amount),
    )?;
    Ok(AccountId(conn.last_insert_rowid()))
}

/// The currency never changes; the amount is read in the account's currency.
pub fn update_account(
    conn: &Connection,
    id: AccountId,
    name: &str,
    kind: AccountKind,
    bank: &str,
    initial_balance: &str,
) -> Result<(), FinanceError> {
    let c = get_account(conn, id)?.currency;
    let (name, bank, amount) = validate(name, bank, initial_balance, c)?;
    conn.execute(
        "UPDATE finance_account SET name = ?1, kind = ?2, bank = ?3, initial_balance = ?4
         WHERE id = ?5",
        (name, kind.as_str(), bank, amount, id.0),
    )?;
    Ok(())
}

const SELECT: &str = "SELECT id, name, kind, bank, currency, initial_balance,
    initial_balance AS balance FROM finance_account";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Account, FinanceError>> {
    let kind: String = r.get(2)?;
    let code: String = r.get(4)?;
    let (id, name, bank, initial_balance, balance) =
        (r.get(0)?, r.get(1)?, r.get(3)?, r.get(5)?, r.get(6)?);
    Ok(AccountKind::parse(&kind).and_then(|kind| {
        Ok(Account {
            id: AccountId(id),
            name,
            kind,
            bank,
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            initial_balance,
            balance,
        })
    }))
}

/// In creation order.
pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>, FinanceError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY created_at, id"))?;
    let rows = stmt.query_map([], row)?;
    rows.map(|r| r?).collect()
}

pub fn get_account(conn: &Connection, id: AccountId) -> Result<Account, FinanceError> {
    match conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id.0], row) {
        Ok(account) => account,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::AccountNotFound),
        Err(e) => Err(e.into()),
    }
}
