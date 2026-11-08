// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Calendar module: the tasks of a month grid, placed by due date.

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;

use crate::calendar;
use crate::tasks::{SELECT_TASKS, Task, TaskError, View, query};

/// One cell of the month grid.
#[derive(Debug)]
pub struct Day {
    pub date: NaiveDate,
    /// False for the filler days of the neighbouring months.
    pub in_month: bool,
    pub today: bool,
    /// Pending first, then done; each oldest created first.
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
         ORDER BY t.due_date, t.done_at IS NOT NULL, t.created_at, t.id"
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
