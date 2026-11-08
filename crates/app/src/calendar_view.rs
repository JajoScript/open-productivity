// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Calendar view and `core::agenda`.

use std::cell::Cell;
use std::rc::Rc;

use open_productivity_core::agenda;
use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::tasks::{self, Task, TaskError, TaskId};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::tasks::{State, ui_id};
use crate::{AppWindow, CalendarDay, CalendarTask, Date, Strings};

fn rows(tasks: Vec<Task>) -> Result<ModelRc<CalendarTask>, String> {
    let rows = tasks
        .into_iter()
        .map(|t| {
            Ok(CalendarTask {
                id: ui_id(t.id.0)?,
                title: t.timed_title().into(),
                done: t.done,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ModelRc::new(VecModel::from(rows)))
}

fn load(w: &AppWindow, conn: &Connection, first: NaiveDate) -> Result<(), String> {
    let today = Local::now().date_naive();
    let weeks = agenda::month(conn, first, today).map_err(|e| e.to_string())?;
    let backlog = agenda::backlog(conn, today).map_err(|e| e.to_string())?;
    let weeks = weeks
        .into_iter()
        .map(|week| {
            let days = week
                .into_iter()
                .map(|d| {
                    Ok(CalendarDay {
                        date: Date {
                            year: d.date.year(),
                            month: d.date.month() as i32,
                            day: d.date.day() as i32,
                        },
                        day: d.date.day() as i32,
                        in_month: d.in_month,
                        today: d.today,
                        tasks: rows(d.tasks)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ModelRc::new(VecModel::from(days)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    w.set_calendar_month(Date {
        year: first.year(),
        month: first.month() as i32,
        day: 1,
    });
    w.set_calendar_weeks(ModelRc::new(VecModel::from(weeks)));
    w.set_calendar_overdue(rows(backlog.overdue)?);
    w.set_calendar_undated(rows(backlog.undated)?);
    Ok(())
}

/// Registers the navigation and makes every reload of the tasks refresh the calendar too.
pub fn init(window: &AppWindow, state: &Rc<State>) {
    let shown = Rc::new(Cell::new(first_of(Local::now().date_naive())));
    let reload = Rc::new({
        let (conn, weak, shown) = (state.conn.clone(), window.as_weak(), shown.clone());
        move || {
            let Some(w) = weak.upgrade() else { return };
            let msg = match load(&w, &conn, shown.get()) {
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

    window.on_reschedule_task({
        let (state, weak) = (state.clone(), window.as_weak());
        move |id, d: Date| {
            let Some(w) = weak.upgrade() else { return };
            let failed = |e: String| w.global::<Strings>().invoke_task_failed(e.into());
            let res = match NaiveDate::from_ymd_opt(d.year, d.month as u32, d.day as u32) {
                Some(due) => tasks::set_due_date(&state.conn, TaskId(id.into()), Some(due))
                    .map_err(|e| e.to_string()),
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
    });

    // 0 goes back to the current month; otherwise moves that many months.
    window.on_calendar_shift({
        let weak = window.as_weak();
        move |months| {
            let today = Local::now().date_naive();
            let target = if months == 0 {
                Some(first_of(today))
            } else {
                let cur = shown.get();
                calendar::month_grid(cur.year(), cur.month(), months, today).map(|g| g.first)
            };
            match (target, weak.upgrade()) {
                (Some(first), _) => {
                    shown.set(first);
                    reload();
                }
                (None, Some(w)) => {
                    let e = shown.get().to_string();
                    let msg = w.global::<Strings>().invoke_calendar_failed(e.into());
                    w.set_calendar_error(msg);
                }
                (None, None) => {}
            }
        }
    });
}

fn first_of(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}
