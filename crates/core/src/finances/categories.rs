// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Categories of expenses and income, their suggested set and the month's totals by category.

use chrono::{Months, NaiveDate};
use rusqlite::Connection;

use super::FinanceError;
use super::budget::BucketId;
use super::money::currency;
use super::rates::load_rates;
use super::transactions::TxKind;
use crate::calendar::month_start;

const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CategoryId(pub i64);

#[derive(Debug, PartialEq)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    pub kind: TxKind,
    pub archived: bool,
    /// The budget bucket an expense category counts in.
    pub bucket: Option<BucketId>,
    /// Some movement uses it, so it cannot be deleted.
    pub in_use: bool,
}

#[derive(Debug, PartialEq)]
pub struct CategoryTotal {
    /// `None`: the movements without category.
    pub category: Option<CategoryId>,
    pub name: Option<String>,
    pub total: i64,
}

/// The categories offered to start with; the app supplies the translated names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Suggested {
    Food,
    Transport,
    Housing,
    Bills,
    Health,
    Leisure,
    Shopping,
    Salary,
    OtherIncome,
}

impl Suggested {
    pub const ALL: [Suggested; 9] = [
        Suggested::Food,
        Suggested::Transport,
        Suggested::Housing,
        Suggested::Bills,
        Suggested::Health,
        Suggested::Leisure,
        Suggested::Shopping,
        Suggested::Salary,
        Suggested::OtherIncome,
    ];

    pub fn kind(self) -> TxKind {
        match self {
            Suggested::Salary | Suggested::OtherIncome => TxKind::Income,
            _ => TxKind::Expense,
        }
    }
}

const SELECT: &str = "SELECT c.id, c.name, c.kind, c.archived, c.bucket_id,
    EXISTS (SELECT 1 FROM finance_transaction t WHERE t.category_id = c.id)
        OR EXISTS (SELECT 1 FROM finance_scheduled s WHERE s.category_id = c.id)
    FROM finance_category c";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Result<Category, FinanceError>> {
    let kind: String = r.get(2)?;
    let (id, name, archived, bucket, in_use): (i64, String, bool, Option<i64>, bool) =
        (r.get(0)?, r.get(1)?, r.get(3)?, r.get(4)?, r.get(5)?);
    Ok(TxKind::parse(&kind).map(|kind| Category {
        id: CategoryId(id),
        name,
        kind,
        archived,
        bucket: bucket.map(BucketId),
        in_use,
    }))
}

pub fn get_category(conn: &Connection, id: CategoryId) -> Result<Category, FinanceError> {
    match conn.query_row(&format!("{SELECT} WHERE c.id = ?1"), [id.0], row) {
        Ok(c) => c,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(FinanceError::CategoryNotFound),
        Err(e) => Err(e.into()),
    }
}

/// Active first, then by name ignoring case.
pub fn list_categories(conn: &Connection, kind: TxKind) -> Result<Vec<Category>, FinanceError> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE c.kind = ?1"))?;
    let rows = stmt.query_map([kind.as_str()], row)?;
    let mut list = rows.map(|r| r?).collect::<Result<Vec<_>, _>>()?;
    list.sort_by_cached_key(|c| (c.archived, c.name.to_lowercase()));
    Ok(list)
}

pub fn has_categories(conn: &Connection) -> Result<bool, FinanceError> {
    Ok(
        conn.query_row("SELECT EXISTS (SELECT 1 FROM finance_category)", [], |r| {
            r.get(0)
        })?,
    )
}

/// Trims the name and rejects an empty one or one an active category of the kind already has
/// (ignoring case), leaving out `except` (the category being renamed or restored).
fn check_name<'a>(
    conn: &Connection,
    kind: TxKind,
    name: &'a str,
    except: Option<CategoryId>,
) -> Result<&'a str, FinanceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FinanceError::EmptyName);
    }
    let lower = name.to_lowercase();
    let taken = list_categories(conn, kind)?
        .iter()
        .any(|c| !c.archived && Some(c.id) != except && c.name.to_lowercase() == lower);
    if taken {
        return Err(FinanceError::DuplicateName);
    }
    Ok(name)
}

fn insert(conn: &Connection, kind: TxKind, name: &str) -> Result<CategoryId, FinanceError> {
    conn.execute(
        "INSERT INTO finance_category (name, kind) VALUES (?1, ?2)",
        (name, kind.as_str()),
    )?;
    Ok(CategoryId(conn.last_insert_rowid()))
}

pub fn create_category(
    conn: &Connection,
    kind: TxKind,
    name: &str,
) -> Result<CategoryId, FinanceError> {
    insert(conn, kind, check_name(conn, kind, name, None)?)
}

pub fn rename_category(conn: &Connection, id: CategoryId, name: &str) -> Result<(), FinanceError> {
    let c = get_category(conn, id)?;
    // An archived category can be renamed too; only active ones count as taken names.
    let name = check_name(conn, c.kind, name, Some(id))?;
    conn.execute(
        "UPDATE finance_category SET name = ?1 WHERE id = ?2",
        (name, id.0),
    )?;
    Ok(())
}

/// Restoring fails with `DuplicateName` if an active category of the kind has the same name.
pub fn set_category_archived(
    conn: &Connection,
    id: CategoryId,
    archived: bool,
) -> Result<(), FinanceError> {
    let c = get_category(conn, id)?;
    if !archived {
        check_name(conn, c.kind, &c.name, Some(id))?;
    }
    conn.execute(
        "UPDATE finance_category SET archived = ?1 WHERE id = ?2",
        (archived, id.0),
    )?;
    Ok(())
}

pub fn delete_category(conn: &Connection, id: CategoryId) -> Result<(), FinanceError> {
    if get_category(conn, id)?.in_use {
        return Err(FinanceError::InUse);
    }
    conn.execute("DELETE FROM finance_category WHERE id = ?1", [id.0])?;
    Ok(())
}

pub fn create_suggested(
    conn: &Connection,
    name: impl Fn(Suggested) -> String,
) -> Result<(), FinanceError> {
    let tx = conn.unchecked_transaction()?;
    if has_categories(&tx)? {
        return Err(FinanceError::CategoriesExist);
    }
    for s in Suggested::ALL {
        create_category(&tx, s.kind(), &name(s))?;
    }
    tx.commit()?;
    Ok(())
}

/// Sum of the month's movements of `kind` by category, biggest first, in the main currency;
/// only categories with movements appear. Each amount converts from its account's currency
/// before adding (saturating).
pub fn category_totals(
    conn: &Connection,
    month: NaiveDate,
    kind: TxKind,
) -> Result<Vec<CategoryTotal>, FinanceError> {
    let from = month_start(month);
    let Some(to) = from.checked_add_months(Months::new(1)) else {
        return Ok(Vec::new());
    };
    let rates = load_rates(conn)?;
    let mut stmt = conn.prepare(
        "SELECT t.category_id, c.name, a.currency, t.amount
         FROM finance_transaction t
         JOIN finance_account a ON a.id = t.account_id
         LEFT JOIN finance_category c ON c.id = t.category_id
         WHERE t.date >= ?1 AND t.date < ?2 AND t.kind = ?3",
    )?;
    let rows = stmt.query_map(
        (
            from.format(DATE_FORMAT).to_string(),
            to.format(DATE_FORMAT).to_string(),
            kind.as_str(),
        ),
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    let mut totals: Vec<CategoryTotal> = Vec::new();
    for row in rows {
        let (category, name, code, amount) = row?;
        let c = currency(&code).ok_or(FinanceError::UnknownCurrency(code))?;
        let amount = rates.to_main(amount, c)?;
        let category = category.map(CategoryId);
        match totals.iter_mut().find(|t| t.category == category) {
            Some(t) => t.total = t.total.saturating_add(amount),
            None => totals.push(CategoryTotal {
                category,
                name,
                total: amount,
            }),
        }
    }
    totals.sort_by_cached_key(|t| {
        (
            std::cmp::Reverse(t.total),
            t.name.as_ref().map(|n| n.to_lowercase()),
            t.category.map(|c| c.0),
        )
    });
    Ok(totals)
}
