// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the chart geometry (ported from the legacy app).

use super::charts::{
    BalanceChart, BalanceLayer, balance_axis, balance_fill, dashed_path, flatten, smooth_segments,
};
use super::*;

fn chart(current: &[i64], previous: &[i64], span: usize) -> BalanceChart {
    BalanceChart {
        current: current.to_vec(),
        previous: previous.to_vec(),
        span,
    }
}

fn clp() -> &'static Currency {
    currency("CLP").unwrap()
}

#[test]
fn balance_axis_has_nice_ticks_that_cover_the_data_and_zero() {
    assert_eq!(balance_axis(&[5, 10], &[3]), (0, 10, 2));
    assert_eq!(balance_axis(&[-5, -10], &[]), (-10, 0, 2));
    assert_eq!(balance_axis(&[-5, 10], &[20, -30]), (-30, 20, 10));
    assert_eq!(balance_axis(&[-2_400, 5_000], &[]), (-4_000, 6_000, 2_000));
    // 2.5 steps, only from a step of 25 up
    assert_eq!(balance_axis(&[0, 110], &[]), (0, 125, 25));
    assert_eq!(
        balance_axis(&[-37_400, 2_353_250], &[]),
        (-1_000_000, 3_000_000, 1_000_000)
    );
    // nothing to draw: a non-empty range, so the scale never divides by zero
    assert_eq!(balance_axis(&[], &[]), (0, 1, 0));
    assert_eq!(balance_axis(&[0, 0], &[0]), (0, 1, 0));
    // always 2 to 6 ticks, on multiples of the step, with 0 among them
    for hi in [1, 3, 7, 13, 99, 101, 4_999, 123_456, 9_999_999] {
        for lo in [0, -1, -8, -250, -77_777] {
            let (t_lo, t_hi, step) = balance_axis(&[lo, hi], &[]);
            assert!(t_lo <= lo && t_hi >= hi && t_lo <= 0 && t_hi >= 0);
            assert!(t_lo % step == 0 && t_hi % step == 0);
            assert!((2..=6).contains(&((t_hi - t_lo) / step + 1)));
        }
    }
}

#[test]
fn y_labels_are_the_ticks_from_the_top() {
    let ticks = |c: &[i64], p: &[i64]| {
        chart(c, p, 2)
            .y_labels()
            .into_iter()
            .map(|t| t.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(ticks(&[5, 10], &[3]), vec![10, 8, 6, 4, 2, 0]);
    assert_eq!(ticks(&[-5, 10], &[]), vec![10, 5, 0, -5]);
    assert_eq!(ticks(&[0, 0], &[]), vec![0]);
    let l = chart(&[-10, 30], &[], 2).y_labels();
    assert_eq!((l[0].value, l[0].pos), (30, 0.0));
    assert_eq!((l[l.len() - 1].value, l[l.len() - 1].pos), (-10, 1.0));
    assert_eq!(l.iter().find(|t| t.value == 0).unwrap().pos, 0.75);
    assert_eq!(chart(&[-10, 30], &[], 2).zero_pos(), 0.75);
    assert_eq!(chart(&[], &[], 2).zero_pos(), 1.0);
}

#[test]
fn amounts_are_compact_with_the_decimal_mark_of_the_style() {
    let usd = currency("USD").unwrap();
    let es = AmountStyle::Spanish;
    assert_eq!(compact_amount(7_500, clp(), es), "7,5k");
    assert_eq!(compact_amount(5_000, clp(), es), "5k");
    assert_eq!(compact_amount(0, clp(), es), "0");
    assert_eq!(compact_amount(-2_500, clp(), es), "-2,5k");
    assert_eq!(compact_amount(1_200_000, clp(), es), "1,2M");
    assert_eq!(compact_amount(-1_250_000, clp(), es), "-1,25M");
    assert_eq!(compact_amount(340, clp(), es), "340");
    assert_eq!(
        compact_amount(1_500_000, clp(), AmountStyle::English),
        "1.5M"
    );
    // USD is in cents but shown in dollars
    assert_eq!(compact_amount(750_000, usd, es), "7,5k");
    assert_eq!(compact_amount(12_345, usd, es), "123,45");
    assert_eq!(compact_amount(0, usd, es), "0");
}

#[test]
fn smoothing_keeps_the_data_points_and_never_overshoots() {
    assert!(smooth_segments(&[(0.0, 1.0)]).is_empty());
    let pts = [
        (0.0, 90.0),
        (10.0, 80.0),
        (20.0, 80.0),
        (30.0, 20.0),
        (40.0, 10.0),
    ];
    let segs = smooth_segments(&pts);
    assert_eq!(segs.len(), 4);
    for (s, p) in segs.iter().zip(pts.windows(2)) {
        assert_eq!((s[0], s[3]), (p[0], p[1]));
        // a falling series: the control points stay between the two ends
        for c in [s[1], s[2]] {
            assert!(c.1 <= s[0].1 && c.1 >= s[3].1, "{c:?} out of {s:?}");
        }
    }
    let flat = flatten(&segs);
    assert_eq!((flat[0], flat[flat.len() - 1]), (pts[0], pts[4]));
}

#[test]
fn balance_line_is_a_curve_scaled_to_the_shared_range() {
    use BalanceLayer::*;
    assert_eq!(
        chart(&[0, 5, 10], &[], 3).layer(Current, 100.0, 100.0),
        "M 0.0 100.0 C 16.7 83.3 33.3 66.7 50.0 50.0 C 66.7 33.3 83.3 16.7 100.0 0.0"
    );
    // the previous month stretches the range and the span
    assert_eq!(
        chart(&[0, 5], &[20], 4).layer(Current, 90.0, 100.0),
        "M 0.0 100.0 C 10.0 91.7 20.0 83.3 30.0 75.0"
    );
    assert_eq!(chart(&[], &[], 30).layer(Current, 90.0, 100.0), "");
    assert_eq!(
        chart(&[0, 0], &[], 2).layer(Zero, 50.0, 10.0),
        "M 0 10.0 L 50.0 10.0"
    );
    // dashed previous period: short pieces only
    let dots = chart(&[0, 5], &[0, 10], 2).layer(Previous, 60.0, 100.0);
    assert!(dots.starts_with("M 0.0 100.0 L ") && !dots.contains('C'));
    assert_eq!(chart(&[0, 5], &[], 2).layer(Previous, 60.0, 100.0), "");
}

#[test]
fn grid_lines_are_every_tick_but_zero() {
    let grid = chart(&[5, 10], &[3], 2).layer(BalanceLayer::Grid, 10.0, 100.0);
    assert_eq!(grid.matches('M').count(), 5);
    assert!(grid.starts_with("M 0 80.0 L 10.0 80.0 M 0 60.0"));
    assert_eq!(
        chart(&[0], &[], 2).layer(BalanceLayer::Grid, 10.0, 100.0),
        ""
    );
}

#[test]
fn balance_fill_is_gain_above_zero_and_overdraft_below() {
    let range = (0, 20);
    assert_eq!(
        balance_fill(&[10, 20], 2, range, 10.0, 20.0, true),
        "M 0.0 10.0 C 3.3 6.7 6.7 3.3 10.0 0.0 L 10.0 20.0 L 0.0 20.0 Z"
    );
    assert!(balance_fill(&[10, 20], 2, range, 10.0, 20.0, false).is_empty());
    let range = (-20, 0);
    assert!(balance_fill(&[-10, -20], 2, range, 10.0, 20.0, true).is_empty());
    assert!(balance_fill(&[-10, -20], 2, range, 10.0, 20.0, false).starts_with("M 0.0 "));
    assert!(balance_fill(&[], 5, (0, 1), 10.0, 20.0, true).is_empty());
    assert!(balance_fill(&[0, 0, 0], 3, (0, 1), 10.0, 20.0, false).is_empty());
}

#[test]
fn balance_fill_is_cut_where_the_line_crosses_zero() {
    let pos = balance_fill(&[-10, 10], 2, (-10, 10), 10.0, 20.0, true);
    assert_eq!(
        pos,
        "M 5.0 10.0 C 6.7 6.7 8.3 3.3 10.0 0.0 L 10.0 10.0 L 5.0 10.0 Z"
    );
    let neg = balance_fill(&[-10, 10], 2, (-10, 10), 10.0, 20.0, false);
    assert_eq!(
        neg,
        "M 0.0 20.0 C 1.7 16.7 3.3 13.3 5.0 10.0 L 5.0 10.0 L 0.0 10.0 Z"
    );
    // two crossings make two shapes above zero and one below
    let s = [5, -5, 5];
    let above = balance_fill(&s, 3, (-5, 5), 10.0, 10.0, true);
    assert_eq!(above.matches('Z').count(), 2);
    let below = balance_fill(&s, 3, (-5, 5), 10.0, 10.0, false);
    assert_eq!(below.matches('Z').count(), 1);
}

#[test]
fn dashes_are_short_segments_along_the_line() {
    assert_eq!(
        dashed_path(&[(0.0, 0.0), (10.0, 0.0)], 4.0, 3.0),
        "M 0.0 0.0 L 4.0 0.0 M 7.0 0.0 L 10.0 0.0"
    );
    // the pattern continues across a vertex
    let two = dashed_path(&[(0.0, 0.0), (5.0, 0.0), (5.0, 5.0)], 4.0, 3.0);
    assert!(two.starts_with("M 0.0 0.0 L 4.0 0.0 M 5.0 2.0"));
    assert!(dashed_path(&[(1.0, 1.0)], 4.0, 3.0).is_empty());
}

#[test]
fn x_labels_are_every_fifth_day() {
    let days = |n| {
        chart(&[], &[], n)
            .x_labels()
            .iter()
            .map(|l| l.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(days(31), vec![5, 10, 15, 20, 25, 30]);
    assert_eq!(days(28), vec![5, 10, 15, 20, 25]);
    assert_eq!(chart(&[], &[], 31).x_labels()[0].pos, 4.0 / 30.0);
    assert_eq!(chart(&[], &[], 11).x_labels()[1].pos, 0.9);
    assert_eq!(chart(&[], &[], 1).x_labels()[0].value, 1);
    assert_eq!(days(3), vec![1]);
}

#[test]
fn sparkline_spans_the_box() {
    assert_eq!(
        sparkline_path(&[0, 10, 5], 100.0, 20.0),
        "M 0.0 18.5 L 50.0 1.5 L 100.0 10.0"
    );
    assert_eq!(
        sparkline_path(&[7, 7], 100.0, 20.0),
        "M 0.0 10.0 L 100.0 10.0"
    );
    assert_eq!(sparkline_path(&[3], 100.0, 20.0), "M 0 10.0 L 100.0 10.0");
    assert!(sparkline_path(&[], 100.0, 20.0).is_empty());
}

#[test]
fn donut_slices_follow_the_shares() {
    assert_eq!(
        donut_path(&[0.5, 0.5], 100.0, 100.0, Some(0)),
        "M 50.0 13.0 A 37.0 37.0 0 0 1 50.0 87.0"
    );
    assert!(donut_path(&[0.5, 0.5], 100.0, 100.0, Some(1)).starts_with("M 50.0 87.0 A"));
    assert_eq!(
        donut_path(&[1.0], 100.0, 100.0, Some(0))
            .matches('A')
            .count(),
        2
    );
    assert_eq!(donut_path(&[], 100.0, 100.0, None).matches('A').count(), 2);
    assert!(donut_path(&[0.25], 100.0, 100.0, Some(3)).is_empty());
}
