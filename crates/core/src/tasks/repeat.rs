// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Repeating tasks: completing one creates the next, intervals and rules.

use super::*;

/// Makes the next task of a repeating one: same title, note, list, marks and times, pending and
/// in the first stage, due on the first repeat date after `today`. The rule moves to it, so
/// completing the old task again changes nothing.
pub(super) fn complete(conn: &Connection, id: TaskId, today: NaiveDate) -> Result<(), TaskError> {
    let rule = conn
        .query_row(
            "SELECT r.frequency, r.interval, r.anchor_day, t.due_date FROM task_recurrence r \
             JOIN task t ON t.id = r.task_id WHERE r.task_id = ?1 AND t.due_date IS NOT NULL",
            [id.0],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((freq, interval, anchor, due)) = rule else {
        return Ok(());
    };
    let freq = Frequency::parse(&freq).ok_or(TaskError::BadFrequency(freq))?;
    let mut next: NaiveDate = due.parse().map_err(|_| TaskError::BadDate(due))?;
    // Past repeats are skipped: a task left undone for months does not come back overdue.
    loop {
        next = recurrence::next_date(next, freq, interval, anchor)
            .ok_or_else(|| TaskError::BadDate(next.to_string()))?;
        if next > today {
            break;
        }
    }
    conn.execute(
        "INSERT INTO task (title, note, due_date, list_id, urgent, important, due_time, end_time) \
         SELECT title, note, ?1, list_id, urgent, important, due_time, end_time FROM task \
         WHERE id = ?2",
        (next.to_string(), id.0),
    )?;
    conn.execute(
        "UPDATE task_recurrence SET task_id = ?1 WHERE task_id = ?2",
        (conn.last_insert_rowid(), id.0),
    )?;
    Ok(())
}

/// The user's text for "every N": blank means 1, otherwise a whole number from 1 to 999.
pub fn parse_interval(text: &str) -> Result<u32, TaskError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(1);
    }
    text.parse()
        .ok()
        .filter(|n| (1..=999).contains(n))
        .ok_or_else(|| TaskError::BadInterval(text.to_string()))
}

/// Sets (`Some((frequency, every))`, `every` being the user's text, see `parse_interval`) or
/// clears the repeat rule. It needs a due date; an existing rule keeps its anchor day.
pub fn set_recurrence(
    conn: &Connection,
    id: TaskId,
    repeat: Option<(Frequency, &str)>,
) -> Result<(), TaskError> {
    let Some((freq, every)) = repeat else {
        conn.execute("DELETE FROM task_recurrence WHERE task_id = ?1", [id.0])?;
        return Ok(());
    };
    let every = parse_interval(every)?;
    let changed = conn.execute(
        "INSERT INTO task_recurrence (task_id, frequency, interval, anchor_day) \
         SELECT id, ?1, ?2, CAST(strftime('%d', due_date) AS INTEGER) FROM task \
         WHERE id = ?3 AND due_date IS NOT NULL \
         ON CONFLICT (task_id) DO UPDATE SET frequency = excluded.frequency, \
         interval = excluded.interval",
        (freq.as_str(), every, id.0),
    )?;
    if changed > 0 {
        return Ok(());
    }
    match conn.query_row("SELECT 1 FROM task WHERE id = ?1", [id.0], |_| Ok(())) {
        Ok(()) => Err(TaskError::RecurrenceWithoutDate),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(TaskError::NotFound),
        Err(e) => Err(e.into()),
    }
}
