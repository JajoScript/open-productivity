// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Scheduled tab draws besides its lists: the calendar marks and the biggest payments.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::FinanceError;
use super::bills::bills_due_in_month;
use super::rates::load_rates;
use super::scheduled::due_in_month;
use super::transactions::TxKind;
use crate::calendar::month_start;

/// What a day of the Scheduled calendar has on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayMark {
    /// A payment is due.
    Due,
    /// A payment that is already late is due.
    Overdue,
}

/// One of the month's biggest pending payments, in the main currency.
#[derive(Debug, Clone, PartialEq)]
pub struct BigDue {
    /// The scheduled payment's description, or the card's name for a bill.
    pub description: String,
    pub bill: bool,
    pub amount: i64,
    /// `amount` relative to the biggest of the list, 0..=1.
    pub share: f32,
}

/// The days of the month holding `month` with a payment or card bill due, by date. Overdue
/// payments of earlier months have no day in this month and are left out.
pub fn due_days(
    conn: &Connection,
    month: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<(NaiveDate, DayMark)>, FinanceError> {
    let first = month_start(month);
    let dues = due_in_month(conn, month, today)?
        .into_iter()
        .map(|r| (r.date, r.overdue));
    let bills = bills_due_in_month(conn, month, today)?
        .into_iter()
        .map(|b| (b.due, b.overdue));
    let mut days: Vec<(NaiveDate, DayMark)> = Vec::new();
    for (date, overdue) in dues.chain(bills).filter(|(d, _)| month_start(*d) == first) {
        let mark = if overdue {
            DayMark::Overdue
        } else {
            DayMark::Due
        };
        match days.iter_mut().find(|(d, _)| *d == date) {
            Some(day) if mark == DayMark::Overdue => day.1 = mark,
            Some(_) => {}
            None => days.push((date, mark)),
        }
    }
    days.sort_by_key(|(d, _)| *d);
    Ok(days)
}

/// The `limit` biggest pending expenses (scheduled occurrences and card bills) of the month, in
/// the main currency, biggest first.
pub fn biggest_due(
    conn: &Connection,
    month: NaiveDate,
    today: NaiveDate,
    limit: usize,
) -> Result<Vec<BigDue>, FinanceError> {
    let rates = load_rates(conn)?;
    let mut items = Vec::new();
    for r in due_in_month(conn, month, today)?
        .into_iter()
        .filter(|r| r.scheduled.kind == TxKind::Expense)
    {
        let s = r.scheduled;
        items.push((s.description, false, rates.to_main(s.amount, s.currency)?));
    }
    for b in bills_due_in_month(conn, month, today)? {
        items.push((b.card_name, true, rates.to_main(b.pending, b.currency)?));
    }
    items.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    items.truncate(limit);
    let max = items.first().map_or(0, |i| i.2);
    Ok(items
        .into_iter()
        .map(|(description, bill, amount)| BigDue {
            description,
            bill,
            amount,
            share: if max > 0 {
                (amount.max(0) as f64 / max as f64) as f32
            } else {
                0.0
            },
        })
        .collect())
}
