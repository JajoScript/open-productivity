// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Task queries: views, week, kanban, Eisenhower and counts.

use super::*;

/// Tasks matching the view's rule, pending first, then done (most recently completed first).
/// All and lists: every task, pending oldest created first.
/// Smart views (Today, Scheduled, Inbox, Overdue): pending by due date, then creation, and
/// only the tasks done today (local day of `now`).
pub fn list(
    conn: &Connection,
    view: View,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Task>, TaskError> {
    let today = now.date_naive();
    let midnight = local_midnight(now)?;
    let (done, order) = if view.is_smart() {
        (
            "AND (t.done_at IS NULL OR t.done_at >= p.midnight)",
            "t.due_date, t.created_at, t.id",
        )
    } else {
        ("", "t.created_at, t.id")
    };
    let sql = format!(
        "WITH p(today, midnight) AS (SELECT ?1, ?2) {SELECT_TASKS} CROSS JOIN p \
         WHERE ({}) {done} \
         ORDER BY t.done_at IS NOT NULL, t.done_at DESC, {order}",
        view.rule()
    );
    query(conn, &sql, (today.to_string(), midnight))
}

/// Start of the local day of `now`, in the UTC text format of `done_at`, so text comparison
/// orders instants.
// shortcut: the fixed offset of `now` is used for midnight, so on a DST-change day the
// boundary can be off by an hour; upgrade with a tz-aware type if it matters.
pub(crate) fn local_midnight(now: DateTime<FixedOffset>) -> Result<String, TaskError> {
    let today = now.date_naive();
    Ok(today
        .and_time(NaiveTime::MIN)
        .and_local_timezone(*now.offset())
        .single()
        .ok_or_else(|| TaskError::BadDate(today.to_string()))?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// Tasks due on each of the 7 days from `monday`, pending first, then oldest created first.
pub fn week(conn: &Connection, monday: NaiveDate) -> Result<[Vec<Task>; 7], TaskError> {
    let sunday =
        calendar::shift(monday, 6).ok_or_else(|| TaskError::BadDate(monday.to_string()))?;
    let sql = format!(
        "{SELECT_TASKS} WHERE t.due_date BETWEEN ?1 AND ?2 \
         ORDER BY t.due_date, t.done_at IS NOT NULL, t.created_at, t.id"
    );
    let mut days: [Vec<Task>; 7] = Default::default();
    for t in query(conn, &sql, (monday.to_string(), sunday.to_string()))? {
        // In range by the query, so the offset is 0..=6.
        let i = t.due.map_or(0, |d| (d - monday).num_days()) as usize;
        days[i].push(t);
    }
    Ok(days)
}

/// The Icebox, In progress and Done columns. Pending ones by due date (undated last), then
/// creation; Done holds what was finished in the last `KANBAN_DONE_DAYS`, newest first.
pub fn kanban(conn: &Connection, now: DateTime<Utc>) -> Result<[Vec<Task>; 3], TaskError> {
    const PENDING_ORDER: &str = "ORDER BY t.due_date IS NULL, t.due_date, t.created_at, t.id";
    let since = (now - chrono::TimeDelta::days(KANBAN_DONE_DAYS))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    Ok([
        query(
            conn,
            &format!(
                "{SELECT_TASKS} WHERE t.done_at IS NULL AND NOT t.in_progress {PENDING_ORDER}"
            ),
            (),
        )?,
        query(
            conn,
            &format!("{SELECT_TASKS} WHERE t.done_at IS NULL AND t.in_progress {PENDING_ORDER}"),
            (),
        )?,
        query(
            conn,
            &format!("{SELECT_TASKS} WHERE t.done_at >= ?1 ORDER BY t.done_at DESC, t.id"),
            [since],
        )?,
    ])
}

pub(crate) const SELECT_TASKS: &str = "SELECT t.id, t.title, t.note, t.done_at IS NOT NULL, t.due_date, \
     t.list_id, l.name, t.urgent, t.important, t.due_time, t.end_time, r.frequency, r.interval FROM task t \
     LEFT JOIN list l ON l.id = t.list_id LEFT JOIN task_recurrence r ON r.task_id = t.id";

/// Runs a `SELECT_TASKS` query and parses the stored due dates.
pub(crate) fn query(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Task>, TaskError> {
    let map = |r: &rusqlite::Row| {
        let list = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<String>>(6)?) {
            (Some(id), Some(name)) => Some((ListId(id), name)),
            _ => None,
        };
        Ok((
            Task {
                id: TaskId(r.get(0)?),
                title: r.get(1)?,
                note: r.get(2)?,
                done: r.get(3)?,
                due: None,
                start: None,
                end: None,
                list,
                priority: Priority {
                    urgent: r.get(7)?,
                    important: r.get(8)?,
                },
                repeat: None,
            },
            [
                r.get::<_, Option<String>>(4)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
            ],
            r.get::<_, Option<u32>>(12)?,
        ))
    };
    conn.prepare(sql)?
        .query_map(params, map)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(mut t, [due, start, end, freq], every)| {
            t.due = due
                .map(|d| d.parse().map_err(|_| TaskError::BadDate(d)))
                .transpose()?;
            let time = |s: Option<String>| {
                s.map(|s| NaiveTime::parse_from_str(&s, "%H:%M").map_err(|_| TaskError::BadTime(s)))
                    .transpose()
            };
            t.start = time(start)?;
            t.end = time(end)?;
            t.repeat = match (freq, every) {
                (Some(f), Some(n)) => {
                    let freq = Frequency::parse(&f).ok_or(TaskError::BadFrequency(f))?;
                    Some((freq, n))
                }
                _ => None,
            };
            Ok(t)
        })
        .collect()
}

/// Pending tasks per quadrant, in `Priority::QUADRANTS` order; each by due date (undated
/// last), then creation.
pub fn eisenhower(conn: &Connection) -> Result<[Vec<Task>; 4], TaskError> {
    let sql = format!(
        "{SELECT_TASKS} WHERE t.done_at IS NULL \
         ORDER BY t.due_date IS NULL, t.due_date, t.created_at, t.id"
    );
    let mut quadrants: [Vec<Task>; 4] = Default::default();
    for t in query(conn, &sql, ())? {
        quadrants[t.priority.quadrant()].push(t);
    }
    Ok(quadrants)
}

/// Pending counts for the lists panel, with the same rules as `list`.
pub fn counts(conn: &Connection, now: DateTime<FixedOffset>) -> Result<Counts, TaskError> {
    let views = [
        View::Today,
        View::Scheduled,
        View::Inbox,
        View::Overdue,
        View::All,
    ];
    let columns: Vec<_> = views
        .iter()
        .map(|v| format!("count(CASE WHEN {} THEN 1 END)", v.rule()))
        .collect();
    let sql = format!(
        "WITH p(today) AS (SELECT ?1) SELECT {} FROM task t CROSS JOIN p WHERE t.done_at IS NULL",
        columns.join(", ")
    );
    let n = conn.query_row(&sql, [now.date_naive().to_string()], |r| {
        Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?])
    })?;
    let [today, scheduled, inbox, overdue, all] = n;
    let per_list = conn
        .prepare(
            "SELECT list_id, count(*) FROM task \
             WHERE done_at IS NULL AND list_id IS NOT NULL GROUP BY list_id",
        )?
        .query_map([], |r| Ok((ListId(r.get(0)?), r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(Counts {
        today,
        scheduled,
        inbox,
        overdue,
        all,
        per_list,
    })
}
