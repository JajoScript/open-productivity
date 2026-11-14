// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The bill of a credit card: what the last closing left to pay and when it is due.
//!
//! Only the current bill exists. Past cycles are not reconstructed: once a newer closing
//! passes, its debt (which includes the older unpaid one) replaces the previous bill.

use chrono::{Datelike, Months, NaiveDate};
use rusqlite::Connection;

use super::FinanceError;
use super::accounts::{Account, AccountId, get_account, list_accounts};
use super::money::Currency;
use super::transactions::DATE_FORMAT;
use crate::calendar::month_start;

#[derive(Clone, Debug, PartialEq)]
pub struct Bill {
    pub card: AccountId,
    pub card_name: String,
    pub currency: &'static Currency,
    pub closing: NaiveDate,
    pub due: NaiveDate,
    /// What is left to pay, in the card's currency; always > 0.
    pub pending: i64,
    /// The due date is before today.
    pub overdue: bool,
}

/// `day` of `first`'s month, capped at the month's last day (31 -> 30 Apr, 28/29 Feb).
fn capped(first: NaiveDate, day: u32) -> Option<NaiveDate> {
    let next = first.checked_add_months(Months::new(1))?;
    let len = next.signed_duration_since(first).num_days() as u32;
    first.with_day(day.clamp(1, len))
}

/// The latest closing not after `today` and the first due date strictly after it.
fn cycle(closing_day: u32, due_day: u32, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let this = month_start(today);
    let mut closing = capped(this, closing_day)?;
    if closing > today {
        closing = capped(this.checked_sub_months(Months::new(1))?, closing_day)?;
    }
    let first = month_start(closing);
    let due = match capped(first, due_day) {
        Some(d) if d > closing => d,
        _ => capped(first.checked_add_months(Months::new(1))?, due_day)?,
    };
    Some((closing, due))
}

fn bill_of(
    conn: &Connection,
    card: &Account,
    today: NaiveDate,
) -> Result<Option<Bill>, FinanceError> {
    let Some(terms) = card.card else {
        return Ok(None);
    };
    let Some((closing, due)) = cycle(terms.closing_day, terms.due_day, today) else {
        return Ok(None);
    };
    let at = closing.format(DATE_FORMAT).to_string();
    // The balance formula of `list_accounts`, only up to the closing date.
    let at_closing: i64 = conn.query_row(
        "SELECT a.initial_balance + coalesce((SELECT sum(CASE WHEN t.account_id = a.id
                THEN CASE t.kind WHEN 'income' THEN t.amount ELSE -t.amount END
                ELSE coalesce(t.to_amount, t.amount) END)
            FROM finance_transaction t
            WHERE (t.account_id = a.id OR t.to_account_id = a.id) AND t.date <= ?2), 0)
         FROM finance_account a WHERE a.id = ?1",
        (card.id.0, &at),
        |r| r.get(0),
    )?;
    let paid_after: i64 = conn.query_row(
        "SELECT coalesce(sum(coalesce(to_amount, amount)), 0) FROM finance_transaction
         WHERE kind = 'transfer' AND to_account_id = ?1 AND date > ?2",
        (card.id.0, &at),
        |r| r.get(0),
    )?;
    let pending = at_closing.saturating_neg().saturating_sub(paid_after);
    Ok((pending > 0).then(|| Bill {
        card: card.id,
        card_name: card.name.clone(),
        currency: card.currency,
        closing,
        due,
        pending,
        overdue: due < today,
    }))
}

/// The bill of `card` as of `today`: the debt at the last closing minus the transfers into the
/// card after it. `None` for a plain account or when nothing is left to pay.
pub fn card_bill(
    conn: &Connection,
    card: AccountId,
    today: NaiveDate,
) -> Result<Option<Bill>, FinanceError> {
    bill_of(conn, &get_account(conn, card)?, today)
}

/// The bills of the active cards, by due date, then card name.
pub fn list_bills(conn: &Connection, today: NaiveDate) -> Result<Vec<Bill>, FinanceError> {
    let mut bills = Vec::new();
    for card in list_accounts(conn)?.iter().filter(|a| !a.archived) {
        bills.extend(bill_of(conn, card, today)?);
    }
    bills.sort_by(|a, b| (a.due, &a.card_name, a.card.0).cmp(&(b.due, &b.card_name, b.card.0)));
    Ok(bills)
}

/// The bills due in the month holding `month` (any day of it works). In the month of `today`
/// the overdue bills from earlier months also show.
pub fn bills_due_in_month(
    conn: &Connection,
    month: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<Bill>, FinanceError> {
    let first = month_start(month);
    let current = month_start(today) == first;
    Ok(list_bills(conn, today)?
        .into_iter()
        .filter(|b| month_start(b.due) == first || (current && b.due < first))
        .collect())
}

/// The account a bill is suggested to be paid from: the first active account that is not a
/// card, else the first active account other than the card.
pub fn bill_payment_source(
    conn: &Connection,
    card: AccountId,
) -> Result<Option<AccountId>, FinanceError> {
    let active: Vec<_> = list_accounts(conn)?
        .into_iter()
        .filter(|a| !a.archived && a.id != card)
        .collect();
    Ok(active
        .iter()
        .find(|a| a.card.is_none())
        .or(active.first())
        .map(|a| a.id))
}
