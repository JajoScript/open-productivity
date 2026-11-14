// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Finances: accounts and their balance, in the instance's main currency.

use std::fmt;

mod accounts;
mod money;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_transactions;
mod transactions;

pub use accounts::*;
pub use money::*;
pub use transactions::*;

#[derive(Debug)]
pub enum FinanceError {
    EmptyName,
    /// The amount is empty, malformed, negative or too large.
    InvalidAmount,
    NoMainCurrency,
    UnknownCurrency(String),
    /// A stored account kind that cannot be read.
    BadKind(String),
    AccountNotFound,
    /// A stored movement kind that cannot be read.
    BadTxKind(String),
    /// A stored movement date that cannot be read.
    BadDate(String),
    TransactionNotFound,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for FinanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FinanceError::EmptyName => write!(f, "the name is empty"),
            FinanceError::InvalidAmount => write!(f, "the amount is not valid"),
            FinanceError::NoMainCurrency => write!(f, "no main currency is set"),
            FinanceError::UnknownCurrency(c) => write!(f, "unknown currency: {c}"),
            FinanceError::BadKind(k) => write!(f, "invalid stored account kind: {k}"),
            FinanceError::AccountNotFound => write!(f, "account not found"),
            FinanceError::BadTxKind(k) => write!(f, "invalid stored movement kind: {k}"),
            FinanceError::BadDate(d) => write!(f, "invalid stored movement date: {d}"),
            FinanceError::TransactionNotFound => write!(f, "movement not found"),
            FinanceError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for FinanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FinanceError::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for FinanceError {
    fn from(e: rusqlite::Error) -> Self {
        FinanceError::Sqlite(e)
    }
}
