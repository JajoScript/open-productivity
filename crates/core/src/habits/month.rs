// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Habit month grid, ranges, day totals and streak stats.

use super::*;

// (id, name, weekdays, start, done dates) per habit.
type RawHabit = (HabitId, String, i64, NaiveDate, HashSet<NaiveDate>);

/// Groups rows of (id, name, weekdays, start, done date or NULL), ordered by id, per habit.
fn group_habits(mut rows: rusqlite::Rows<'_>) -> Result<Vec<RawHabit>, HabitError> {
    let mut raw: Vec<RawHabit> = Vec::new();
    while let Some(r) = rows.next()? {
        let id = HabitId(r.get(0)?);
        if raw.last().is_none_or(|h| h.0 != id) {
            let start = parse_date(r.get(3)?)?;
            raw.push((id, r.get(1)?, r.get(2)?, start, HashSet::new()));
        }
        if let (Some(date), Some(h)) = (r.get::<_, Option<String>>(4)?, raw.last_mut()) {
            h.4.insert(parse_date(date)?);
        }
    }
    Ok(raw)
}

/// Streaks and the share of the shown month done.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HabitStats {
    /// Consecutive applicable done days up to today; an undone today does not break it.
    pub current: u32,
    /// Longest run of consecutive applicable done days.
    pub best: u32,
    /// Done share of the shown month's applicable days up to today, floored.
    pub month_pct: u32,
}

/// Stats of one habit. Streaks are as of `today`; the percentage is for the month holding
/// `shown`. Only days on a selected weekday and from `start` count: other marks are ignored.
pub(super) fn stats(
    weekdays: i64,
    start: NaiveDate,
    today: NaiveDate,
    shown: NaiveDate,
    done: &HashSet<NaiveDate>,
) -> HabitStats {
    // (date, done) of every applicable day from the start to today.
    let seq: Vec<(NaiveDate, bool)> = start
        .iter_days()
        .take_while(|d| *d <= today)
        .filter(|d| weekdays & weekday_bit(*d) != 0)
        .map(|d| (d, done.contains(&d)))
        .collect();
    let (mut best, mut run) = (0, 0);
    for &(_, ok) in &seq {
        run = if ok { run + 1 } else { 0 };
        best = best.max(run);
    }
    let mut current = 0;
    for (i, &(d, ok)) in seq.iter().rev().enumerate() {
        if ok {
            current += 1;
        } else if i > 0 || d != today {
            break;
        }
    }
    let in_month = seq
        .iter()
        .filter(|(d, _)| calendar::month_start(*d) == calendar::month_start(shown));
    let total = in_month.clone().count() as u32;
    let hits = in_month.filter(|(_, ok)| *ok).count() as u32;
    HabitStats {
        current,
        best,
        month_pct: (hits * 100).checked_div(total).unwrap_or(0),
    }
}

/// The month holding `shown`: every active habit with its state per day and its stats, from
/// one query over all their logs.
pub fn month(
    conn: &Connection,
    shown: NaiveDate,
    today: NaiveDate,
) -> Result<HabitMonth, HabitError> {
    let first = calendar::month_start(shown);
    let next = first
        .checked_add_months(Months::new(1))
        .ok_or(HabitError::OutOfRange)?;
    let len = (next - first).num_days() as u32;
    let mut stmt = conn.prepare(
        "SELECT h.id, h.name, h.weekdays, h.start_date, l.date          FROM habit h LEFT JOIN habit_log l ON l.habit_id = h.id          WHERE h.archived = 0 ORDER BY h.id",
    )?;
    let raw = group_habits(stmt.query([])?)?;
    let habits: Vec<MonthHabit> = raw
        .into_iter()
        .map(|(id, name, weekdays, start, done)| {
            let days = (0..len)
                .map(|i| {
                    let date = first + TimeDelta::days(i.into());
                    day_state(weekdays, start, date, today, done.contains(&date))
                })
                .collect();
            MonthHabit {
                id,
                name,
                days,
                stats: stats(weekdays, start, today, first, &done),
                weekdays: std::array::from_fn(|i| weekdays & (1 << i) != 0),
                start,
            }
        })
        .collect();
    let day_totals = day_totals(&habits, len as usize);
    let week_totals = day_totals.chunks(7).map(|c| c.iter().sum()).collect();
    let month_total = day_totals.iter().sum();
    let heat = day_totals
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let date = first + TimeDelta::days(i as i64);
            match t.pct() {
                None => Heat::None,
                _ if date > today => Heat::Future,
                Some(0) => Heat::L0,
                Some(1..=25) => Heat::L1,
                Some(26..=50) => Heat::L2,
                Some(51..=75) => Heat::L3,
                Some(_) => Heat::L4,
            }
        })
        .collect();
    Ok(HabitMonth {
        lead: first.weekday().num_days_from_monday(),
        month_total,
        heat,
        first,
        days: len,
        today: (calendar::month_start(today) == first).then(|| today.day()),
        can_next: next <= today,
        habits,
        day_totals,
        week_totals,
    })
}

/// What a habit shows on `date`: the one rule for which days apply, shared by every view.
fn day_state(
    weekdays: i64,
    start: NaiveDate,
    date: NaiveDate,
    today: NaiveDate,
    done: bool,
) -> DayState {
    if weekdays & weekday_bit(date) == 0 || date < start {
        DayState::Skipped
    } else if date > today {
        DayState::Future
    } else if done {
        DayState::Done
    } else {
        DayState::Open
    }
}

/// For each of `len` days from `from`, the active habits that apply that day (`done` is the mark;
/// future days are pending), in the order of the grid. One query over the range.
pub fn range(
    conn: &Connection,
    from: NaiveDate,
    len: u32,
    today: NaiveDate,
) -> Result<Vec<Vec<DayHabit>>, HabitError> {
    let to = from + TimeDelta::days(i64::from(len.max(1)) - 1);
    let mut stmt = conn.prepare(
        "SELECT h.id, h.name, h.weekdays, h.start_date, l.date FROM habit h
         LEFT JOIN habit_log l ON l.habit_id = h.id AND l.date BETWEEN ?1 AND ?2
         WHERE h.archived = 0 ORDER BY h.id",
    )?;
    let raw = group_habits(stmt.query((from.to_string(), to.to_string()))?)?;
    Ok((0..len)
        .map(|i| {
            let date = from + TimeDelta::days(i.into());
            raw.iter()
                .filter_map(|(id, name, weekdays, start, done)| {
                    let state = day_state(*weekdays, *start, date, today, done.contains(&date));
                    (state != DayState::Skipped).then(|| DayHabit {
                        id: *id,
                        name: name.clone(),
                        done: state == DayState::Done,
                    })
                })
                .collect()
        })
        .collect())
}

/// Per day: habits done and habits that apply (a done mark only counts on an applicable day).
fn day_totals(habits: &[MonthHabit], len: usize) -> Vec<Total> {
    (0..len)
        .map(|i| {
            habits
                .iter()
                .fold(Total::default(), |a, h| match h.days[i] {
                    DayState::Skipped => a,
                    DayState::Done => Total {
                        done: a.done + 1,
                        applicable: a.applicable + 1,
                    },
                    _ => Total {
                        applicable: a.applicable + 1,
                        ..a
                    },
                })
        })
        .collect()
}
