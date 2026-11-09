// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Habits view and `core::habits`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::habits::{self, DayState as CoreState, HabitError, HabitId, Heat};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::tasks::ui_id;
use crate::{
    AppWindow, ArchivedRow, Date, DayHabit, DayState, HabitRow, HeatCell, HeatLevel, Strings,
    WeekTotal,
};

/// Publishes the habits that apply today for the Today tab of Tasks.
pub fn load_today(w: &AppWindow, conn: &Connection) -> Result<(), String> {
    let today = Local::now().date_naive();
    let list = habits::today(conn, today).map_err(|e| e.to_string())?;
    let done = ui_id(habits::done_count(&list) as i64)?;
    let rows = list
        .into_iter()
        .map(|h| {
            Ok(DayHabit {
                id: ui_id(h.id.0)?,
                name: h.name.into(),
                done: h.done,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    w.set_habits_done(done);
    w.set_day_habits(ModelRc::new(VecModel::from(rows)));
    Ok(())
}

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
        if let Some(w) = self.window.upgrade() {
            load_today(&w, &self.conn)?;
        }
        let archived = habits::archived(&self.conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id, name)| {
                Ok(ArchivedRow {
                    id: ui_id(id.0)?,
                    name: name.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
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
                    weekdays: ModelRc::new(VecModel::from(h.weekdays.to_vec())),
                    start: Date {
                        year: h.start.year(),
                        month: h.start.month() as i32,
                        day: h.start.day() as i32,
                    },
                    weeks: blocks(h.days.into_iter().map(day_state).collect()),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let totals: Vec<WeekTotal> = m
            .week_totals
            .iter()
            .map(|t| WeekTotal {
                done: t.done as i32,
                applicable: t.applicable as i32,
                pct: t.pct().unwrap_or(0) as i32,
            })
            .collect();
        if let Some(w) = self.window.upgrade() {
            w.set_habit_week_totals(ModelRc::new(VecModel::from(totals)));
            w.set_habits_month(Date {
                year: m.first.year(),
                month: m.first.month() as i32,
                day: 1,
            });
            let t = m.month_total;
            w.set_habit_month_total(WeekTotal {
                done: t.done as i32,
                applicable: t.applicable as i32,
                pct: t.pct().unwrap_or(0) as i32,
            });
            // Blank cells pad the first row so the map starts on Monday.
            let cells: Vec<HeatCell> = (0..m.lead)
                .map(|_| HeatCell {
                    today: false,
                    level: HeatLevel::Blank,
                })
                .chain(m.heat.iter().enumerate().map(|(i, h)| HeatCell {
                    today: m.today == Some(i as u32 + 1),
                    level: heat_level(*h),
                }))
                .collect();
            w.set_habit_heat(blocks(cells));
            w.set_habits_today_day(m.today.map_or(0, |d| d as i32));
            w.set_habits_can_next(m.can_next);
            w.set_archived_habits(ModelRc::new(VecModel::from(archived)));
            w.set_habit_day_numbers(blocks((1..=m.days as i32).collect()));
            w.invoke_refresh_calendar();
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

    /// Runs a create or update from the modal's raw days and date; returns the message for it.
    fn save(
        &self,
        days: ModelRc<bool>,
        start: Date,
        op: impl FnOnce(&Connection, [bool; 7], NaiveDate, NaiveDate) -> Result<(), HabitError>,
    ) -> slint::SharedString {
        let Some(w) = self.window.upgrade() else {
            return Default::default();
        };
        let s = w.global::<Strings>();
        let flags: Vec<bool> = days.iter().collect();
        let Ok(flags) = <[bool; 7]>::try_from(flags) else {
            return s.invoke_habit_failed("weekdays".into());
        };
        // A date the picker cannot produce is an error, never "start today".
        let Some(start) = NaiveDate::from_ymd_opt(start.year, start.month as u32, start.day as u32)
        else {
            return s.invoke_habit_failed(
                format!("{}-{}-{}", start.year, start.month, start.day).into(),
            );
        };
        match op(&self.conn, flags, start, Local::now().date_naive()) {
            Err(HabitError::EmptyName) => s.get_habit_empty_name(),
            Err(HabitError::NoWeekdays) => s.get_habit_no_days(),
            Err(HabitError::FutureStart) => s.get_habit_future_start(),
            Err(e) => s.invoke_habit_failed(e.to_string().into()),
            Ok(()) => {
                self.after(Ok(()));
                Default::default()
            }
        }
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

fn heat_level(h: Heat) -> HeatLevel {
    match h {
        Heat::None => HeatLevel::None,
        Heat::Future => HeatLevel::Future,
        Heat::L0 => HeatLevel::L0,
        Heat::L1 => HeatLevel::L1,
        Heat::L2 => HeatLevel::L2,
        Heat::L3 => HeatLevel::L3,
        Heat::L4 => HeatLevel::L4,
    }
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

    // Both return the message for the modal; empty means it worked and the modal closes.
    window.on_create_habit({
        let state = state.clone();
        move |name, days, start| {
            state.save(days, start, |c, d, s, t| {
                habits::create(c, &name, d, s, t).map(|_| ())
            })
        }
    });

    window.on_update_habit({
        let state = state.clone();
        move |id, name, days, start| {
            state.save(days, start, |c, d, s, t| {
                habits::update(c, HabitId(id.into()), &name, d, s, t)
            })
        }
    });

    window.on_archive_habit({
        let state = state.clone();
        move |id| state.after(habits::archive(&state.conn, HabitId(id.into())))
    });

    window.on_restore_habit({
        let state = state.clone();
        move |id| state.after(habits::restore(&state.conn, HabitId(id.into())))
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

    // From the Today tab of Tasks; `after` reloads the month and the Today section.
    window.on_set_day_habit_done({
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
