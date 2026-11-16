// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Goals and projects: create, view, edit and delete.

use std::fmt;

use chrono::NaiveDate;
use rusqlite::{Connection, params};

use crate::finances::{self, AccountId, Currency, FinanceError, ReserveId};

mod entries;
mod pace;
mod tasks;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_financial;

pub use entries::{Entry, EntryId, add_entry, delete_entry, list_entries};
pub use pace::{Pace, PaceLabel, Portfolio, expected_pace, pace_label, portfolio, timeline};
pub use tasks::{GoalTask, create_task, list_tasks};

const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoalId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GoalKind {
    Goal,
    Project,
}

impl GoalKind {
    fn as_str(self) -> &'static str {
        match self {
            GoalKind::Goal => "goal",
            GoalKind::Project => "project",
        }
    }
}

/// The numeric measure of a goal: reach `target` `unit`s by adding up entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Numeric {
    pub target: i64,
    pub unit: String,
}

/// Progress toward a numeric goal's target.
#[derive(Clone, Debug, PartialEq)]
pub struct NumericProgress {
    pub target: i64,
    pub unit: String,
    /// Sum of the entries; it may pass `target`.
    pub total: i64,
    /// `total` over `target` as a whole 0..=100, truncated and capped.
    pub percent: u32,
}

/// The money measure: reserve `target` (typed in the account's currency) in the goal's own
/// reserve, which is created with the goal in `account`.
#[derive(Clone, Debug, PartialEq)]
pub struct Financial {
    pub target: String,
    pub account: AccountId,
}

/// Progress toward a financial goal's target, in its account's currency.
#[derive(Clone, Debug, PartialEq)]
pub struct FinancialProgress {
    pub target: i64,
    /// The reserve's balance; it may pass `target`.
    pub reserved: i64,
    /// `reserved` over `target` as a whole 0..=100, truncated and capped.
    pub percent: u32,
    pub currency: &'static Currency,
    pub reserve: ReserveId,
    pub account: AccountId,
    pub account_name: String,
}

/// What the user types when creating or editing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoalDraft {
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub deadline: Option<NaiveDate>,
    /// Only goals (not projects) take a measure, at most one, and it never changes after
    /// creating; editing changes only its target (and the unit).
    pub numeric: Option<Numeric>,
    pub financial: Option<Financial>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: GoalId,
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub kind: GoalKind,
    pub deadline: Option<NaiveDate>,
    /// Days from `today` to the deadline (negative once passed); `None` without a deadline.
    pub days_left: Option<i64>,
    pub tasks_done: u32,
    pub tasks_total: u32,
    /// `tasks_done` over `tasks_total` as a whole 0..=100; 0 without tasks.
    pub tasks_percent: u32,
    pub numeric: Option<NumericProgress>,
    pub financial: Option<FinancialProgress>,
    /// What is compared with the pace: tasks for a project, the measure for a goal with one;
    /// `None` for a goal without a measure, even with tasks.
    pub progress: Option<u32>,
    /// Only with `progress` and a deadline, and never once completed.
    pub pace: Option<Pace>,
    pub completed: bool,
    pub archived: bool,
}

/// The views of the goals list; `in_tab` says which goals each one shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tab {
    All,
    Goals,
    Projects,
    Completed,
    Archived,
}

/// Archived goals show only in `Archived`; the other tabs hide them.
pub fn in_tab(goal: &Goal, tab: Tab) -> bool {
    match tab {
        Tab::Archived => goal.archived,
        _ if goal.archived => false,
        Tab::All => true,
        Tab::Goals => goal.kind == GoalKind::Goal,
        Tab::Projects => goal.kind == GoalKind::Project,
        Tab::Completed => goal.completed,
    }
}

/// Whole percent of `done` over `total`, truncated; 0 when there is nothing to do.
pub fn percent(done: u32, total: u32) -> u32 {
    (done * 100).checked_div(total).unwrap_or(0)
}

/// A whole number typed in a field: digits, optionally grouped by `.` in thousands, from 1 to
/// 1000 million (so sums of a few entries still fit the UI's i32).
pub fn parse_count(text: &str) -> Result<i64, GoalError> {
    let text = text.trim();
    let mut groups = text.split('.');
    let first = groups.next().unwrap_or_default();
    let grouped = !text.contains('.') || (first.len() <= 3 && groups.all(|g| g.len() == 3));
    let digits = text.replace('.', "");
    if !grouped || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(GoalError::BadNumber);
    }
    match digits.parse::<i64>() {
        Ok(n) if (1..=1_000_000_000).contains(&n) => Ok(n),
        _ => Err(GoalError::BadNumber),
    }
}

#[derive(Debug)]
pub enum GoalError {
    EmptyTitle,
    NotFound,
    /// A project takes no measure.
    ProjectMeasure,
    /// A goal takes one measure: numeric or financial.
    TwoMeasures,
    /// The target amount is not a valid amount above 0 in the account's currency.
    BadAmount,
    /// The goal's reserve still holds money, so the goal cannot be deleted.
    ReserveNotEmpty,
    /// The reserve or its account refused it.
    Finance(FinanceError),
    /// The target is not above 0.
    BadTarget,
    /// Editing tried to add or remove the measure.
    MeasureChanged,
    /// An entry value is not above 0.
    BadValue,
    /// An entry for a goal that does not exist or has no measure.
    NotNumeric,
    /// Text that is not a positive whole number.
    BadNumber,
    /// A stored kind or date that cannot be read.
    BadDate(String),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for GoalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoalError::EmptyTitle => write!(f, "the title is empty"),
            GoalError::NotFound => write!(f, "goal not found"),
            GoalError::ProjectMeasure => write!(f, "a project takes no measure"),
            GoalError::TwoMeasures => write!(f, "a goal takes one measure"),
            GoalError::BadAmount => write!(f, "the target amount is not valid"),
            GoalError::ReserveNotEmpty => write!(f, "the goal's reserve holds money"),
            GoalError::Finance(e) => write!(f, "{e}"),
            GoalError::BadTarget => write!(f, "the target must be above 0"),
            GoalError::MeasureChanged => write!(f, "the measure cannot change"),
            GoalError::BadValue => write!(f, "the value must be above 0"),
            GoalError::NotNumeric => write!(f, "the goal has no numeric measure"),
            GoalError::BadNumber => write!(f, "not a positive whole number"),
            GoalError::BadDate(d) => write!(f, "invalid stored goal value: {d}"),
            GoalError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for GoalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GoalError::Sqlite(e) => Some(e),
            GoalError::Finance(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for GoalError {
    fn from(e: rusqlite::Error) -> Self {
        GoalError::Sqlite(e)
    }
}

impl From<FinanceError> for GoalError {
    fn from(e: FinanceError) -> Self {
        match e {
            FinanceError::Sqlite(e) => GoalError::Sqlite(e),
            e => GoalError::Finance(e),
        }
    }
}

/// Days from `today` to `deadline` (negative once it has passed).
pub fn days_left(deadline: NaiveDate, today: NaiveDate) -> i64 {
    (deadline - today).num_days()
}

fn title(draft: &GoalDraft) -> Result<&str, GoalError> {
    match draft.title.trim() {
        "" => Err(GoalError::EmptyTitle),
        t => Ok(t),
    }
}

fn deadline_text(draft: &GoalDraft) -> Option<String> {
    draft.deadline.map(|d| d.format(DATE_FORMAT).to_string())
}

/// The target and trimmed unit to store; the target must be above 0.
fn measure(draft: &GoalDraft) -> Result<(Option<i64>, Option<&str>), GoalError> {
    match &draft.numeric {
        None => Ok((None, None)),
        Some(n) if n.target <= 0 => Err(GoalError::BadTarget),
        Some(n) => Ok((Some(n.target), Some(n.unit.trim()))),
    }
}

/// The target amount typed for `account`, in the smallest unit of its currency.
fn target_amount(conn: &Connection, f: &Financial) -> Result<i64, GoalError> {
    let account = finances::get_account(conn, f.account)?;
    finances::parse_amount(&f.target, account.currency)
        .filter(|a| *a > 0)
        .ok_or(GoalError::BadAmount)
}

/// Creates a goal or project. A financial goal gets its empty reserve, named like it, in the
/// same transaction: the account must be an active plain one.
pub fn create(conn: &Connection, kind: GoalKind, draft: &GoalDraft) -> Result<GoalId, GoalError> {
    let title = title(draft)?;
    let measured = draft.numeric.is_some() || draft.financial.is_some();
    if measured && kind == GoalKind::Project {
        return Err(GoalError::ProjectMeasure);
    }
    if draft.numeric.is_some() && draft.financial.is_some() {
        return Err(GoalError::TwoMeasures);
    }
    let (target, unit) = measure(draft)?;
    let tx = conn.unchecked_transaction()?;
    let amount = draft
        .financial
        .as_ref()
        .map(|f| target_amount(&tx, f))
        .transpose()?;
    tx.execute(
        "INSERT INTO goal (title, emoji, description, kind, deadline, target_count, unit,
         target_amount) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            title,
            draft.emoji.trim(),
            draft.description.trim(),
            kind.as_str(),
            deadline_text(draft),
            target,
            unit,
            amount
        ],
    )?;
    let id = tx.last_insert_rowid();
    if let Some(f) = &draft.financial {
        let reserve = finances::create_reserve(&tx, title, f.account)?;
        tx.execute(
            "UPDATE finance_reserve SET goal_id = ?1 WHERE id = ?2",
            [id, reserve.0],
        )?;
    }
    tx.commit()?;
    Ok(GoalId(id))
}

/// Edits the text and the deadline, and the target (and unit) of a goal with a measure; the
/// reserve of a financial goal takes the new title. The kind, the measure and the account never
/// change, and `None` clears the deadline.
pub fn update(conn: &Connection, id: GoalId, draft: &GoalDraft) -> Result<(), GoalError> {
    let title = title(draft)?;
    let (target, unit) = measure(draft)?;
    let tx = conn.unchecked_transaction()?;
    let (numeric, account): (bool, Option<i64>) = match tx.query_row(
        "SELECT g.target_count IS NOT NULL, r.account_id
         FROM goal g LEFT JOIN finance_reserve r ON r.goal_id = g.id WHERE g.id = ?1",
        [id.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ) {
        Ok(n) => n,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(GoalError::NotFound),
        Err(e) => return Err(e.into()),
    };
    let same_account = account == draft.financial.as_ref().map(|f| f.account.0);
    if numeric != draft.numeric.is_some() || !same_account {
        return Err(GoalError::MeasureChanged);
    }
    let amount = draft
        .financial
        .as_ref()
        .map(|f| target_amount(&tx, f))
        .transpose()?;
    tx.execute(
        "UPDATE goal SET title = ?1, emoji = ?2, description = ?3, deadline = ?4,
         target_count = ?5, unit = ?6, target_amount = ?7 WHERE id = ?8",
        params![
            title,
            draft.emoji.trim(),
            draft.description.trim(),
            deadline_text(draft),
            target,
            unit,
            amount,
            id.0
        ],
    )?;
    tx.execute(
        "UPDATE finance_reserve SET name = ?1 WHERE goal_id = ?2",
        params![title, id.0],
    )?;
    tx.commit()?;
    Ok(())
}

/// Completes the goal now. It is final: completing again keeps the first date.
pub fn complete(conn: &Connection, id: GoalId) -> Result<(), GoalError> {
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        "UPDATE goal SET completed_at = coalesce(completed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE id = ?1",
        [id.0],
    )?;
    if n == 0 {
        return Err(GoalError::NotFound);
    }
    Ok(tx.commit()?)
}

/// Archives or restores a goal without touching its completion.
pub fn set_archived(conn: &Connection, id: GoalId, archived: bool) -> Result<(), GoalError> {
    let tx = conn.unchecked_transaction()?;
    if tx.execute(
        "UPDATE goal SET archived = ?1 WHERE id = ?2",
        params![archived, id.0],
    )? == 0
    {
        return Err(GoalError::NotFound);
    }
    Ok(tx.commit()?)
}

/// Completes now every uncompleted goal (archived too) whose progress reached 100 %: a numeric
/// goal whose entries add up to the target, a financial goal whose reserve holds the target, or
/// a project with tasks that are all done. A goal without a measure never completes by itself.
/// Returns how many it completed.
pub fn complete_reached(conn: &Connection) -> Result<usize, GoalError> {
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        &format!("UPDATE goal SET completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE completed_at IS NULL AND CASE
           WHEN target_count IS NOT NULL THEN
             (SELECT coalesce(sum(value), 0) FROM goal_entry WHERE goal_id = goal.id) >= target_count
           WHEN target_amount IS NOT NULL THEN
             (SELECT {RESERVED} FROM finance_reserve r WHERE r.goal_id = goal.id) >= target_amount
           WHEN kind = 'project' THEN
             EXISTS (SELECT 1 FROM task WHERE goal_id = goal.id)
             AND NOT EXISTS (SELECT 1 FROM task WHERE goal_id = goal.id AND done_at IS NULL)
           ELSE 0 END"
        ),
        [],
    )?;
    tx.commit()?;
    Ok(n)
}

/// Deletes the goal with its entries and its reserve, which must be empty; its tasks stay,
/// unlinked.
pub fn delete(conn: &Connection, id: GoalId) -> Result<(), GoalError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE task SET goal_id = NULL WHERE goal_id = ?1", [id.0])?;
    tx.execute("DELETE FROM goal_entry WHERE goal_id = ?1", [id.0])?;
    let reserved: i64 = tx.query_row(
        &format!("SELECT coalesce(sum({RESERVED}), 0) FROM finance_reserve r WHERE r.goal_id = ?1"),
        [id.0],
        |r| r.get(0),
    )?;
    if reserved != 0 {
        return Err(GoalError::ReserveNotEmpty);
    }
    // Its moves go with it (ON DELETE CASCADE).
    tx.execute("DELETE FROM finance_reserve WHERE goal_id = ?1", [id.0])?;
    if tx.execute("DELETE FROM goal WHERE id = ?1", [id.0])? == 0 {
        return Err(GoalError::NotFound);
    }
    tx.commit()?;
    Ok(())
}

/// The balance of the reserve `r`: deposits minus withdrawals.
const RESERVED: &str =
    "coalesce((SELECT sum(CASE m.kind WHEN 'deposit' THEN m.amount ELSE -m.amount END)
    FROM finance_reserve_move m WHERE m.reserve_id = r.id), 0)";

fn select() -> String {
    format!(
        "SELECT goal.id, goal.title, goal.emoji, goal.description, goal.kind, goal.deadline,
    (SELECT count(*) FROM task WHERE goal_id = goal.id AND done_at IS NOT NULL),
    (SELECT count(*) FROM task WHERE goal_id = goal.id),
    goal.target_count, coalesce(goal.unit, ''),
    (SELECT coalesce(sum(value), 0) FROM goal_entry WHERE goal_id = goal.id),
    goal.created_at, goal.completed_at IS NOT NULL, goal.archived,
    goal.target_amount, r.id, a.id, a.name, a.currency, {RESERVED}
    FROM goal LEFT JOIN finance_reserve r ON r.goal_id = goal.id
    LEFT JOIN finance_account a ON a.id = r.account_id"
    )
}

fn row(r: &rusqlite::Row, today: NaiveDate) -> rusqlite::Result<Result<Goal, GoalError>> {
    let kind: String = r.get(4)?;
    let deadline: Option<String> = r.get(5)?;
    let (id, title, emoji, description) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (tasks_done, tasks_total): (u32, u32) = (r.get(6)?, r.get(7)?);
    let (target, unit, total): (Option<i64>, String, i64) = (r.get(8)?, r.get(9)?, r.get(10)?);
    let created_at: String = r.get(11)?;
    let (completed, archived): (bool, bool) = (r.get(12)?, r.get(13)?);
    let financial = match (r.get::<_, Option<i64>>(14)?, r.get::<_, Option<i64>>(15)?) {
        (Some(target), Some(reserve)) => {
            let code: String = r.get(18)?;
            let Some(currency) = finances::currency(&code) else {
                return Ok(Err(GoalError::BadDate(code)));
            };
            let reserved: i64 = r.get(19)?;
            Some(FinancialProgress {
                target,
                reserved,
                percent: (reserved.max(0).saturating_mul(100) / target).min(100) as u32,
                currency,
                reserve: ReserveId(reserve),
                account: AccountId(r.get(16)?),
                account_name: r.get(17)?,
            })
        }
        _ => None,
    };
    let kind = match kind.as_str() {
        "goal" => GoalKind::Goal,
        "project" => GoalKind::Project,
        _ => return Ok(Err(GoalError::BadDate(kind))),
    };
    let deadline = match deadline {
        None => None,
        Some(d) => match NaiveDate::parse_from_str(&d, DATE_FORMAT) {
            Ok(d) => Some(d),
            Err(_) => return Ok(Err(GoalError::BadDate(d))),
        },
    };
    let numeric = target.map(|target| NumericProgress {
        target,
        unit,
        total,
        percent: (total.saturating_mul(100) / target).min(100) as u32,
    });
    let tasks_percent = percent(tasks_done, tasks_total);
    let progress = match (&numeric, &financial, kind) {
        (Some(n), _, _) => Some(n.percent),
        (_, Some(f), _) => Some(f.percent),
        (None, None, GoalKind::Project) => Some(tasks_percent),
        (None, None, GoalKind::Goal) => None,
    };
    let pace = if completed {
        None
    } else {
        match pace::pace(progress, &created_at, deadline, today) {
            Ok(p) => p,
            Err(e) => return Ok(Err(e)),
        }
    };
    Ok(Ok(Goal {
        id: GoalId(id),
        title,
        emoji,
        description,
        kind,
        deadline,
        days_left: deadline.map(|d| days_left(d, today)),
        tasks_done,
        tasks_total,
        tasks_percent,
        numeric,
        financial,
        progress,
        pace,
        completed,
        archived,
    }))
}

pub fn get(conn: &Connection, id: GoalId, today: NaiveDate) -> Result<Goal, GoalError> {
    match conn.query_row(&format!("{} WHERE goal.id = ?1", select()), [id.0], |r| {
        row(r, today)
    }) {
        Ok(g) => g,
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(GoalError::NotFound),
        Err(e) => Err(e.into()),
    }
}

/// Nearest deadline first, undated last, then creation order.
pub fn list(conn: &Connection, today: NaiveDate) -> Result<Vec<Goal>, GoalError> {
    let mut stmt = conn.prepare(&format!(
        "{} ORDER BY goal.deadline IS NULL, goal.deadline, goal.created_at, goal.id",
        select()
    ))?;
    let rows = stmt.query_map([], |r| row(r, today))?;
    rows.map(|r| r?).collect()
}
