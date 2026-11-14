// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Scheduled payments: one-time, recurring or in monthly installments. Confirming one records
//! a movement and moves the schedule on; skipping only moves it on.

use chrono::{Datelike, Months, NaiveDate};
use rusqlite::Connection;

use super::FinanceError;
use super::accounts::{AccountId, check_account, get_account};
use super::categories::CategoryId;
use super::money::{Currency, currency, parse_amount};
use super::transactions::{DATE_FORMAT, TxKind, check_category, insert};
use crate::calendar::month_start;
use crate::recurrence::{Frequency, next_date};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledId(pub i64);

/// How a scheduled payment repeats. `Installments(n)` are `n` (>= 2) monthly payments;
/// recurring payments are weekly, monthly or yearly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScheduleForm {
    Once,
    Recurring(Frequency),
    Installments(i64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheduled {
    pub id: ScheduledId,
    pub kind: TxKind,
    pub description: String,
    pub account_id: AccountId,
    pub account_name: String,
    pub currency: &'static Currency,
    pub category: Option<CategoryId>,
    pub category_name: Option<String>,
    /// Always > 0, in the account's currency.
    pub amount: i64,
    /// The day of the month that monthly and yearly dates keep.
    pub start_date: NaiveDate,
    /// The occurrence that confirm and skip act on.
    pub next_date: NaiveDate,
    pub form: ScheduleForm,
    pub installments_paid: i64,
    pub done: bool,
}

/// What an occurrence of the "to pay" list is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DueKind {
    Once,
    Recurring(Frequency),
    Installment { number: i64, total: i64 },
}

/// One occurrence of a payment due in a month.
#[derive(Clone, Debug, PartialEq)]
pub struct DueRow {
    pub scheduled: Scheduled,
    pub date: NaiveDate,
    /// The occurrence that confirm and skip act on.
    pub is_next: bool,
    pub kind: DueKind,
    /// Its date is before today.
    pub overdue: bool,
}

const SELECT: &str = "SELECT s.id, s.kind, s.description, s.account_id, a.name, a.currency,
    s.category_id, c.name, s.amount, s.start_date, s.next_date, s.form, s.frequency,
    s.installments_total, s.installments_paid, s.done
    FROM finance_scheduled s
    JOIN finance_account a ON a.id = s.account_id
    LEFT JOIN finance_category c ON c.id = s.category_id";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Scheduled, FinanceError>> {
    let kind: String = r.get(1)?;
    let code: String = r.get(5)?;
    let (start, next, form): (String, String, String) = (r.get(9)?, r.get(10)?, r.get(11)?);
    let (freq, total): (Option<String>, Option<i64>) = (r.get(12)?, r.get(13)?);
    let (id, description, account_id, account_name) = (r.get(0)?, r.get(2)?, r.get(3)?, r.get(4)?);
    let (category, category_name): (Option<i64>, Option<String>) = (r.get(6)?, r.get(7)?);
    let (amount, installments_paid, done) = (r.get(8)?, r.get(14)?, r.get(15)?);
    Ok((|| {
        let date = |s: String| {
            NaiveDate::parse_from_str(&s, DATE_FORMAT).map_err(|_| FinanceError::BadDate(s))
        };
        let form = match (form.as_str(), freq, total) {
            ("once", _, _) => ScheduleForm::Once,
            ("recurring", Some(f), _) => {
                ScheduleForm::Recurring(Frequency::parse(&f).ok_or(FinanceError::InvalidSchedule)?)
            }
            ("installments", _, Some(n)) => ScheduleForm::Installments(n),
            _ => return Err(FinanceError::InvalidSchedule),
        };
        Ok(Scheduled {
            id: ScheduledId(id),
            kind: TxKind::parse(&kind)?,
            description,
            account_id: AccountId(account_id),
            account_name,
            currency: currency(&code).ok_or(FinanceError::UnknownCurrency(code))?,
            category: category.map(CategoryId),
            category_name,
            amount,
            start_date: date(start)?,
            next_date: date(next)?,
            form,
            installments_paid,
            done,
        })
    })())
}

pub fn get_scheduled(conn: &Connection, id: ScheduledId) -> Result<Scheduled, FinanceError> {
    match conn.query_row(&format!("{SELECT} WHERE s.id = ?1"), [id.0], row) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::ScheduledNotFound),
        Err(e) => Err(e.into()),
    }
}

/// Every scheduled payment: the pending ones first, by next date.
pub fn list_scheduled(conn: &Connection) -> Result<Vec<Scheduled>, FinanceError> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY s.done, s.next_date, s.description, s.id"
    ))?;
    let rows = stmt.query_map([], row)?;
    rows.map(|r| r?).collect()
}

/// Installments are at least 2; recurring is weekly, monthly or yearly.
fn check_form(form: ScheduleForm) -> Result<(), FinanceError> {
    match form {
        ScheduleForm::Installments(n) if n < 2 => Err(FinanceError::InvalidSchedule),
        ScheduleForm::Recurring(Frequency::Daily) => Err(FinanceError::InvalidSchedule),
        _ => Ok(()),
    }
}

/// The amount (> 0) read in the account's currency.
fn read_amount(conn: &Connection, account: AccountId, amount: &str) -> Result<i64, FinanceError> {
    let c = get_account(conn, account)?.currency;
    parse_amount(amount, c)
        .filter(|a| *a > 0)
        .ok_or(FinanceError::InvalidAmount)
}

/// Schedules a payment whose first date is `first_date`. The account must be active.
#[allow(clippy::too_many_arguments)]
pub fn create_scheduled(
    conn: &Connection,
    kind: TxKind,
    description: &str,
    account: AccountId,
    amount: &str,
    category: Option<CategoryId>,
    first_date: NaiveDate,
    form: ScheduleForm,
) -> Result<ScheduledId, FinanceError> {
    let amount = read_amount(conn, account, amount)?;
    check_account(conn, account, None)?;
    check_category(conn, category, kind, None)?;
    check_form(form)?;
    let (form, freq, total) = match form {
        ScheduleForm::Once => ("once", None, None),
        ScheduleForm::Recurring(f) => ("recurring", Some(f.as_str()), None),
        ScheduleForm::Installments(n) => ("installments", None, Some(n)),
    };
    let date = first_date.format(DATE_FORMAT).to_string();
    conn.execute(
        "INSERT INTO finance_scheduled
             (kind, description, account_id, category_id, amount, start_date, next_date, form,
              frequency, installments_total)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9)",
        (
            kind.as_str(),
            description.trim(),
            account.0,
            category.map(|c| c.0),
            amount,
            date,
            form,
            freq,
            total,
        ),
    )?;
    Ok(ScheduledId(conn.last_insert_rowid()))
}

fn pending(conn: &Connection, id: ScheduledId) -> Result<Scheduled, FinanceError> {
    let s = get_scheduled(conn, id)?;
    if s.done {
        return Err(FinanceError::ScheduledDone);
    }
    Ok(s)
}

/// Edits a pending payment; its form and kind do not change. `frequency` is required for a
/// recurring payment and must be `None` for the others. A new `next` date also moves the day
/// that monthly and yearly dates keep.
#[allow(clippy::too_many_arguments)]
pub fn update_scheduled(
    conn: &Connection,
    id: ScheduledId,
    description: &str,
    amount: &str,
    account: AccountId,
    category: Option<CategoryId>,
    next: NaiveDate,
    frequency: Option<Frequency>,
) -> Result<(), FinanceError> {
    let s = pending(conn, id)?;
    let amount = read_amount(conn, account, amount)?;
    check_account(conn, account, Some(s.account_id))?;
    check_category(conn, category, s.kind, s.category)?;
    let frequency = match (s.form, frequency) {
        (ScheduleForm::Recurring(_), Some(f)) => {
            check_form(ScheduleForm::Recurring(f))?;
            Some(f.as_str())
        }
        (ScheduleForm::Recurring(_), None) | (_, Some(_)) => {
            return Err(FinanceError::InvalidSchedule);
        }
        (_, None) => None,
    };
    conn.execute(
        "UPDATE finance_scheduled
         SET description = ?2, amount = ?3, account_id = ?4, category_id = ?5, next_date = ?6,
             frequency = ?7, start_date = CASE WHEN ?8 THEN ?6 ELSE start_date END
         WHERE id = ?1",
        (
            id.0,
            description.trim(),
            amount,
            account.0,
            category.map(|c| c.0),
            next.format(DATE_FORMAT).to_string(),
            frequency,
            next != s.next_date,
        ),
    )?;
    Ok(())
}

/// Deletes a payment; the movements it created stay, unlinked.
pub fn delete_scheduled(conn: &Connection, id: ScheduledId) -> Result<(), FinanceError> {
    match conn.execute("DELETE FROM finance_scheduled WHERE id = ?1", [id.0])? {
        0 => Err(FinanceError::ScheduledNotFound),
        _ => Ok(()),
    }
}

/// The occurrence after `date`: weekly adds 7 days; monthly, yearly and installments keep the
/// day of `start_date` (31 -> 30 Apr -> 31 May). `None` for a one-time payment.
fn next_after(s: &Scheduled, date: NaiveDate) -> Option<NaiveDate> {
    let freq = match s.form {
        ScheduleForm::Once => return None,
        ScheduleForm::Recurring(f) => f,
        ScheduleForm::Installments(_) => Frequency::Monthly,
    };
    next_date(date, freq, 1, s.start_date.day())
}

/// (position from `next_date`, date) of the occurrences in `[from, to]`.
fn walk(s: &Scheduled, from: NaiveDate, to: NaiveDate) -> Vec<(i64, NaiveDate)> {
    if s.done {
        return Vec::new();
    }
    let limit = match s.form {
        ScheduleForm::Once => 1,
        ScheduleForm::Installments(total) => (total - s.installments_paid).max(0),
        ScheduleForm::Recurring(_) => i64::MAX,
    };
    let (mut out, mut date, mut i) = (Vec::new(), s.next_date, 0);
    while i < limit && date <= to {
        if date >= from {
            out.push((i, date));
        }
        let Some(next) = next_after(s, date) else {
            break;
        };
        date = next;
        i += 1;
    }
    out
}

/// Moves a payment on after its occurrence was paid or skipped. Only a paid installment counts.
fn advance(conn: &Connection, s: &Scheduled, paid: bool) -> Result<(), FinanceError> {
    let next = next_after(s, s.next_date).unwrap_or(s.next_date);
    let (done, count) = match s.form {
        ScheduleForm::Once => (true, s.installments_paid),
        ScheduleForm::Recurring(_) => (false, s.installments_paid),
        ScheduleForm::Installments(total) => {
            let n = s.installments_paid + i64::from(paid);
            (n >= total, n)
        }
    };
    conn.execute(
        "UPDATE finance_scheduled SET next_date = ?2, installments_paid = ?3, done = ?4
         WHERE id = ?1",
        (s.id.0, next.format(DATE_FORMAT).to_string(), count, done),
    )?;
    Ok(())
}

/// Confirms the next occurrence: records a movement on `date` for `amount` (typed, in the
/// account's currency; the UI pre-fills the scheduled one) and moves the payment on, in one
/// transaction. The movement keeps the link and, for an installment, its number.
pub fn confirm_scheduled(
    conn: &Connection,
    id: ScheduledId,
    date: NaiveDate,
    amount: &str,
) -> Result<(), FinanceError> {
    let s = pending(conn, id)?;
    let amount = read_amount(conn, s.account_id, amount)?;
    check_category(conn, s.category, s.kind, s.category)?;
    let number = match s.form {
        ScheduleForm::Installments(_) => Some(s.installments_paid + 1),
        _ => None,
    };
    let tx = conn.unchecked_transaction()?;
    insert(
        &tx,
        s.kind,
        s.account_id,
        amount,
        date,
        &s.description,
        s.category,
        Some((s.id, number)),
    )?;
    advance(&tx, &s, true)?;
    tx.commit()?;
    Ok(())
}

/// Skips the next occurrence without a movement. A skipped installment is not paid: it goes to
/// the end.
pub fn skip_scheduled(conn: &Connection, id: ScheduledId) -> Result<(), FinanceError> {
    let s = pending(conn, id)?;
    advance(conn, &s, false)
}

/// The "to pay" list of the month holding `month` (any day of it works): one row per
/// occurrence in it, by date, description and id. In the month of `today` the overdue payments
/// (`next_date` before the month) also show, with their date.
pub fn due_in_month(
    conn: &Connection,
    month: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<DueRow>, FinanceError> {
    let first = month_start(month);
    let Some(last) = first
        .checked_add_months(Months::new(1))
        .and_then(|d| d.pred_opt())
    else {
        return Ok(Vec::new());
    };
    let current = month_start(today) == first;
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE s.done = 0"))?;
    let all = stmt
        .query_map([], row)?
        .map(|r| r?)
        .collect::<Result<Vec<_>, _>>()?;
    let mut rows = Vec::new();
    for s in all {
        let mut hits = walk(&s, first, last);
        if current && s.next_date < first {
            hits.insert(0, (0, s.next_date));
        }
        for (i, date) in hits {
            let kind = match s.form {
                ScheduleForm::Once => DueKind::Once,
                ScheduleForm::Recurring(f) => DueKind::Recurring(f),
                ScheduleForm::Installments(total) => DueKind::Installment {
                    number: s.installments_paid + 1 + i,
                    total,
                },
            };
            rows.push(DueRow {
                scheduled: s.clone(),
                date,
                is_next: i == 0,
                kind,
                overdue: date < today,
            });
        }
    }
    rows.sort_by(|a, b| {
        (a.date, &a.scheduled.description, a.scheduled.id.0).cmp(&(
            b.date,
            &b.scheduled.description,
            b.scheduled.id.0,
        ))
    });
    Ok(rows)
}
