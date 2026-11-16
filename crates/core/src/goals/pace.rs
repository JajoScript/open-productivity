// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Pace: how the progress of a goal compares with the share of its time already gone.

use chrono::{DateTime, Local, NaiveDate};

use super::{Goal, GoalError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaceLabel {
    Ahead,
    OnPace,
    Behind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pace {
    /// Share of the time already elapsed, a whole 0..=100.
    pub expected: u32,
    pub label: PaceLabel,
}

/// Averages over the goals that have a pace (so not the completed ones), as whole 0..=100 truncated; all 0 when none has.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Portfolio {
    pub count: u32,
    pub progress: u32,
    pub pace: u32,
}

/// The share of the time between `created` and `deadline` already elapsed on `today`, a whole
/// 0..=100 truncated. A deadline on or before the creation day counts as 100.
pub fn expected_pace(created: NaiveDate, deadline: NaiveDate, today: NaiveDate) -> u32 {
    let total = (deadline - created).num_days();
    if total <= 0 {
        return 100;
    }
    ((today - created).num_days() * 100 / total).clamp(0, 100) as u32
}

pub fn pace_label(progress: u32, expected: u32) -> PaceLabel {
    match progress.cmp(&expected) {
        std::cmp::Ordering::Greater => PaceLabel::Ahead,
        std::cmp::Ordering::Less => PaceLabel::Behind,
        std::cmp::Ordering::Equal => PaceLabel::OnPace,
    }
}

/// The pace of a goal with `progress` and a deadline; `created_at` is the stored UTC timestamp.
pub(super) fn pace(
    progress: Option<u32>,
    created_at: &str,
    deadline: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<Option<Pace>, GoalError> {
    let (Some(progress), Some(deadline)) = (progress, deadline) else {
        return Ok(None);
    };
    let created = DateTime::parse_from_rfc3339(created_at)
        .map_err(|_| GoalError::BadDate(created_at.into()))?
        .with_timezone(&Local)
        .date_naive();
    let expected = expected_pace(created, deadline, today);
    Ok(Some(Pace {
        expected,
        label: pace_label(progress, expected),
    }))
}

pub fn portfolio(goals: &[Goal]) -> Portfolio {
    let (mut count, mut progress, mut pace) = (0, 0, 0);
    for g in goals.iter().filter(|g| !g.archived) {
        if let (Some(p), Some(q)) = (g.progress, g.pace) {
            count += 1;
            progress += p;
            pace += q.expected;
        }
    }
    match (progress.checked_div(count), pace.checked_div(count)) {
        (Some(progress), Some(pace)) => Portfolio {
            count,
            progress,
            pace,
        },
        _ => Portfolio::default(),
    }
}

/// The goals with a deadline, not completed or archived, nearest first, ties by title.
pub fn timeline(goals: &[Goal]) -> Vec<&Goal> {
    let mut dated: Vec<&Goal> = goals
        .iter()
        .filter(|g| g.deadline.is_some() && !g.completed && !g.archived)
        .collect();
    dated.sort_by(|a, b| {
        a.deadline
            .cmp(&b.deadline)
            .then_with(|| a.title.cmp(&b.title))
    });
    dated
}
