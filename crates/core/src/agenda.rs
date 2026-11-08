// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Calendar module: the tasks of a month grid, placed by due date.

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use rusqlite::Connection;

use crate::calendar;
use crate::tasks::{SELECT_TASKS, Task, TaskError, TaskId, View, query};

/// One cell of the month grid.
#[derive(Debug)]
pub struct Day {
    pub date: NaiveDate,
    /// False for the filler days of the neighbouring months.
    pub in_month: bool,
    pub today: bool,
    /// Pending first, then done; each with a time first (by time), then oldest created first.
    pub tasks: Vec<Task>,
}

/// The 6 weeks (Monday first) of the month that starts on `first`, filler days included, with
/// the tasks due on each day. One range query; tasks without a due date never show up.
pub fn month(
    conn: &Connection,
    first: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<Vec<Day>>, TaskError> {
    let bad = || TaskError::BadDate(first.to_string());
    let grid = calendar::month_grid(first.year(), first.month(), 0, today).ok_or_else(bad)?;
    let start = calendar::week_start(grid.first);
    let end = calendar::shift(start, 41).ok_or_else(bad)?;
    let mut days = (0..42)
        .map(|i| {
            calendar::shift(start, i as i64).map(|date| Day {
                date,
                in_month: grid.cells[i].is_some(),
                today: date == today,
                tasks: Vec::new(),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)?;
    let sql = format!(
        "{SELECT_TASKS} WHERE t.due_date BETWEEN ?1 AND ?2 \
         ORDER BY t.due_date, t.done_at IS NOT NULL, t.due_time IS NULL, t.due_time, \
         t.created_at, t.id"
    );
    for t in query(conn, &sql, (start.to_string(), end.to_string()))? {
        // In range by the query, so the offset is 0..=41.
        let i = t.due.map_or(0, |d| (d - start).num_days()) as usize;
        days[i].tasks.push(t);
    }
    let mut weeks = Vec::with_capacity(6);
    while !days.is_empty() {
        weeks.push(days.drain(..7).collect());
    }
    Ok(weeks)
}

const DAY_END: u32 = 24 * 60;
/// How long a task with a start and no end is drawn, never stored.
const DEFAULT_LEN: u32 = 60;
const SLOT: u32 = 30;

/// Minutes since midnight of a time of day.
pub fn minutes_of(t: NaiveTime) -> u32 {
    t.hour() * 60 + t.minute()
}

/// A drop position (minutes since midnight) snapped to the nearest 30 minutes, the last slot
/// being 23:30. Past the end of the day it is still 23:30.
pub fn snap_to_slot(minutes: u32) -> NaiveTime {
    let m = ((minutes + SLOT / 2) / SLOT * SLOT).min(DAY_END - SLOT);
    NaiveTime::from_hms_opt(m / 60, m % 60, 0).unwrap_or(NaiveTime::MIN)
}

/// Lane of each span (start, end in minutes) and how many lanes its cluster needs. Spans that
/// only touch do not overlap; a cluster is transitive (A-B and B-C overlapping share lanes
/// even when A and C do not). Results come in the order of the input.
pub fn assign_lanes(spans: &[(u32, u32)]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| spans[i]);
    let mut out = vec![(0, 1); spans.len()];
    // End of each lane of the open cluster, its members, and how far it reaches.
    let (mut ends, mut members, mut reach): (Vec<u32>, Vec<usize>, u32) = (vec![], vec![], 0);
    let close = |ends: &mut Vec<u32>, members: &mut Vec<usize>, out: &mut Vec<(usize, usize)>| {
        for &m in members.iter() {
            out[m].1 = ends.len();
        }
        ends.clear();
        members.clear();
    };
    for i in order {
        let (start, end) = spans[i];
        if start >= reach {
            close(&mut ends, &mut members, &mut out);
        }
        let lane = ends.iter().position(|&e| e <= start).unwrap_or(ends.len());
        if lane == ends.len() {
            ends.push(end);
        } else {
            ends[lane] = end;
        }
        reach = if members.is_empty() {
            end
        } else {
            reach.max(end)
        };
        members.push(i);
        out[i].0 = lane;
    }
    close(&mut ends, &mut members, &mut out);
    out
}

/// A task with a start time, placed in its day.
#[derive(Debug)]
pub struct Block {
    pub task: Task,
    /// Minutes since midnight where it starts, and how long it is drawn (to 24:00 at most).
    pub start: u32,
    pub len: u32,
    pub lane: usize,
    pub lanes: usize,
}

/// One column of the week view.
#[derive(Debug)]
pub struct WeekDay {
    pub date: NaiveDate,
    pub today: bool,
    /// Tasks of the day without a time: pending first, then done.
    pub untimed: Vec<Task>,
    /// Tasks with a time, by start.
    pub blocks: Vec<Block>,
}

/// The 7 days (Monday first) of the week that holds `date`, with the tasks due on each. One
/// range query.
pub fn week(
    conn: &Connection,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<WeekDay>, TaskError> {
    let bad = || TaskError::BadDate(date.to_string());
    let start = calendar::week_start(date);
    let end = calendar::shift(start, 6).ok_or_else(bad)?;
    let mut days = (0..7)
        .map(|i| {
            calendar::shift(start, i).map(|date| WeekDay {
                date,
                today: date == today,
                untimed: Vec::new(),
                blocks: Vec::new(),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)?;
    let sql = format!(
        "{SELECT_TASKS} WHERE t.due_date BETWEEN ?1 AND ?2 \
         ORDER BY t.due_date, t.done_at IS NOT NULL, t.due_time IS NULL, t.due_time, \
         t.created_at, t.id"
    );
    for t in query(conn, &sql, (start.to_string(), end.to_string()))? {
        // In range by the query, so the offset is 0..=6.
        let i = t.due.map_or(0, |d| (d - start).num_days()) as usize;
        match t.start {
            None => days[i].untimed.push(t),
            Some(s) => {
                let from = minutes_of(s);
                let len = t
                    .end
                    .map_or(DEFAULT_LEN, |e| minutes_of(e).saturating_sub(from));
                days[i].blocks.push(Block {
                    start: from,
                    len: len.min(DAY_END - from).max(1),
                    task: t,
                    lane: 0,
                    lanes: 1,
                });
            }
        }
    }
    for day in &mut days {
        let spans: Vec<_> = day
            .blocks
            .iter()
            .map(|b| (b.start, b.start + b.len))
            .collect();
        for (b, (lane, lanes)) in day.blocks.iter_mut().zip(assign_lanes(&spans)) {
            (b.lane, b.lanes) = (lane, lanes);
        }
    }
    Ok(days)
}

/// Moves a task to `date` at `start`, keeping its length when it has an end. A task that would
/// run past 23:59 starts earlier so that it ends at 23:59. One transaction.
pub fn reschedule_at(
    conn: &Connection,
    id: TaskId,
    date: NaiveDate,
    start: NaiveTime,
) -> Result<(), TaskError> {
    let tx = conn.unchecked_transaction()?;
    let old: (Option<String>, Option<String>) = tx
        .query_row(
            "SELECT due_time, end_time FROM task WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => TaskError::NotFound,
            e => e.into(),
        })?;
    let at = |s: &str| {
        NaiveTime::parse_from_str(s, "%H:%M")
            .map(minutes_of)
            .map_err(|_| TaskError::BadTime(s.to_string()))
    };
    let mut from = minutes_of(start);
    let mut end = None;
    if let (Some(s), Some(e)) = (&old.0, &old.1) {
        let len = at(e)?.saturating_sub(at(s)?);
        from = from.min(DAY_END - 1 - len);
        end = Some(from + len);
    }
    let text = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
    tx.execute(
        "UPDATE task SET due_date = ?1, due_time = ?2, end_time = ?3 WHERE id = ?4",
        (date.to_string(), text(from), end.map(text), id.0),
    )?;
    tx.commit()?;
    Ok(())
}

/// Moves a task to `date` and drops its times, so it sits in the day's "no time" strip.
pub fn reschedule_untimed(conn: &Connection, id: TaskId, date: NaiveDate) -> Result<(), TaskError> {
    match conn.execute(
        "UPDATE task SET due_date = ?1, due_time = NULL, end_time = NULL WHERE id = ?2",
        (date.to_string(), id.0),
    )? {
        0 => Err(TaskError::NotFound),
        _ => Ok(()),
    }
}

/// The calendar's side panel: pending tasks that have no place in the month grid.
#[derive(Debug)]
pub struct Backlog {
    /// Due before `today`, by date then creation.
    pub overdue: Vec<Task>,
    /// Without a due date (in a list or not), by creation.
    pub undated: Vec<Task>,
}

pub fn backlog(conn: &Connection, today: NaiveDate) -> Result<Backlog, TaskError> {
    let overdue = format!(
        "WITH p(today) AS (SELECT ?1) {SELECT_TASKS} CROSS JOIN p \
         WHERE ({}) AND t.done_at IS NULL ORDER BY t.due_date, t.created_at, t.id",
        View::Overdue.rule()
    );
    let undated = format!(
        "{SELECT_TASKS} WHERE t.due_date IS NULL AND t.done_at IS NULL \
         ORDER BY t.created_at, t.id"
    );
    Ok(Backlog {
        overdue: query(conn, &overdue, [today.to_string()])?,
        undated: query(conn, &undated, [])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::tasks::{self, View};

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn setup(name: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("op-agenda-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db::open(&dir.join("test.db")).unwrap();
        (dir, conn)
    }

    fn add(conn: &Connection, title: &str, due: Option<&str>) -> tasks::TaskId {
        let id = tasks::create(conn, title, View::All, d("2026-11-08")).unwrap();
        tasks::set_due_date(conn, id, due.map(d)).unwrap();
        id
    }

    fn day(weeks: &[Vec<Day>], date: &str) -> usize {
        weeks
            .iter()
            .flatten()
            .position(|x| x.date == d(date))
            .unwrap()
    }

    fn titles(weeks: &[Vec<Day>], date: &str) -> Vec<String> {
        let all: Vec<&Day> = weeks.iter().flatten().collect();
        all[day(weeks, date)]
            .tasks
            .iter()
            .map(|t| t.title.clone())
            .collect()
    }

    #[test]
    fn tasks_fall_on_their_day_and_fillers_too() {
        let (dir, conn) = setup("fall");
        add(&conn, "in", Some("2026-11-15"));
        add(&conn, "lead", Some("2026-10-30"));
        add(&conn, "trail", Some("2026-12-06"));
        add(&conn, "outside", Some("2026-12-07"));
        add(&conn, "undated", None);
        // November 2026 starts on a Sunday: the grid runs 2026-10-26 to 2026-12-06.
        let w = month(&conn, d("2026-11-01"), d("2026-11-08")).unwrap();
        assert_eq!((w.len(), w[0].len()), (6, 7));
        assert_eq!(titles(&w, "2026-11-15"), ["in"]);
        assert_eq!(titles(&w, "2026-10-30"), ["lead"]);
        assert_eq!(titles(&w, "2026-12-06"), ["trail"]);
        let n: usize = w.iter().flatten().map(|x| x.tasks.len()).sum();
        assert_eq!(n, 3);
        let all: Vec<&Day> = w.iter().flatten().collect();
        assert!(all[day(&w, "2026-11-01")].in_month);
        assert!(!all[day(&w, "2026-12-01")].in_month);
        assert!(all[day(&w, "2026-11-08")].today);
        assert_eq!(all.iter().filter(|x| x.today).count(), 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn created_on_a_day_shows_in_that_cell() {
        let (dir, conn) = setup("created-on");
        tasks::create_on(&conn, "past", d("2026-10-28")).unwrap();
        let w = month(&conn, d("2026-11-01"), d("2026-11-08")).unwrap();
        assert_eq!(titles(&w, "2026-10-28"), ["past"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_before_done_then_creation() {
        let (dir, conn) = setup("order");
        let a = add(&conn, "a", Some("2026-11-10"));
        add(&conn, "b", Some("2026-11-10"));
        add(&conn, "c", Some("2026-11-10"));
        tasks::set_done(&conn, a, true).unwrap();
        let w = month(&conn, d("2026-11-01"), d("2026-11-08")).unwrap();
        assert_eq!(titles(&w, "2026-11-10"), ["b", "c", "a"]);
        assert!(w.iter().flatten().any(|x| x.tasks.iter().any(|t| t.done)));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timed_tasks_come_first_within_pending_and_done() {
        let (dir, conn) = setup("timed");
        let at = |title: &str, time: Option<&str>| {
            let id = add(&conn, title, Some("2026-11-10"));
            if let Some(t) = time {
                tasks::set_time(&conn, id, Some(t.parse().unwrap()), None).unwrap();
            }
            id
        };
        at("plain1", None);
        at("late", Some("14:00"));
        at("plain2", None);
        at("early", Some("09:30"));
        let done_plain = at("done-plain", None);
        let done_timed = at("done-timed", Some("08:00"));
        tasks::set_done(&conn, done_plain, true).unwrap();
        tasks::set_done(&conn, done_timed, true).unwrap();
        let w = month(&conn, d("2026-11-01"), d("2026-11-08")).unwrap();
        assert_eq!(
            titles(&w, "2026-11-10"),
            [
                "early",
                "late",
                "plain1",
                "plain2",
                "done-timed",
                "done-plain"
            ]
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn names(ts: &[Task]) -> Vec<&str> {
        ts.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn backlog_sections() {
        let (dir, conn) = setup("backlog");
        let today = d("2026-11-08");
        add(&conn, "late2", Some("2026-11-05"));
        add(&conn, "late1", Some("2026-11-01"));
        add(&conn, "today", Some("2026-11-08"));
        add(&conn, "future", Some("2026-11-09"));
        add(&conn, "none1", None);
        let listed = add(&conn, "none2", None);
        let list = crate::lists::create(&conn, "L").unwrap();
        tasks::set_list(&conn, listed, Some(list)).unwrap();
        let old = add(&conn, "old-done", Some("2026-10-01"));
        tasks::set_done(&conn, old, true).unwrap();
        let gone = add(&conn, "none-done", None);
        tasks::set_done(&conn, gone, true).unwrap();
        let b = backlog(&conn, today).unwrap();
        assert_eq!(names(&b.overdue), ["late1", "late2"]);
        assert_eq!(names(&b.undated), ["none1", "none2"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rescheduling_an_overdue_task_moves_it_to_the_day() {
        let (dir, conn) = setup("resched");
        let today = d("2026-11-08");
        let id = add(&conn, "late", Some("2026-11-01"));
        let undated = add(&conn, "none", None);
        tasks::set_due_date(&conn, id, Some(d("2026-11-20"))).unwrap();
        tasks::set_due_date(&conn, undated, Some(d("2026-11-21"))).unwrap();
        let b = backlog(&conn, today).unwrap();
        assert!(b.overdue.is_empty() && b.undated.is_empty());
        let w = month(&conn, d("2026-11-01"), today).unwrap();
        assert_eq!(titles(&w, "2026-11-20"), ["late"]);
        assert_eq!(titles(&w, "2026-11-21"), ["none"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn t(s: &str) -> NaiveTime {
        s.parse().unwrap()
    }

    type Stored = (String, Option<String>, Option<String>);

    fn stored(conn: &Connection, id: tasks::TaskId) -> Stored {
        conn.query_row(
            "SELECT due_date, due_time, end_time FROM task WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn row(date: &str, s: Option<&str>, e: Option<&str>) -> Stored {
        (date.into(), s.map(Into::into), e.map(Into::into))
    }

    #[test]
    fn snap_rounds_to_half_hours_within_the_day() {
        assert_eq!(snap_to_slot(0), t("00:00"));
        assert_eq!(snap_to_slot(14), t("00:00"));
        assert_eq!(snap_to_slot(15), t("00:30"));
        assert_eq!(snap_to_slot(44), t("00:30"));
        assert_eq!(snap_to_slot(45), t("01:00"));
        assert_eq!(snap_to_slot(7 * 60 + 10), t("07:00"));
        assert_eq!(snap_to_slot(23 * 60 + 14), t("23:00"));
        assert_eq!(snap_to_slot(23 * 60 + 15), t("23:30"));
        assert_eq!(snap_to_slot(1439), t("23:30"));
        assert_eq!(snap_to_slot(5000), t("23:30"));
    }

    #[test]
    fn lanes() {
        assert!(assign_lanes(&[]).is_empty());
        // No overlap: each alone.
        assert_eq!(assign_lanes(&[(60, 120), (180, 240)]), [(0, 1), (0, 1)]);
        // Partial overlap.
        assert_eq!(assign_lanes(&[(60, 120), (90, 150)]), [(0, 2), (1, 2)]);
        // Contained, given out of order.
        assert_eq!(assign_lanes(&[(70, 80), (60, 180)]), [(1, 2), (0, 2)]);
        // Chain A-B, B-C: A and C share lane 0 in a 2-lane cluster.
        assert_eq!(
            assign_lanes(&[(60, 120), (100, 160), (150, 200)]),
            [(0, 2), (1, 2), (0, 2)]
        );
        // Three at once.
        assert_eq!(
            assign_lanes(&[(60, 180), (70, 170), (80, 160)]),
            [(0, 3), (1, 3), (2, 3)]
        );
        // Touching is not overlapping.
        assert_eq!(assign_lanes(&[(60, 120), (120, 180)]), [(0, 1), (0, 1)]);
        // A later, separate cluster does not inherit the lane count.
        assert_eq!(
            assign_lanes(&[(0, 60), (30, 90), (120, 180)]),
            [(0, 2), (1, 2), (0, 1)]
        );
    }

    #[test]
    fn week_places_untimed_and_blocks_in_lanes() {
        let (dir, conn) = setup("week");
        let today = d("2026-11-08");
        let timed = |title: &str, day: &str, s: &str, e: Option<&str>| {
            let id = add(&conn, title, Some(day));
            tasks::set_time(&conn, id, Some(t(s)), e.map(t)).unwrap();
        };
        // Week of Monday 2026-11-02 to Sunday 2026-11-08.
        timed("a", "2026-11-04", "09:00", Some("10:00"));
        timed("b", "2026-11-04", "09:30", None);
        timed("c", "2026-11-04", "10:00", None);
        timed("late", "2026-11-04", "23:30", None);
        add(&conn, "plain", Some("2026-11-04"));
        add(&conn, "out", Some("2026-11-09"));
        add(&conn, "undated", None);
        let w = week(&conn, d("2026-11-06"), today).unwrap();
        assert_eq!(w.len(), 7);
        assert_eq!((w[0].date, w[6].date), (d("2026-11-02"), d("2026-11-08")));
        assert!(w[6].today && !w[0].today);
        let wed = &w[2];
        assert_eq!(wed.untimed.len(), 1);
        let got: Vec<_> = wed
            .blocks
            .iter()
            .map(|b| (b.task.title.as_str(), b.start, b.len, b.lane, b.lanes))
            .collect();
        // b has no end: one hour, 09:30-10:30; c starts when a ends and joins b's cluster.
        assert_eq!(
            got,
            [
                ("a", 540, 60, 0, 2),
                ("b", 570, 60, 1, 2),
                ("c", 600, 60, 0, 2),
                ("late", 1410, 30, 0, 1)
            ]
        );
        let n: usize = w.iter().map(|x| x.blocks.len() + x.untimed.len()).sum();
        assert_eq!(n, 5);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reschedule_at_keeps_duration_or_pulls_back_from_midnight() {
        let (dir, conn) = setup("resched-at");
        let id = add(&conn, "x", Some("2026-11-02"));
        tasks::set_time(&conn, id, Some(t("14:15")), Some(t("16:00"))).unwrap();
        reschedule_at(&conn, id, d("2026-11-05"), t("09:00")).unwrap();
        assert_eq!(
            stored(&conn, id),
            row("2026-11-05", Some("09:00"), Some("10:45"))
        );
        // Would end at 00:15 of the next day: ends at 23:59 instead.
        reschedule_at(&conn, id, d("2026-11-05"), t("22:30")).unwrap();
        assert_eq!(
            stored(&conn, id),
            row("2026-11-05", Some("22:14"), Some("23:59"))
        );
        // Without an end, and without a date before, it is just placed.
        let open = add(&conn, "y", None);
        reschedule_at(&conn, open, d("2026-11-06"), t("23:30")).unwrap();
        assert_eq!(stored(&conn, open), row("2026-11-06", Some("23:30"), None));
        assert!(matches!(
            reschedule_at(&conn, tasks::TaskId(999), d("2026-11-06"), t("08:00")),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reschedule_untimed_moves_the_day_and_drops_the_times() {
        let (dir, conn) = setup("resched-untimed");
        let id = add(&conn, "x", Some("2026-11-02"));
        tasks::set_time(&conn, id, Some(t("14:15")), Some(t("16:00"))).unwrap();
        reschedule_untimed(&conn, id, d("2026-11-04")).unwrap();
        assert_eq!(stored(&conn, id), row("2026-11-04", None, None));
        // The same day, to drop the times only.
        tasks::set_time(&conn, id, Some(t("08:00")), None).unwrap();
        reschedule_untimed(&conn, id, d("2026-11-04")).unwrap();
        assert_eq!(stored(&conn, id), row("2026-11-04", None, None));
        assert!(matches!(
            reschedule_untimed(&conn, tasks::TaskId(999), d("2026-11-04")),
            Err(TaskError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn month_shapes() {
        let (dir, conn) = setup("shapes");
        let today = d("2026-11-08");
        let count = |first: &str| {
            let w = month(&conn, d(first), today).unwrap();
            assert_eq!(w.len(), 6);
            let inside: Vec<&Day> = w.iter().flatten().filter(|x| x.in_month).collect();
            (inside.len(), w[0][0].date)
        };
        // Leap February, starting on a Tuesday.
        assert_eq!(count("2028-02-01"), (29, d("2028-01-31")));
        // February 2027 starts on a Monday: no leading filler.
        assert_eq!(count("2027-02-01"), (28, d("2027-02-01")));
        // November 2026 starts on a Sunday: six leading fillers.
        assert_eq!(count("2026-11-01"), (30, d("2026-10-26")));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
