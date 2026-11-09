// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Calendar view and `core::agenda`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::agenda;
use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::habits::DayHabit;
use open_productivity_core::tasks::{self, Task, TaskError, TaskId};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::tasks::{State, ui_id};
use crate::{
    AppWindow, CalendarDay, CalendarHabit, CalendarMode, CalendarTask, Date, Strings, WeekBlock,
    WeekColumn,
};

fn row(t: &Task) -> Result<CalendarTask, String> {
    Ok(CalendarTask {
        id: ui_id(t.id.0)?,
        title: t.timed_title().into(),
        done: t.done,
    })
}

fn rows(tasks: Vec<Task>) -> Result<ModelRc<CalendarTask>, String> {
    let rows = tasks.iter().map(row).collect::<Result<Vec<_>, String>>()?;
    Ok(ModelRc::new(VecModel::from(rows)))
}

fn habit_rows(habits: Vec<DayHabit>) -> Result<ModelRc<CalendarHabit>, String> {
    let rows = habits
        .into_iter()
        .map(|h| {
            Ok(CalendarHabit {
                id: ui_id(h.id.0)?,
                name: h.name.into(),
                done: h.done,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ModelRc::new(VecModel::from(rows)))
}

fn ui_date(d: NaiveDate) -> Date {
    Date {
        year: d.year(),
        month: d.month() as i32,
        day: d.day() as i32,
    }
}

fn week_columns(days: Vec<agenda::WeekDay>) -> Result<ModelRc<WeekColumn>, String> {
    let cols = days
        .into_iter()
        .map(|d| {
            let blocks = d
                .blocks
                .iter()
                .map(|b| {
                    Ok(WeekBlock {
                        task: row(&b.task)?,
                        start: b.start as i32,
                        len: b.len as i32,
                        lane: b.lane as i32,
                        lanes: b.lanes as i32,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(WeekColumn {
                date: ui_date(d.date),
                today: d.today,
                untimed: rows(d.untimed)?,
                habits: habit_rows(d.habits)?,
                blocks: ModelRc::new(VecModel::from(blocks)),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ModelRc::new(VecModel::from(cols)))
}

fn now_minutes() -> i32 {
    agenda::minutes_of(Local::now().time()) as i32
}

fn load(
    w: &AppWindow,
    conn: &Connection,
    anchor: NaiveDate,
    mode: CalendarMode,
) -> Result<(), String> {
    let today = Local::now().date_naive();
    let backlog = agenda::backlog(conn, today).map_err(|e| e.to_string())?;
    if mode == CalendarMode::Week {
        let days = agenda::week(conn, anchor, today).map_err(|e| e.to_string())?;
        w.set_calendar_week(week_columns(days)?);
    } else {
        let weeks = agenda::month(conn, first_of(anchor), today).map_err(|e| e.to_string())?;
        let weeks = weeks
            .into_iter()
            .map(|week| {
                let days = week
                    .into_iter()
                    .map(|d| {
                        Ok(CalendarDay {
                            date: ui_date(d.date),
                            day: d.date.day() as i32,
                            in_month: d.in_month,
                            today: d.today,
                            habits_done: d.habits_done() as i32,
                            habits_total: d.habits.len() as i32,
                            tasks: rows(d.tasks)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(ModelRc::new(VecModel::from(days)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        w.set_calendar_weeks(ModelRc::new(VecModel::from(weeks)));
    }
    w.set_calendar_mode(mode);
    w.set_calendar_month(Date {
        day: 1,
        ..ui_date(first_of(anchor))
    });
    w.set_calendar_now(now_minutes());
    w.set_calendar_overdue(rows(backlog.overdue)?);
    w.set_calendar_undated(rows(backlog.undated)?);
    Ok(())
}

/// Registers the navigation and makes every reload of the tasks refresh the calendar too. The
/// returned timer moves the line of the current time and must be kept alive.
pub fn init(window: &AppWindow, state: &Rc<State>) -> Timer {
    // The month shown is the one holding `anchor`; the week shown, the one holding it too.
    let anchor = Rc::new(Cell::new(Local::now().date_naive()));
    let mode = Rc::new(Cell::new(CalendarMode::Month));
    let reload = Rc::new({
        let (conn, weak) = (state.conn.clone(), window.as_weak());
        let (anchor, mode) = (anchor.clone(), mode.clone());
        move || {
            let Some(w) = weak.upgrade() else { return };
            let msg = match load(&w, &conn, anchor.get(), mode.get()) {
                Ok(()) => String::new(),
                Err(e) => w
                    .global::<Strings>()
                    .invoke_calendar_failed(e.into())
                    .into(),
            };
            w.set_calendar_error(msg.into());
        }
    });
    reload();
    let _ = state.on_reload.set(Box::new({
        let reload = reload.clone();
        move || reload()
    }));

    // Habits are marked or created in their own view; the calendar only reads them.
    window.on_refresh_calendar({
        let reload = reload.clone();
        move || reload()
    });

    window.on_create_task_on({
        let (state, weak) = (state.clone(), window.as_weak());
        move |title, d: Date| {
            let Some(w) = weak.upgrade() else {
                return Default::default();
            };
            let strings = w.global::<Strings>();
            let Some(due) = NaiveDate::from_ymd_opt(d.year, d.month as u32, d.day as u32) else {
                return strings
                    .invoke_task_failed(format!("{}-{}-{}", d.year, d.month, d.day).into());
            };
            match tasks::create_on(&state.conn, &title, due) {
                Err(TaskError::EmptyTitle) => strings.get_task_title_empty(),
                Err(e) => strings.invoke_task_failed(e.to_string().into()),
                Ok(_) => {
                    // Failures of the reload show in the tasks error line, as everywhere.
                    if let Err(e) = state.reload() {
                        w.set_tasks_error(strings.invoke_task_failed(e.into()));
                    }
                    Default::default()
                }
            }
        }
    });

    // One drag-and-drop: applies `op` to the task and the dropped day, then reloads.
    let reschedule = {
        let (state, weak) = (state.clone(), window.as_weak());
        move |id: i32,
              d: Date,
              op: &dyn Fn(&Connection, TaskId, NaiveDate) -> Result<(), TaskError>| {
            let Some(w) = weak.upgrade() else { return };
            let failed = |e: String| w.global::<Strings>().invoke_task_failed(e.into());
            let res = match NaiveDate::from_ymd_opt(d.year, d.month as u32, d.day as u32) {
                Some(due) => op(&state.conn, TaskId(id.into()), due).map_err(|e| e.to_string()),
                None => Err(format!("{}-{}-{}", d.year, d.month, d.day)),
            };
            // Reload even on failure so the grid shows what is really stored; it also clears
            // the error line, so the message is set after.
            let reloaded = state.reload();
            w.set_calendar_error(match res.and(reloaded) {
                Ok(()) => Default::default(),
                Err(e) => failed(e),
            });
        }
    };
    let reschedule = Rc::new(reschedule);
    window.on_reschedule_task({
        let reschedule = reschedule.clone();
        move |id, d| reschedule(id, d, &|c, id, due| tasks::set_due_date(c, id, Some(due)))
    });
    window.on_reschedule_task_untimed({
        let reschedule = reschedule.clone();
        move |id, d| reschedule(id, d, &agenda::reschedule_untimed)
    });
    window.on_reschedule_task_at(move |id, d, minutes| {
        let start = agenda::snap_to_slot(minutes.max(0) as u32);
        reschedule(id, d, &|c, id, due| {
            agenda::reschedule_at(c, id, due, start)
        })
    });
    window.on_calendar_slot(|m| agenda::minutes_of(agenda::snap_to_slot(m.max(0) as u32)) as i32);

    window.on_calendar_set_mode({
        let (anchor, mode, reload) = (anchor.clone(), mode.clone(), reload.clone());
        move |m| {
            let today = Local::now().date_naive();
            // Into the week of today when the month shown holds it.
            if m == CalendarMode::Week && first_of(anchor.get()) == first_of(today) {
                anchor.set(today);
            }
            mode.set(m);
            reload();
        }
    });

    // 0 goes back to today; otherwise moves that many months (or weeks).
    window.on_calendar_shift({
        let weak = window.as_weak();
        move |n| {
            let today = Local::now().date_naive();
            let cur = anchor.get();
            let target = if n == 0 {
                Some(today)
            } else if mode.get() == CalendarMode::Week {
                calendar::shift(cur, 7 * i64::from(n))
            } else {
                calendar::month_grid(cur.year(), cur.month(), n, today).map(|g| g.first)
            };
            match (target, weak.upgrade()) {
                (Some(date), _) => {
                    anchor.set(date);
                    reload();
                }
                (None, Some(w)) => {
                    let msg = w
                        .global::<Strings>()
                        .invoke_calendar_failed(cur.to_string().into());
                    w.set_calendar_error(msg);
                }
                (None, None) => {}
            }
        }
    });

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), {
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.set_calendar_now(now_minutes());
            }
        }
    });
    timer
}

fn first_of(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}
