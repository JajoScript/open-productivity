// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the date picker and `core::calendar`.

use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::dates::{core_date, ui_date};
use crate::{AppWindow, Calendar, MonthView};

pub fn init(window: &AppWindow) {
    let cal = window.global::<Calendar>();
    cal.on_month(|year, month, delta| {
        let today = Local::now().date_naive();
        // An invalid request leaves the picker on an empty grid instead of panicking.
        let Some(g) = u32::try_from(month)
            .ok()
            .and_then(|m| calendar::month_grid(year, m, delta, today))
        else {
            return MonthView::default();
        };
        let days: Vec<i32> = g.cells.iter().map(|c| c.map_or(0, |d| d as i32)).collect();
        MonthView {
            year: g.first.year(),
            month: g.first.month() as i32,
            days: ModelRc::new(VecModel::from(days)),
            today: g.today.map_or(0, |d| d as i32),
        }
    });
    cal.on_today(|| ui_date(Local::now().date_naive()));
    cal.on_shift(|date, days| {
        core_date(date.clone())
            .ok()
            .and_then(|d| calendar::shift(d, i64::from(days)))
            .map_or(date, ui_date)
    });
}
