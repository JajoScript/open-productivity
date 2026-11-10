// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Week tab and `core::tasks::week`.

use std::rc::Rc;

use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::tasks;
use slint::{ModelRc, VecModel};

use crate::tasks::{State, task_row};
use crate::{AppWindow, WeekDay};

impl State {
    pub fn reload_week(&self, today: NaiveDate) -> Result<(), String> {
        let monday = self.monday.get();
        let days = tasks::week(&self.conn, monday).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(days.len());
        for (i, list) in (0..).zip(days) {
            let date = calendar::shift(monday, i).ok_or_else(|| monday.to_string())?;
            let rows = list
                .into_iter()
                .map(|t| task_row(t, today, false))
                .collect::<Result<Vec<_>, _>>()?;
            out.push(WeekDay {
                day: date.day() as i32,
                month: date.month() as i32,
                weekday: i as i32,
                today: date == today,
                tasks: ModelRc::new(VecModel::from(rows)),
            });
        }
        self.week.set_vec(out);
        Ok(())
    }
}

pub fn init(
    window: &AppWindow,
    state: &Rc<State>,
    finish: &Rc<impl Fn(Result<(), String>) + 'static>,
) {
    // 0 goes back to the current week; otherwise moves that many weeks.
    window.on_week_shift({
        let (state, finish) = (state.clone(), finish.clone());
        move |weeks| {
            let today = Local::now().date_naive();
            let from = state.monday.get();
            let monday = calendar::navigate(from, calendar::Period::Week, weeks, today)
                .map(calendar::week_start);
            match monday {
                Some(m) => {
                    state.monday.set(m);
                    finish(state.reload());
                }
                None => finish(Err(state.monday.get().to_string())),
            }
        }
    });
}
