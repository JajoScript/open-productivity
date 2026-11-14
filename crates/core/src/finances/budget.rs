// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The budget: buckets that each get a percentage of a month's income, and the expense
//! categories that count against them.

use std::collections::HashMap;

use chrono::{Months, NaiveDate};
use rusqlite::{Connection, OptionalExtension};

use super::FinanceError;
use super::categories::{CategoryId, get_category};
use super::money::currency;
use super::rates::load_rates;
use super::summary::kind_sum;
use super::transactions::{DATE_FORMAT, TxKind};
use crate::calendar::month_start;
use crate::instance_settings;

const BASE_KEY: &str = "finance.budget_base";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BucketId(pub i64);

/// Whose income the percentages apply to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BudgetBase {
    /// The income of the month shown.
    Current,
    /// The income of the month before the one shown.
    Previous,
}

#[derive(Debug, PartialEq)]
pub struct Bucket {
    pub id: BucketId,
    pub name: String,
    pub archived: bool,
    /// Some category belongs to it, so it cannot be deleted.
    pub in_use: bool,
}

#[derive(Debug, PartialEq)]
pub struct BucketShare {
    pub id: BucketId,
    pub name: String,
    pub percent: i64,
    /// `base * percent / 100`, rounded down, in the main currency.
    pub assigned: i64,
    /// The month's expenses in the bucket's categories, in the main currency.
    pub spent: i64,
    /// `assigned - spent`; negative when overspent.
    pub remaining: i64,
    pub overspent: bool,
    /// Spent against assigned, per thousand (not capped); 0 when nothing is assigned.
    pub spent_per_mille: i64,
}

#[derive(Debug, PartialEq)]
pub struct BudgetSummary {
    /// The income the percentages apply to, in the main currency (transfers excluded).
    pub base: i64,
    /// The month whose income `base` is.
    pub base_month: NaiveDate,
    /// Active buckets by position.
    pub buckets: Vec<BucketShare>,
    /// Base minus what the active buckets get; an archived bucket's share falls here.
    pub unassigned: i64,
}

fn text(d: NaiveDate) -> String {
    d.format(DATE_FORMAT).to_string()
}

pub fn budget_base(conn: &Connection) -> Result<BudgetBase, FinanceError> {
    Ok(match instance_settings::get(conn, BASE_KEY)?.as_deref() {
        Some("previous") => BudgetBase::Previous,
        _ => BudgetBase::Current,
    })
}

pub fn set_budget_base(conn: &Connection, base: BudgetBase) -> Result<(), FinanceError> {
    let v = match base {
        BudgetBase::Current => "current",
        BudgetBase::Previous => "previous",
    };
    instance_settings::set(conn, BASE_KEY, v)?;
    Ok(())
}

/// The budget of the month holding `month`: its percentages and its spending against them.
pub fn budget_summary(conn: &Connection, month: NaiveDate) -> Result<BudgetSummary, FinanceError> {
    let from = month_start(month);
    let rates = load_rates(conn)?;
    let base_month = match budget_base(conn)? {
        BudgetBase::Current => Some(from),
        BudgetBase::Previous => from.checked_sub_months(Months::new(1)),
    };
    let (Some(base_month), Some(to)) = (base_month, from.checked_add_months(Months::new(1))) else {
        return Ok(BudgetSummary {
            base: 0,
            base_month: from,
            buckets: Vec::new(),
            unassigned: 0,
        });
    };
    let base = match base_month.checked_add_months(Months::new(1)) {
        Some(end) => kind_sum(conn, &rates, TxKind::Income, base_month, end)?,
        None => 0,
    };

    // Each bucket's percent comes from the latest month up to this one with any share.
    let mut stmt = conn.prepare(
        "SELECT b.id, b.name, COALESCE(s.percent, 0)
         FROM finance_budget_bucket b
         LEFT JOIN finance_budget_share s ON s.bucket_id = b.id AND s.month =
             (SELECT MAX(month) FROM finance_budget_share WHERE month <= ?1)
         WHERE b.archived = 0
         ORDER BY b.position, b.id",
    )?;
    let mut buckets = stmt
        .query_map([text(from)], |r| {
            let percent: i64 = r.get(2)?;
            let assigned = (i128::from(base) * i128::from(percent) / 100) as i64;
            Ok(BucketShare {
                id: BucketId(r.get(0)?),
                name: r.get(1)?,
                percent,
                assigned,
                spent: 0,
                remaining: assigned,
                overspent: false,
                spent_per_mille: 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Expenses without category, or in a category without bucket, count nowhere.
    let mut stmt = conn.prepare(
        "SELECT c.bucket_id, a.currency, t.amount
         FROM finance_transaction t
         JOIN finance_category c ON c.id = t.category_id
         JOIN finance_account a ON a.id = t.account_id
         WHERE t.kind = 'expense' AND c.bucket_id IS NOT NULL
           AND t.date >= ?1 AND t.date < ?2",
    )?;
    let rows = stmt.query_map([text(from), text(to)], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut spent: HashMap<i64, i64> = HashMap::new();
    for row in rows {
        let (bucket, code, amount) = row?;
        let c = currency(&code).ok_or(FinanceError::UnknownCurrency(code))?;
        let e = spent.entry(bucket).or_insert(0);
        *e = e.saturating_add(rates.to_main(amount, c)?);
    }
    let mut assigned_total = 0i64;
    for b in &mut buckets {
        b.spent = spent.get(&b.id.0).copied().unwrap_or(0);
        b.remaining = b.assigned.saturating_sub(b.spent);
        b.overspent = b.spent > b.assigned;
        if b.assigned > 0 {
            b.spent_per_mille = (i128::from(b.spent.max(0)) * 1000 / i128::from(b.assigned)) as i64;
        }
        assigned_total = assigned_total.saturating_add(b.assigned);
    }
    Ok(BudgetSummary {
        base,
        base_month,
        unassigned: base.saturating_sub(assigned_total),
        buckets,
    })
}

/// Active buckets by position, then the archived ones.
pub fn list_buckets(conn: &Connection) -> Result<Vec<Bucket>, FinanceError> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.name, b.archived,
            EXISTS (SELECT 1 FROM finance_category c WHERE c.bucket_id = b.id)
         FROM finance_budget_bucket b ORDER BY b.archived, b.position, b.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Bucket {
            id: BucketId(r.get(0)?),
            name: r.get(1)?,
            archived: r.get(2)?,
            in_use: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn create_bucket(conn: &Connection, name: &str) -> Result<BucketId, FinanceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FinanceError::EmptyName);
    }
    conn.execute(
        "INSERT INTO finance_budget_bucket (name, position)
         VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM finance_budget_bucket))",
        [name],
    )?;
    Ok(BucketId(conn.last_insert_rowid()))
}

fn found(changed: usize) -> Result<(), FinanceError> {
    if changed == 0 {
        Err(FinanceError::BucketNotFound)
    } else {
        Ok(())
    }
}

pub fn rename_bucket(conn: &Connection, id: BucketId, name: &str) -> Result<(), FinanceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FinanceError::EmptyName);
    }
    found(conn.execute(
        "UPDATE finance_budget_bucket SET name = ?1 WHERE id = ?2",
        (name, id.0),
    )?)
}

/// An archived bucket keeps its categories and shares but leaves the budget: its percentage
/// counts as unassigned. Restoring may push the active percentages past 100 until edited.
pub fn set_bucket_archived(
    conn: &Connection,
    id: BucketId,
    archived: bool,
) -> Result<(), FinanceError> {
    found(conn.execute(
        "UPDATE finance_budget_bucket SET archived = ?1 WHERE id = ?2",
        (archived, id.0),
    )?)
}

/// Only a bucket no category belongs to; its shares go with it.
pub fn delete_bucket(conn: &Connection, id: BucketId) -> Result<(), FinanceError> {
    let used: Option<bool> = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM finance_category WHERE bucket_id = ?1)
             FROM finance_budget_bucket WHERE id = ?1",
            [id.0],
            |r| r.get(0),
        )
        .optional()?;
    match used {
        None => Err(FinanceError::BucketNotFound),
        Some(true) => Err(FinanceError::BucketInUse),
        Some(false) => {
            conn.execute("DELETE FROM finance_budget_bucket WHERE id = ?1", [id.0])?;
            Ok(())
        }
    }
}

/// Puts an expense category in a bucket, or takes it out (`None`).
pub fn set_category_bucket(
    conn: &Connection,
    category: CategoryId,
    bucket: Option<BucketId>,
) -> Result<(), FinanceError> {
    if get_category(conn, category)?.kind != TxKind::Expense {
        return Err(FinanceError::CategoryKindMismatch);
    }
    if let Some(b) = bucket {
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM finance_budget_bucket WHERE id = ?1)",
            [b.0],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(FinanceError::BucketNotFound);
        }
    }
    conn.execute(
        "UPDATE finance_category SET bucket_id = ?1 WHERE id = ?2",
        (bucket.map(|b| b.0), category.0),
    )?;
    Ok(())
}

/// Sets one bucket's percent from `month` on. A month without shares of its own first copies
/// every row (archived buckets too) of the month it inherits from, so earlier months do not
/// change and archived buckets keep their percentages. The active buckets may not exceed 100.
pub fn set_share(
    conn: &Connection,
    month: NaiveDate,
    bucket: BucketId,
    percent: i64,
) -> Result<(), FinanceError> {
    let tx = conn.unchecked_transaction()?;
    write_share(&tx, month, bucket, percent)?; // an error drops the transaction, undoing it
    tx.commit()?;
    Ok(())
}

/// `set_share` inside the caller's transaction.
fn write_share(
    tx: &Connection,
    month: NaiveDate,
    bucket: BucketId,
    percent: i64,
) -> Result<(), FinanceError> {
    if !(0..=100).contains(&percent) {
        return Err(FinanceError::InvalidShares);
    }
    let month = text(month_start(month));
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM finance_budget_bucket WHERE id = ?1)",
        [bucket.0],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(FinanceError::BucketNotFound);
    }
    // Copies the latest earlier month; nothing to copy when this one already has rows.
    tx.execute(
        "INSERT INTO finance_budget_share (month, bucket_id, percent)
         SELECT ?1, bucket_id, percent FROM finance_budget_share
         WHERE month = (SELECT MAX(month) FROM finance_budget_share WHERE month < ?1)
           AND NOT EXISTS (SELECT 1 FROM finance_budget_share WHERE month = ?1)",
        [&month],
    )?;
    tx.execute(
        "INSERT INTO finance_budget_share (month, bucket_id, percent) VALUES (?1, ?2, ?3)
         ON CONFLICT (month, bucket_id) DO UPDATE SET percent = excluded.percent",
        (&month, bucket.0, percent),
    )?;
    let total: i64 = tx.query_row(
        "SELECT COALESCE(SUM(s.percent), 0) FROM finance_budget_share s
         JOIN finance_budget_bucket b ON b.id = s.bucket_id
         WHERE s.month = ?1 AND b.archived = 0",
        [&month],
        |r| r.get(0),
    )?;
    if total > 100 {
        return Err(FinanceError::InvalidShares);
    }
    Ok(())
}

/// The 50/30/20 budget (needs, wants, savings) from `month` on, in an instance without
/// buckets; the app supplies the translated names.
pub fn create_suggested_budget(
    conn: &Connection,
    month: NaiveDate,
    names: [&str; 3],
) -> Result<(), FinanceError> {
    let tx = conn.unchecked_transaction()?;
    if !list_buckets(&tx)?.is_empty() {
        return Err(FinanceError::BucketsExist);
    }
    for (name, percent) in names.into_iter().zip([50, 30, 20]) {
        let id = create_bucket(&tx, name)?;
        write_share(&tx, month, id, percent)?;
    }
    tx.commit()?;
    Ok(())
}
