// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Calendar view and `core::agenda`.

use std::cell::Cell;
use std::rc::Rc;

use open_productivity_core::agenda;
use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::tasks::{State, ui_id};
use crate::{AppWindow, CalendarDay, CalendarTask, Date, Strings};

fn load(w: &AppWindow, conn: &Connection, first: NaiveDate) -> Result<(), String> {
    let today = Local::now().date_naive();
    let weeks = agenda::month(conn, first, today).map_err(|e| e.to_string())?;
    let weeks = weeks
        .into_iter()
        .map(|week| {
            let days = week
                .into_iter()
                .map(|d| {
                    let tasks = d
                        .tasks
                        .into_iter()
                        .map(|t| {
                            Ok(CalendarTask {
                                id: ui_id(t.id.0)?,
                                title: t.title.into(),
                                done: t.done,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    Ok(CalendarDay {
                        day: d.date.day() as i32,
                        in_month: d.in_month,
                        today: d.today,
                        tasks: ModelRc::new(VecModel::from(tasks)),
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
