// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Month grids for date pickers.

use chrono::{Datelike, Months, NaiveDate, TimeDelta};

/// One month laid out in 6 weeks of 7 days, Monday first.
#[derive(Debug, PartialEq, Eq)]
pub struct MonthGrid {
    /// The first day of the month shown.
    pub first: NaiveDate,
    /// Day of the month per cell; `None` for the cells outside it.
    pub cells: [Option<u32>; 42],
    /// Day of the month that is `today`, when today falls in this month.
    pub today: Option<u32>,
}

/// The month `delta` months away from `year`/`month` (1-12). `None` when the month is
/// invalid or the result leaves the supported date range.
// shortcut: weeks always start on Monday; follow the locale when settings get a first-day option.
pub fn month_grid(year: i32, month: u32, delta: i32, today: NaiveDate) -> Option<MonthGrid> {
    let base = NaiveDate::from_ymd_opt(year, month, 1)?;
    let step = Months::new(delta.unsigned_abs());
    let first = if delta >= 0 {
        base.checked_add_months(step)?
    } else {
        base.checked_sub_months(step)?
    };
    let len = first
        .checked_add_months(Months::new(1))?
        .signed_duration_since(first)
        .num_days() as usize;
    let lead = first.weekday().num_days_from_monday() as usize;
    let mut cells = [None; 42];
    for (day, cell) in (1..).zip(&mut cells[lead..lead + len]) {
        *cell = Some(day);
    }
    let today =
        (today.year() == first.year() && today.month() == first.month()).then(|| today.day());
    Some(MonthGrid {
        first,
        cells,
        today,
    })
}

/// Monday of the week that holds `date`.
// shortcut: weeks start on Monday, like `month_grid`; follow the locale with a first-day option.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - TimeDelta::days(date.weekday().num_days_from_monday().into())
}

/// The date `days` away from `date` (negative goes back); `None` out of the supported range.
pub fn shift(date: NaiveDate, days: i64) -> Option<NaiveDate> {
    date.checked_add_signed(TimeDelta::try_days(days)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn days(g: &MonthGrid) -> Vec<u32> {
        g.cells.iter().flatten().copied().collect()
    }

    #[test]
    fn lays_out_from_monday() {
        // 2026-11-01 is a Sunday: six blank cells before it.
        let g = month_grid(2026, 11, 0, d("2026-11-03")).unwrap();
        assert_eq!(g.first, d("2026-11-01"));
        assert!(g.cells[..6].iter().all(Option::is_none));
        assert_eq!(g.cells[6], Some(1));
        assert_eq!(days(&g), (1..=30).collect::<Vec<_>>());
        assert_eq!(g.today, Some(3));
    }

    #[test]
    fn month_starting_monday_has_no_lead() {
        // 2027-02-01 is a Monday.
        let g = month_grid(2027, 2, 0, d("2026-11-03")).unwrap();
        assert_eq!(g.cells[0], Some(1));
        assert_eq!(days(&g).len(), 28);
        assert_eq!(g.today, None);
    }

    #[test]
    fn leap_february_and_six_week_months_fit() {
        assert_eq!(
            days(&month_grid(2028, 2, 0, d("2026-01-01")).unwrap()).len(),
            29
        );
        // 2026-08-01 is a Saturday: 5 blanks + 31 days needs the sixth week.
        let g = month_grid(2026, 8, 0, d("2026-01-01")).unwrap();
        assert_eq!(g.cells[35], Some(31));
    }

    #[test]
    fn delta_crosses_years() {
        let today = d("2026-11-03");
        assert_eq!(
            month_grid(2026, 12, 1, today).unwrap().first,
            d("2027-01-01")
        );
        assert_eq!(
            month_grid(2026, 1, -1, today).unwrap().first,
            d("2025-12-01")
        );
        assert_eq!(
            month_grid(2026, 11, -13, today).unwrap().first,
            d("2025-10-01")
        );
    }

    #[test]
    fn invalid_month_is_none() {
        assert_eq!(month_grid(2026, 0, 0, d("2026-11-03")), None);
        assert_eq!(month_grid(2026, 13, 0, d("2026-11-03")), None);
    }

    #[test]
    fn week_starts_on_monday() {
        // 2026-11-05 is a Thursday.
        assert_eq!(week_start(d("2026-11-05")), d("2026-11-02"));
        assert_eq!(week_start(d("2026-11-02")), d("2026-11-02"));
        // Sunday belongs to the week before, across a year boundary.
        assert_eq!(week_start(d("2027-01-03")), d("2026-12-28"));
    }

    #[test]
    fn shift_crosses_month_and_year_boundaries() {
        assert_eq!(shift(d("2026-11-30"), 1), Some(d("2026-12-01")));
        assert_eq!(shift(d("2026-12-28"), 7), Some(d("2027-01-04")));
        assert_eq!(shift(d("2026-03-01"), -1), Some(d("2026-02-28")));
        assert_eq!(shift(NaiveDate::MAX, 1), None);
    }
}
