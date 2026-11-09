// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! How often a task repeats and the date of its next turn.

use chrono::{Datelike, Months, NaiveDate, TimeDelta};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Frequency {
    /// The value stored in `task_recurrence.frequency`.
    pub fn as_str(self) -> &'static str {
        match self {
            Frequency::Daily => "daily",
            Frequency::Weekly => "weekly",
            Frequency::Monthly => "monthly",
            Frequency::Yearly => "yearly",
        }
    }

    pub fn parse(s: &str) -> Option<Frequency> {
        [
            Frequency::Daily,
            Frequency::Weekly,
            Frequency::Monthly,
            Frequency::Yearly,
        ]
        .into_iter()
        .find(|f| f.as_str() == s)
    }
}

/// The date `interval` periods after `due`. Monthly and yearly keep `anchor_day` (the day of the
/// month the series started on), capped at the end of a shorter month, so there is no drift:
/// 31 Jan -> 28 Feb -> 31 Mar. `None` when the date leaves the supported range.
pub fn next_date(
    due: NaiveDate,
    freq: Frequency,
    interval: u32,
    anchor_day: u32,
) -> Option<NaiveDate> {
    let days =
        |n: i64| due.checked_add_signed(TimeDelta::try_days(n.checked_mul(interval.into())?)?);
    let months = |n: u32| {
        let first = due
            .with_day(1)?
            .checked_add_months(Months::new(n.checked_mul(interval)?))?;
        let len = first
            .checked_add_months(Months::new(1))?
            .signed_duration_since(first);
        first.with_day(anchor_day.clamp(1, len.num_days() as u32))
    };
    match freq {
        Frequency::Daily => days(1),
        Frequency::Weekly => days(7),
        Frequency::Monthly => months(1),
        Frequency::Yearly => months(12),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn next(due: &str, f: Frequency, n: u32, anchor: u32) -> NaiveDate {
        next_date(d(due), f, n, anchor).unwrap()
    }

    #[test]
    fn daily_and_weekly_add_days() {
        assert_eq!(next("2026-12-30", Frequency::Daily, 3, 30), d("2027-01-02"));
        assert_eq!(next("2026-11-08", Frequency::Weekly, 1, 8), d("2026-11-15"));
        assert_eq!(next("2026-11-08", Frequency::Weekly, 2, 8), d("2026-11-22"));
    }

    #[test]
    fn monthly_keeps_the_anchor_without_drift() {
        let a = next("2027-01-31", Frequency::Monthly, 1, 31);
        assert_eq!(a, d("2027-02-28"));
        let b = next_date(a, Frequency::Monthly, 1, 31).unwrap();
        assert_eq!(b, d("2027-03-31"));
        assert_eq!(
            next("2027-03-31", Frequency::Monthly, 1, 31),
            d("2027-04-30")
        );
        assert_eq!(
            next("2028-01-31", Frequency::Monthly, 1, 31),
            d("2028-02-29")
        );
    }

    #[test]
    fn monthly_intervals_and_year_rollover() {
        assert_eq!(
            next("2026-11-15", Frequency::Monthly, 3, 15),
            d("2027-02-15")
        );
        assert_eq!(
            next("2026-11-30", Frequency::Monthly, 3, 30),
            d("2027-02-28")
        );
        assert_eq!(
            next("2026-12-10", Frequency::Monthly, 1, 10),
            d("2027-01-10")
        );
    }

    #[test]
    fn yearly_handles_leap_day() {
        let a = next("2028-02-29", Frequency::Yearly, 1, 29);
        assert_eq!(a, d("2029-02-28"));
        assert_eq!(
            next("2031-02-28", Frequency::Yearly, 1, 29),
            d("2032-02-29")
        );
        assert_eq!(next("2026-05-05", Frequency::Yearly, 2, 5), d("2028-05-05"));
    }

    #[test]
    fn out_of_range_is_none() {
        assert_eq!(next_date(NaiveDate::MAX, Frequency::Daily, 1, 1), None);
        assert_eq!(next_date(NaiveDate::MAX, Frequency::Yearly, 1, 1), None);
    }

    #[test]
    fn frequency_text_round_trips() {
        for f in [
            Frequency::Daily,
            Frequency::Weekly,
            Frequency::Monthly,
            Frequency::Yearly,
        ] {
            assert_eq!(Frequency::parse(f.as_str()), Some(f));
        }
        assert_eq!(Frequency::parse("hourly"), None);
    }
}
