// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Conversion between the UI `Date` and `NaiveDate`.

use open_productivity_core::chrono::{Datelike, NaiveDate};

use crate::Date;

pub fn ui_date(d: NaiveDate) -> Date {
    Date {
        year: d.year(),
        month: d.month() as i32,
        day: d.day() as i32,
    }
}

/// A date the picker cannot produce is an error, never a default.
pub fn core_date(d: Date) -> Result<NaiveDate, String> {
    NaiveDate::from_ymd_opt(d.year, d.month as u32, d.day as u32)
        .ok_or_else(|| format!("{}-{}-{}", d.year, d.month, d.day))
}
