// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The pending payments the Calendar and the Today view show, read only.

use std::collections::HashSet;

use chrono::{Months, NaiveDate};
use rusqlite::Connection;

use super::FinanceError;
use super::bills::bills_due_in_month;
use super::money::Currency;
use super::scheduled::due_in_month;
use super::transactions::TxKind;
use crate::calendar::month_start;

/// A pending payment: an occurrence of a scheduled payment or a card's bill.
#[derive(Debug, Clone, PartialEq)]
pub struct Payment {
    pub date: NaiveDate,
    /// The scheduled payment's description, or the card's name for a bill.
    pub description: String,
    pub bill: bool,
    /// A bill is always an expense.
    pub kind: TxKind,
    /// Always > 0, in `currency`, the account's.
    pub amount: i64,
    pub currency: &'static Currency,
    /// The date is before today.
    pub overdue: bool,
}

/// Every pending payment of the months from `first` to `last`, by date then description.
/// The month of `today` repeats the overdue ones of earlier months with their own date, so an
/// occurrence (same payment, same date) is kept once.
fn gather(
    conn: &Connection,
    first: NaiveDate,
    last: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<Payment>, FinanceError> {
    let (mut seen, mut out) = (HashSet::new(), Vec::new());
    let mut month = month_start(first);
    while month <= last {
        for r in due_in_month(conn, month, today)? {
            if seen.insert((false, r.scheduled.id.0, r.date)) {
                let s = r.scheduled;
                out.push(Payment {
                    date: r.date,
                    description: s.description,
                    bill: false,
                    kind: s.kind,
                    amount: s.amount,
                    currency: s.currency,
                    overdue: r.overdue,
                });
            }
        }
        for b in bills_due_in_month(conn, month, today)? {
            if seen.insert((true, b.card.0, b.due)) {
                out.push(Payment {
                    date: b.due,
                    description: b.card_name,
                    bill: true,
                    kind: TxKind::Expense,
                    amount: b.pending,
                    currency: b.currency,
                    overdue: b.overdue,
                });
            }
        }
        month = match month.checked_add_months(Months::new(1)) {
            Some(m) => m,
            None => break,
        };
    }
    out.sort_by(|a, b| (a.date, &a.description).cmp(&(b.date, &b.description)));
    Ok(out)
}

/// The pending payments dated from `start` to `end`, both included.
pub fn payments_between(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<Payment>, FinanceError> {
    let mut all = gather(conn, start, end, today)?;
    all.retain(|p| p.date >= start && p.date <= end);
    Ok(all)
}

/// The pending payments of today and the overdue ones, by date.
pub fn payments_due(conn: &Connection, today: NaiveDate) -> Result<Vec<Payment>, FinanceError> {
    let mut all = gather(conn, today, today, today)?;
    all.retain(|p| p.date <= today);
    Ok(all)
}
