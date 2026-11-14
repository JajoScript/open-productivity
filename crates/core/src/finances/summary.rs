// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The month summary: income, expenses, balance, accounts total, expense by category and the
//! latest movements.

use chrono::{Months, NaiveDate};
use rusqlite::{Connection, ToSql};

use super::FinanceError;
use super::categories::{CategoryId, category_totals};
use super::transactions::{DATE_FORMAT, Movement, SELECT, TxKind, row};
use crate::calendar::month_start;

/// How many movements the summary lists.
const RECENT: usize = 5;

/// A figure of the month and the same figure a month earlier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Figure {
    pub value: i64,
    pub previous: i64,
}

impl Figure {
    /// `value` minus `previous`; saturates.
    pub fn change(&self) -> i64 {
        self.value.saturating_sub(self.previous)
    }
}

#[derive(Debug, PartialEq)]
pub struct CategoryBar {
    /// None is the expenses without category.
    pub category: Option<CategoryId>,
    pub name: Option<String>,
    pub total: i64,
    /// Total relative to the first bar, in (0, 1].
    pub width: f32,
}

#[derive(Debug, PartialEq)]
pub struct MonthSummary {
    pub income: Figure,
    pub expense: Figure,
    /// Income minus expenses.
    pub balance: Figure,
    /// Non-archived accounts at the end of the month.
    pub accounts_total: Figure,
    /// Expenses by category, biggest first.
    pub by_category: Vec<CategoryBar>,
    /// The newest movements of the month.
    pub recent: Vec<Movement>,
}

fn text(d: NaiveDate) -> String {
    d.format(DATE_FORMAT).to_string()
}

/// Adds the first column of every row, saturating (SQL's own SUM would fail on overflow).
fn sum(conn: &Connection, sql: &str, params: &[&dyn ToSql]) -> Result<i64, FinanceError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| r.get::<_, i64>(0))?;
    let mut total = 0i64;
    for amount in rows {
        total = total.saturating_add(amount?);
    }
    Ok(total)
}

fn kind_sum(
    conn: &Connection,
    kind: TxKind,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<i64, FinanceError> {
    sum(
        conn,
        "SELECT amount FROM finance_transaction WHERE kind = ?1 AND date >= ?2 AND date < ?3",
        &[&kind.as_str(), &text(from), &text(to)],
    )
}

/// Open balances plus every movement dated before `end`; a transfer between two active
/// accounts nets zero.
fn accounts_total_before(conn: &Connection, end: NaiveDate) -> Result<i64, FinanceError> {
    let opening = sum(
        conn,
        "SELECT initial_balance FROM finance_account WHERE archived = 0",
        &[],
    )?;
    let moved = sum(
        conn,
        "SELECT CASE WHEN t.account_id = a.id
                THEN CASE t.kind WHEN 'income' THEN t.amount ELSE -t.amount END
                ELSE t.amount END
         FROM finance_transaction t
         JOIN finance_account a ON a.id = t.account_id OR a.id = t.to_account_id
         WHERE a.archived = 0 AND t.date < ?1",
        &[&text(end)],
    )?;
    Ok(opening.saturating_add(moved))
}

/// The summary of the month holding `month` (any day of it works) against the month before.
// shortcut: sums amounts as they are, as every account is in the main currency today; convert
// with the exchange rates when other currencies arrive (slice 6).
pub fn month_summary(conn: &Connection, month: NaiveDate) -> Result<MonthSummary, FinanceError> {
    let from = month_start(month);
    let zero = Figure {
        value: 0,
        previous: 0,
    };
    let (Some(to), Some(before)) = (
        from.checked_add_months(Months::new(1)),
        from.checked_sub_months(Months::new(1)),
    ) else {
        return Ok(MonthSummary {
            income: zero,
            expense: zero,
            balance: zero,
            accounts_total: zero,
            by_category: Vec::new(),
            recent: Vec::new(),
        });
    };
    let figure = |kind| -> Result<Figure, FinanceError> {
        Ok(Figure {
            value: kind_sum(conn, kind, from, to)?,
            previous: kind_sum(conn, kind, before, from)?,
        })
    };
    let (income, expense) = (figure(TxKind::Income)?, figure(TxKind::Expense)?);
    let balance = Figure {
        value: income.value.saturating_sub(expense.value),
        previous: income.previous.saturating_sub(expense.previous),
    };
    let accounts_total = Figure {
        value: accounts_total_before(conn, to)?,
        previous: accounts_total_before(conn, from)?,
    };

    let totals = category_totals(conn, from, TxKind::Expense)?;
    let max = totals.first().map_or(0, |t| t.total);
    let by_category = totals
        .into_iter()
        .map(|t| CategoryBar {
            category: t.category,
            name: t.name,
            total: t.total,
            width: t.total as f32 / max as f32,
        })
        .collect();

    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE t.date >= ?1 AND t.date < ?2 ORDER BY t.date DESC, t.id DESC LIMIT {RECENT}"
    ))?;
    let recent = stmt
        .query_map([text(from), text(to)], row)?
        .map(|r| r?)
        .collect::<Result<_, _>>()?;

    Ok(MonthSummary {
        income,
        expense,
        balance,
        accounts_total,
        by_category,
        recent,
    })
}
