// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the date picker and `core::calendar`.

use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{AppWindow, Calendar, Date, MonthView};

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
    cal.on_today(|| {
        let t = Local::now().date_naive();
        Date { year: t.year(), month: t.month() as i32, day: t.day() as i32 }
    });
}
