// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Habits view and `core::habits`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::chrono::{Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::habits::{self, HabitError, HabitId};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::tasks::{show_today, ui_id};
use crate::{AppWindow, HabitRow, Strings};

struct State {
    conn: Rc<Connection>,
    rows: Rc<VecModel<HabitRow>>,
    // Day of the last reload, so the timer only rebuilds rows when it changes.
    day: Cell<NaiveDate>,
    window: slint::Weak<AppWindow>,
}

impl State {
    fn reload(&self) -> Result<(), String> {
        let today = Local::now().date_naive();
        self.day.set(today);
        if let Some(w) = self.window.upgrade() {
            show_today(&w, today);
        }
        let rows = habits::for_day(&self.conn, today)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|h| {
                Ok(HabitRow {
                    id: ui_id(h.id.0)?,
                    name: h.name.into(),
                    done: h.done,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.rows.set_vec(rows);
        Ok(())
    }

    fn show_error(&self, msg: String) {
        if let Some(w) = self.window.upgrade() {
            w.set_habits_error(msg.into());
        }
    }

    /// Shows the outcome in the view.
    fn finish(&self, res: Result<(), String>) {
        let msg = match (res, self.window.upgrade()) {
            (Err(e), Some(w)) => w.global::<Strings>().invoke_habit_failed(e.into()).into(),
            _ => String::new(),
        };
        self.show_error(msg);
    }

    /// Reloads even on failure so the view shows what is really stored.
    fn after(&self, res: Result<(), HabitError>) {
        let reloaded = self.reload();
        self.finish(res.map_err(|e| e.to_string()).and(reloaded));
    }
}

/// The returned timer must be kept alive for the midnight reload to run.
pub fn init(window: &AppWindow, conn: Rc<Connection>) -> Timer {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        day: Cell::new(Local::now().date_naive()),
        window: window.as_weak(),
    });
    window.set_habits(ModelRc::from(state.rows.clone()));
    state.finish(state.reload());

    window.on_create_habit({
        let state = state.clone();
        move |name| match habits::create(&state.conn, &name, Local::now().date_naive()) {
            Err(HabitError::EmptyName) => {
                let msg = state.window.upgrade().map_or_else(String::new, |w| {
                    w.global::<Strings>().get_habit_empty_name().into()
                });
                state.show_error(msg);
            }
            res => state.after(res.map(|_| ())),
        }
    });

    window.on_set_habit_done({
        let state = state.clone();
        move |id, done| {
            let today = Local::now().date_naive();
            state.after(habits::set_done(
                &state.conn,
                HabitId(id.into()),
                today,
                today,
                done,
            ))
        }
    });

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        // set_vec closes open popups, so reload only when the day rolls over.
        if Local::now().date_naive() != state.day.get() {
            state.finish(state.reload())
        }
    });
    timer
}
