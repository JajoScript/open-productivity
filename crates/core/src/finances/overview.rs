// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Summary tab draws: the four cards with their daily series, the category donut and
//! the upcoming payments. Everything is in the main currency unless a field says otherwise.

use chrono::{Datelike, Months, NaiveDate};
use rusqlite::{Connection, ToSql};

use super::FinanceError;
use super::bills::bills_due_in_month;
use super::categories::category_totals;
use super::charts::BalanceChart;
use super::money::{Currency, Rates, currency};
use super::rates::load_rates;
use super::scheduled::{ScheduleForm, due_in_month};
use super::summary::accounts_total_before;
use super::transactions::{DATE_FORMAT, Movement, MovementFilter, TxKind, list_month};
use crate::calendar::month_start;

/// One card: the month's value, its change against the month before and the running value per
/// day (the sparkline).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SummaryCard {
    pub value: i64,
    /// Whole percent against the month before; `None` when that month is 0 (or for the total).
    pub percent: Option<i64>,
    /// Value at the end of each day, from day 1 up to today in the current month.
    pub series: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MonthCards {
    /// The balance of the active accounts at the end of the month.
    pub total: SummaryCard,
    /// Income minus expenses.
    pub balance: SummaryCard,
    pub income: SummaryCard,
    pub expense: SummaryCard,
    /// Income minus expenses of the month before; its series covers all its days.
    pub previous: SummaryCard,
    /// The last day drawn: today in the current month, else the month's length.
    pub shown: usize,
    /// Days the balance chart spans: the longer of the two months.
    pub span: usize,
}

impl MonthCards {
    /// The Balance of the period chart: this month so far against the month before.
    pub fn balance_chart(&self) -> BalanceChart {
        BalanceChart {
            current: self.balance.series.clone(),
            previous: self.previous.series.clone(),
            span: self.span,
        }
    }
}

/// Change from `previous` to `value` in whole percent, rounded half away from zero. The base is
/// `|previous|`, so a balance that goes from -100 to 50 reads +150% and from 100 to -50 reads
/// -150%. `None` when `previous` is 0.
pub fn percent_change(value: i64, previous: i64) -> Option<i64> {
    if previous == 0 {
        return None;
    }
    let num = (value as i128 - previous as i128) * 100;
    let den = (previous as i128).abs();
    let q = (num.abs() * 2 + den) / (den * 2);
    Some((if num < 0 { -q } else { q }).clamp(i64::MIN as i128, i64::MAX as i128) as i64)
}

/// Converts and adds, per day of the month, the rows (date, amount, currency code) of `sql`.
fn daily(
    conn: &Connection,
    rates: &Rates,
    sql: &str,
    params: &[&dyn ToSql],
    days: usize,
) -> Result<Vec<i64>, FinanceError> {
    let mut out = vec![0i64; days];
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (date, amount, code) = row?;
        let Some(i) = date
            .get(8..10)
            .and_then(|d| d.parse::<usize>().ok())
            .and_then(|d| d.checked_sub(1))
            .filter(|i| *i < days)
        else {
            continue;
        };
        let c = currency(&code).ok_or(FinanceError::UnknownCurrency(code))?;
        out[i] = out[i].saturating_add(rates.to_main(amount, c)?);
    }
    Ok(out)
}

/// The daily sums of the movements of `kind` dated from `from` (included) to `to` (excluded).
fn kind_daily(
    conn: &Connection,
    rates: &Rates,
    kind: TxKind,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<i64>, FinanceError> {
    daily(
        conn,
        rates,
        "SELECT t.date, t.amount, a.currency FROM finance_transaction t
         JOIN finance_account a ON a.id = t.account_id
         WHERE t.date >= ?1 AND t.date < ?2 AND t.kind = ?3",
        &[
            &from.format(DATE_FORMAT).to_string(),
            &to.format(DATE_FORMAT).to_string(),
            &kind.as_str(),
        ],
        (to - from).num_days() as usize,
    )
}

/// Running sum of `daily` starting at `base`, cut to the first `shown` days.
fn card(daily: &[i64], base: i64, percent: Option<i64>, shown: usize) -> SummaryCard {
    let mut acc = base;
    let series = daily
        .iter()
        .map(|d| {
            acc = acc.saturating_add(*d);
            acc
        })
        .take(shown)
        .collect();
    SummaryCard {
        value: daily.iter().fold(base, |a, d| a.saturating_add(*d)),
        percent,
        series,
    }
}

fn minus(a: &[i64], b: &[i64]) -> Vec<i64> {
    a.iter().zip(b).map(|(a, b)| a.saturating_sub(*b)).collect()
}

/// The four cards of the month holding `month` (any day works) against the month before. In the
/// month of `today` the series stop at today; the values still cover the whole month.
pub fn month_cards(
    conn: &Connection,
    month: NaiveDate,
    today: NaiveDate,
) -> Result<MonthCards, FinanceError> {
    let first = month_start(month);
    let (Some(next), Some(prev)) = (
        first.checked_add_months(Months::new(1)),
        first.checked_sub_months(Months::new(1)),
    ) else {
        return Ok(MonthCards::default());
    };
    let days = (next - first).num_days() as usize;
    let prev_days = (first - prev).num_days() as usize;
    let shown = if today < first || today >= next {
        days
    } else {
        today.day() as usize
    };
    let rates = load_rates(conn)?;
    let income = kind_daily(conn, &rates, TxKind::Income, first, next)?;
    let expense = kind_daily(conn, &rates, TxKind::Expense, first, next)?;
    let prev_income = kind_daily(conn, &rates, TxKind::Income, prev, first)?;
    let prev_expense = kind_daily(conn, &rates, TxKind::Expense, prev, first)?;
    let net = minus(&income, &expense);
    let prev_net = minus(&prev_income, &prev_expense);

    // What every movement does to the active accounts: income adds, expenses and outgoing
    // transfers subtract, incoming transfers add.
    let flow = daily(
        conn,
        &rates,
        "SELECT t.date,
                CASE WHEN t.account_id = a.id
                    THEN CASE t.kind WHEN 'income' THEN t.amount ELSE -t.amount END
                    ELSE coalesce(t.to_amount, t.amount) END,
                a.currency
         FROM finance_transaction t
         JOIN finance_account a ON a.id = t.account_id OR a.id = t.to_account_id
         WHERE a.archived = 0 AND t.date >= ?1 AND t.date < ?2",
        &[
            &first.format(DATE_FORMAT).to_string(),
            &next.format(DATE_FORMAT).to_string(),
        ],
        days,
    )?;
    let opening = accounts_total_before(conn, &rates, first)?;

    let total = |d: &[i64]| d.iter().fold(0i64, |a, v| a.saturating_add(*v));
    let (income_card, expense_card) = (
        card(&income, 0, None, shown),
        card(&expense, 0, None, shown),
    );
    let percent = |value: i64, previous: &[i64]| percent_change(value, total(previous));
    Ok(MonthCards {
        total: card(&flow, opening, None, shown),
        balance: card(&net, 0, percent(total(&net), &prev_net), shown),
        income: SummaryCard {
            percent: percent(income_card.value, &prev_income),
            ..income_card
        },
        expense: SummaryCard {
            percent: percent(expense_card.value, &prev_expense),
            ..expense_card
        },
        previous: card(&prev_net, 0, None, prev_days),
        shown,
        span: days.max(prev_days),
    })
}

/// Categories kept apart in the donut; further ones are summed into "Others".
const SLICES: usize = 5;

/// One slice of the donut.
#[derive(Clone, Debug, PartialEq)]
pub struct CategorySlice {
    /// `None` is the movements without category (or, with `others`, the rest).
    pub name: Option<String>,
    /// The sum of the categories beyond the biggest ones.
    pub others: bool,
    pub value: i64,
    /// Whole percent of the total, rounded half up (the slices can add up to 99 or 101).
    pub percent: i64,
    /// Fraction of the ring, 0..=1.
    pub share: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CategoryBreakdown {
    pub total: i64,
    /// Biggest first.
    pub slices: Vec<CategorySlice>,
}

/// What the month moved per category of `kind`: the biggest few, then the rest as one slice.
pub fn category_breakdown(
    conn: &Connection,
    month: NaiveDate,
    kind: TxKind,
) -> Result<CategoryBreakdown, FinanceError> {
    let mut slices: Vec<(Option<String>, bool, i64)> = category_totals(conn, month, kind)?
        .into_iter()
        .map(|t| (t.name, false, t.total))
        .collect();
    let total = slices.iter().fold(0i64, |a, s| a.saturating_add(s.2));
    if slices.len() > SLICES {
        let rest = slices
            .split_off(SLICES)
            .iter()
            .fold(0i64, |a, s| a.saturating_add(s.2));
        slices.push((None, true, rest));
    }
    let den = (total as i128).max(1);
    Ok(CategoryBreakdown {
        total,
        slices: slices
            .into_iter()
            .map(|(name, others, value)| CategorySlice {
                name,
                others,
                value,
                percent: ((value as i128 * 100 + den / 2) / den) as i64,
                share: (value as f64 / den as f64) as f32,
            })
            .collect(),
    })
}

/// The newest `limit` movements of the month, of the kind `filter` keeps.
pub fn recent_movements(
    conn: &Connection,
    month: NaiveDate,
    filter: MovementFilter,
    limit: usize,
) -> Result<Vec<Movement>, FinanceError> {
    let mut all = list_month(conn, month, filter)?;
    all.truncate(limit);
    Ok(all)
}

/// Which pending payments the Upcoming list keeps. One-time payments only count in `All`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpcomingFilter {
    All,
    Bills,
    Installments,
    Recurring,
}

impl UpcomingFilter {
    fn keeps(self, form: ScheduleForm) -> bool {
        match self {
            UpcomingFilter::All => true,
            UpcomingFilter::Bills => false,
            UpcomingFilter::Installments => matches!(form, ScheduleForm::Installments(_)),
            UpcomingFilter::Recurring => matches!(form, ScheduleForm::Recurring(_)),
        }
    }

    fn has_bills(self) -> bool {
        matches!(self, UpcomingFilter::All | UpcomingFilter::Bills)
    }
}

/// What is due in one month, for the bars.
#[derive(Clone, Debug, PartialEq)]
pub struct UpcomingMonth {
    /// Its first day.
    pub month: NaiveDate,
    /// In the main currency.
    pub total: i64,
    /// Total relative to the biggest month, 0..=1; 0 when none has anything due.
    pub share: f32,
}

/// A pending payment of the list, in its own currency.
#[derive(Clone, Debug, PartialEq)]
pub struct UpcomingItem {
    /// Empty for a scheduled payment without description; the card's name for a bill.
    pub description: String,
    pub bill: bool,
    pub date: NaiveDate,
    pub amount: i64,
    pub currency: &'static Currency,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Upcoming {
    /// The month of `today` and the next three.
    pub months: Vec<UpcomingMonth>,
    /// The first pending payments by date, overdue ones included.
    pub items: Vec<UpcomingItem>,
}

/// Months the Upcoming bars show.
const BARS: u32 = 4;
/// How far ahead the list looks for payments.
const HORIZON: u32 = 12;

/// The pending expenses (scheduled occurrences and card bills) that `filter` keeps: the total
/// of each of the next months and the first `limit` payments.
pub fn upcoming_payments(
    conn: &Connection,
    today: NaiveDate,
    filter: UpcomingFilter,
    limit: usize,
) -> Result<Upcoming, FinanceError> {
    let this_month = month_start(today);
    let rates = load_rates(conn)?;
    let (mut months, mut items) = (Vec::new(), Vec::new());
    for n in 0..HORIZON {
        let Some(first) = this_month.checked_add_months(Months::new(n)) else {
            break;
        };
        if n >= BARS && items.len() >= limit {
            break;
        }
        let mut month_items: Vec<UpcomingItem> = due_in_month(conn, first, today)?
            .into_iter()
            .filter(|r| r.scheduled.kind == TxKind::Expense && filter.keeps(r.scheduled.form))
            .map(|r| UpcomingItem {
                description: r.scheduled.description,
                bill: false,
                date: r.date,
                amount: r.scheduled.amount,
                currency: r.scheduled.currency,
            })
            .collect();
        if filter.has_bills() {
            month_items.extend(
                bills_due_in_month(conn, first, today)?
                    .into_iter()
                    .map(|b| UpcomingItem {
                        description: b.card_name,
                        bill: true,
                        date: b.due,
                        amount: b.pending,
                        currency: b.currency,
                    }),
            );
        }
        month_items.sort_by_key(|i| i.date);
        if n < BARS {
            let mut total = 0i64;
            for i in &month_items {
                total = total.saturating_add(rates.to_main(i.amount, i.currency)?);
            }
            months.push(UpcomingMonth {
                month: first,
                total,
                share: 0.0,
            });
        }
        items.extend(month_items);
    }
    items.truncate(limit);
    let max = months.iter().map(|m| m.total).max().unwrap_or(0);
    if max > 0 {
        for m in &mut months {
            m.share = (m.total.max(0) as f64 / max as f64) as f32;
        }
    }
    Ok(Upcoming { months, items })
}

/// Which accounts the Summary's Accounts card lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountFilter {
    All,
    /// Everything that is not credit.
    Accounts,
    /// Cards and credit lines.
    Credit,
}

impl AccountFilter {
    pub fn keeps(self, kind: super::AccountKind) -> bool {
        use super::AccountKind::{Credit, CreditLine};
        match self {
            AccountFilter::All => true,
            AccountFilter::Accounts => !matches!(kind, Credit | CreditLine),
            AccountFilter::Credit => matches!(kind, Credit | CreditLine),
        }
    }
}
