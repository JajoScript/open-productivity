// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Finances: accounts, movements, categories, exchange rates and the month summary.

use std::fmt;

mod accounts;
mod bills;
mod budget;
mod categories;
mod charts;
mod due_view;
mod money;
mod overview;
mod rates;
mod reserves;
mod scheduled;
mod summary;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bills;
#[cfg(test)]
mod tests_budget;
#[cfg(test)]
mod tests_cards;
#[cfg(test)]
mod tests_categories;
#[cfg(test)]
mod tests_charts;
#[cfg(test)]
mod tests_credit_lines;
#[cfg(test)]
mod tests_currencies;
#[cfg(test)]
mod tests_due_view;
#[cfg(test)]
mod tests_main_currency;
#[cfg(test)]
mod tests_overview;
#[cfg(test)]
mod tests_reserves;
#[cfg(test)]
mod tests_scheduled;
#[cfg(test)]
mod tests_summary;
#[cfg(test)]
mod tests_transactions;
#[cfg(test)]
mod tests_transfers;
#[cfg(test)]
mod tests_worth;
mod transactions;
mod transfers;
mod worth;

pub use accounts::*;
pub use bills::*;
pub use budget::*;
pub use categories::*;
pub use charts::*;
pub use due_view::*;
pub use money::*;
pub use overview::*;
pub use rates::*;
pub use reserves::*;
pub use scheduled::*;
pub use summary::*;
pub use transactions::*;
pub use transfers::*;
pub use worth::*;

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
    BucketNotFound,
    /// Some category belongs to the bucket.
    BucketInUse,
    /// The suggested budget can only be created in an instance without buckets.
    BucketsExist,
    /// A percentage is outside 0..=100 or the active buckets would add up to more than 100.
    InvalidShares,
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
    /// A non-main currency needs its exchange rate to the main one.
    MissingRate(String),
    /// The rate is malformed, not positive, too large or for the main currency.
    InvalidRate,
    /// The account has movements, so its currency cannot change.
    CurrencyLocked,
    /// A card's limit is empty, malformed or not positive.
    InvalidLimit,
    /// A closing or due day is outside 1..=31.
    InvalidDay,
    /// A card cannot become a plain account, nor the reverse.
    KindLocked,
    /// A scheduled payment's form, frequency or installments are not valid.
    InvalidSchedule,
    ScheduledNotFound,
    /// A finished scheduled payment cannot be edited, confirmed or skipped.
    ScheduledDone,
    /// The account has this many pending scheduled payments.
    AccountHasSchedules(i64),
    ReserveNotFound,
    /// The reserve (or the account's reserves) still holds money, so it cannot be archived.
    ReserveNotEmpty,
    /// The reserve has moves, so it cannot be deleted.
    ReserveInUse,
    /// More than the account's available (reserve) or than the reserve's balance (withdraw).
    Insufficient,
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
            FinanceError::BucketNotFound => write!(f, "bucket not found"),
            FinanceError::BucketInUse => write!(f, "the bucket is in use"),
            FinanceError::BucketsExist => write!(f, "buckets already exist"),
            FinanceError::InvalidShares => write!(f, "the percentages are not valid"),
            FinanceError::CategoriesExist => write!(f, "categories already exist"),
            FinanceError::CategoryKindMismatch => write!(f, "the category kind differs"),
            FinanceError::CategoryArchived => write!(f, "the category is archived"),
            FinanceError::SameAccount => write!(f, "the accounts are the same"),
            FinanceError::AccountInUse => write!(f, "the account is in use"),
            FinanceError::AccountArchived => write!(f, "the account is archived"),
            FinanceError::MissingRate(c) => write!(f, "missing exchange rate for {c}"),
            FinanceError::InvalidRate => write!(f, "the exchange rate is not valid"),
            FinanceError::CurrencyLocked => write!(f, "the account's currency is locked"),
            FinanceError::InvalidLimit => write!(f, "the credit limit is not valid"),
            FinanceError::InvalidDay => write!(f, "the day is not valid"),
            FinanceError::KindLocked => write!(f, "the account kind cannot change"),
            FinanceError::InvalidSchedule => write!(f, "the scheduled payment is not valid"),
            FinanceError::ScheduledNotFound => write!(f, "scheduled payment not found"),
            FinanceError::ScheduledDone => write!(f, "the scheduled payment is finished"),
            FinanceError::AccountHasSchedules(n) => {
                write!(f, "the account has {n} scheduled payments")
            }
            FinanceError::ReserveNotFound => write!(f, "reserve not found"),
            FinanceError::ReserveNotEmpty => write!(f, "the reserve holds money"),
            FinanceError::ReserveInUse => write!(f, "the reserve has moves"),
            FinanceError::Insufficient => write!(f, "not enough money"),
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
