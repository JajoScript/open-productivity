// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Finances: accounts and their balance, in the instance's main currency.

use std::fmt;

mod accounts;
mod categories;
mod money;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_categories;
#[cfg(test)]
mod tests_transactions;
#[cfg(test)]
mod tests_transfers;
mod transactions;
mod transfers;

pub use accounts::*;
pub use categories::*;
pub use money::*;
pub use transactions::*;
pub use transfers::*;

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
    CategoryNotFound,
    /// Another active category of the same kind has that name.
    DuplicateName,
    /// Some movement uses the category.
    InUse,
    /// The suggested categories can only be created in an instance without categories.
    CategoriesExist,
    /// The category's kind differs from the movement's.
    CategoryKindMismatch,
    /// An archived category cannot be newly assigned to a movement.
    CategoryArchived,
    /// A transfer needs two different accounts.
    SameAccount,
    /// Some movement uses the account.
    AccountInUse,
    /// An archived account cannot be newly used by a movement.
    AccountArchived,
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
            FinanceError::CategoryNotFound => write!(f, "category not found"),
            FinanceError::DuplicateName => write!(f, "a category with that name exists"),
            FinanceError::InUse => write!(f, "the category is in use"),
            FinanceError::CategoriesExist => write!(f, "categories already exist"),
            FinanceError::CategoryKindMismatch => write!(f, "the category kind differs"),
            FinanceError::CategoryArchived => write!(f, "the category is archived"),
            FinanceError::SameAccount => write!(f, "the accounts are the same"),
            FinanceError::AccountInUse => write!(f, "the account is in use"),
            FinanceError::AccountArchived => write!(f, "the account is archived"),
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
