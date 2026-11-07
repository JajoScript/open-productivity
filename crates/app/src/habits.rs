// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Habits view and `core::habits`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::habits::{self, DayState as CoreState, HabitError, HabitId};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::tasks::ui_id;
use crate::{AppWindow, Date, DayState, HabitRow, Strings};

struct State {
    conn: Rc<Connection>,
    rows: Rc<VecModel<HabitRow>>,
    // Day of the last reload, so the timer only rebuilds rows when it changes.
    day: Cell<NaiveDate>,
    // First day of the month shown.
    shown: Cell<NaiveDate>,
    window: slint::Weak<AppWindow>,
}

impl State {
    fn reload(&self) -> Result<(), String> {
        let today = Local::now().date_naive();
        self.day.set(today);
        let m = habits::month(&self.conn, self.shown.get(), today).map_err(|e| e.to_string())?;
        let rows = m
            .habits
            .into_iter()
            .map(|h| {
                Ok(HabitRow {
                    id: ui_id(h.id.0)?,
                    name: h.name.into(),
                    streak: h.stats.current as i32,
                    best: h.stats.best as i32,
                    month_pct: h.stats.month_pct as i32,
                    weeks: blocks(h.days.into_iter().map(day_state).collect()),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if let Some(w) = self.window.upgrade() {
            w.set_habits_month(Date {
                year: m.first.year(),
                month: m.first.month() as i32,
                day: 1,
            });
            w.set_habits_today_day(m.today.map_or(0, |d| d as i32));
            w.set_habits_can_next(m.can_next);
            w.set_habit_day_numbers(blocks((1..=m.days as i32).collect()));
        }
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

/// Splits the days into blocks of seven for the grid.
fn blocks<T: Clone + 'static>(cells: Vec<T>) -> ModelRc<ModelRc<T>> {
    ModelRc::new(VecModel::from(
        cells
            .chunks(7)
            .map(|c| ModelRc::new(VecModel::from(c.to_vec())))
            .collect::<Vec<_>>(),
    ))
}

fn day_state(s: CoreState) -> DayState {
    match s {
        CoreState::Done => DayState::Done,
        CoreState::Open => DayState::Open,
        CoreState::Skipped => DayState::Skipped,
        CoreState::Future => DayState::Future,
    }
}

/// The returned timer must be kept alive for the midnight reload to run.
pub fn init(window: &AppWindow, conn: Rc<Connection>) -> Timer {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        day: Cell::new(Local::now().date_naive()),
        shown: Cell::new(habits::month_start(Local::now().date_naive())),
        window: window.as_weak(),
    });
    window.set_habits(ModelRc::from(state.rows.clone()));
    state.finish(state.reload());

    window.on_create_habit({
        let state = state.clone();
        // Returns the message for the creator; empty closes it.
        move |name, days, start: Date| {
            let Some(w) = state.window.upgrade() else {
                return Default::default();
            };
            let s = w.global::<Strings>();
            let today = Local::now().date_naive();
            let flags: Vec<bool> = days.iter().collect();
            let Ok(flags) = <[bool; 7]>::try_from(flags) else {
                return s.invoke_habit_failed("weekdays".into());
            };
            // A date the picker cannot produce is an error, never "start today".
            let Some(start) =
                NaiveDate::from_ymd_opt(start.year, start.month as u32, start.day as u32)
            else {
                return s.invoke_habit_failed(
                    format!("{}-{}-{}", start.year, start.month, start.day).into(),
                );
            };
            match habits::create(&state.conn, &name, flags, start, today) {
                Err(HabitError::EmptyName) => s.get_habit_empty_name(),
                Err(HabitError::NoWeekdays) => s.get_habit_no_days(),
                Err(HabitError::FutureStart) => s.get_habit_future_start(),
                Err(e) => s.invoke_habit_failed(e.to_string().into()),
                Ok(_) => {
                    state.after(Ok(()));
                    Default::default()
                }
            }
        }
    });

    window.on_set_habit_done({
        let state = state.clone();
        move |id, day, done| {
            let today = Local::now().date_naive();
            let shown = state.shown.get();
            let res = match NaiveDate::from_ymd_opt(shown.year(), shown.month(), day as u32) {
                Some(date) => habits::set_done(&state.conn, HabitId(id.into()), date, today, done),
                None => Err(HabitError::OutOfRange),
            };
            state.after(res)
        }
    });

    // 0 goes back to the current month; otherwise moves that many months.
    window.on_habits_shift({
        let state = state.clone();
        move |months| {
            let today = Local::now().date_naive();
            let target = if months == 0 {
                Ok(habits::month_start(today))
            } else {
                habits::shift_month(state.shown.get(), months, today)
            };
            match target {
                Ok(first) => {
                    state.shown.set(first);
                    state.after(Ok(()));
                }
                Err(e) => state.after(Err(e)),
            }
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
