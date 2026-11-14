// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Geometry of the Summary charts: sparklines, the balance curves with their axes, and the
//! donut. Everything is computed here as SVG path strings and label positions; the interface
//! only draws them.

use super::money::{AmountStyle, Currency};

/// Sparkline of `series` in a `w` x `h` box: the lowest value at the bottom, the highest at the
/// top, a flat line in the middle when they match. A single day draws a flat line across.
pub fn sparkline_path(series: &[i64], w: f32, h: f32) -> String {
    let (Some(&min), Some(&max)) = (series.iter().min(), series.iter().max()) else {
        return String::new();
    };
    let pad = 1.5;
    let y = |v: i64| {
        if max == min {
            h / 2.0
        } else {
            h - pad - (v - min) as f32 / (max - min) as f32 * (h - 2.0 * pad)
        }
    };
    if series.len() == 1 {
        return format!("M 0 {0:.1} L {w:.1} {0:.1}", y(series[0]));
    }
    let step = w / (series.len() - 1) as f32;
    let mut out = String::new();
    for (i, v) in series.iter().enumerate() {
        let cmd = if i == 0 { 'M' } else { 'L' };
        out.push_str(&format!("{cmd} {:.1} {:.1} ", i as f32 * step, y(*v)));
    }
    out.trim_end().to_string()
}

/// Nice axis shared by both balance series: (low, high, step), with every tick a multiple of
/// `step` (1, 2, 2.5 or 5 times a power of ten), 0 among them, and at most 6 ticks. Nothing to
/// plot gives (0, 1, 0): no ticks besides 0.
pub(super) fn balance_axis(current: &[i64], previous: &[i64]) -> (i64, i64, i64) {
    let all = current.iter().chain(previous);
    let lo = all.clone().copied().min().unwrap_or(0).min(0);
    let hi = all.copied().max().unwrap_or(0).max(0);
    if lo == hi {
        return (0, 1, 0);
    }
    let mut mag = 1i64;
    loop {
        // tenths of `mag`, so 2.5 stays an integer from mag 10 up
        for tenths in [10, 20, 25, 50] {
            if tenths * mag % 10 != 0 {
                continue;
            }
            let step = tenths * mag / 10;
            let (t_lo, t_hi) = (lo.div_euclid(step) * step, -(-hi).div_euclid(step) * step);
            if (t_hi - t_lo) / step < 6 {
                return (t_lo, t_hi, step);
            }
        }
        mag *= 10;
    }
}

/// Distance from the top, 0..1, of `v` in a plot that spans `lo..=hi`.
fn balance_pos(v: f64, lo: i64, hi: i64) -> f32 {
    ((hi as f64 - v) / (hi - lo) as f64) as f32
}

/// Points of a series in a `w` x `h` plot of `span` days: day i at x = i / (span - 1).
fn balance_points(
    series: &[i64],
    span: usize,
    range: (i64, i64),
    w: f32,
    h: f32,
) -> Vec<(f32, f32)> {
    let step = w / span.saturating_sub(1).max(1) as f32;
    let (lo, hi) = range;
    series
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f32 * step, balance_pos(*v as f64, lo, hi) * h))
        .collect()
}

/// `pts` as short "M a L b" pieces (`dash` long, `gap` apart), since Slint paths cannot dash.
pub(super) fn dashed_path(pts: &[(f32, f32)], dash: f32, gap: f32) -> String {
    let period = dash + gap;
    let (mut phase, mut out) = (0.0f32, String::new());
    for pair in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let len = (x1 - x0).hypot(y1 - y0);
        let mut d = 0.0;
        while d < len {
            let on = phase < dash;
            let step = (if on { dash - phase } else { period - phase }).min(len - d);
            if on {
                let at = |t: f32| (x0 + (x1 - x0) * t / len, y0 + (y1 - y0) * t / len);
                let (a, b) = (at(d), at(d + step));
                out.push_str(&format!("M {:.1} {:.1} L {:.1} {:.1} ", a.0, a.1, b.0, b.1));
            }
            d += step;
            phase = (phase + step) % period;
        }
    }
    out.trim_end().to_string()
}

/// One cubic Bézier: start, two control points, end.
type Seg = [(f32, f32); 4];

/// Monotone cubic (Fritsch-Butland) through `pts`, whose x must increase: smooth, and it never
/// overshoots, so it invents no valleys or peaks between days.
pub(super) fn smooth_segments(pts: &[(f32, f32)]) -> Vec<Seg> {
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let d: Vec<f32> = pts
        .windows(2)
        .map(|p| {
            let dx = p[1].0 - p[0].0;
            if dx > 0.0 {
                (p[1].1 - p[0].1) / dx
            } else {
                0.0
            }
        })
        .collect();
    let m = |i: usize| match i {
        0 => d[0],
        i if i == n - 1 => d[n - 2],
        i if d[i - 1] * d[i] <= 0.0 => 0.0,
        i => 2.0 * d[i - 1] * d[i] / (d[i - 1] + d[i]),
    };
    pts.windows(2)
        .enumerate()
        .map(|(i, p)| {
            let h = (p[1].0 - p[0].0) / 3.0;
            [
                p[0],
                (p[0].0 + h, p[0].1 + m(i) * h),
                (p[1].0 - h, p[1].1 - m(i + 1) * h),
                p[1],
            ]
        })
        .collect()
}

/// "M a C b c d C ..." of consecutive segments.
fn curve_path(segs: &[Seg]) -> String {
    let Some(first) = segs.first() else {
        return String::new();
    };
    let mut out = format!("M {:.1} {:.1}", first[0].0, first[0].1);
    for s in segs {
        out.push_str(&format!(
            " C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
            s[1].0, s[1].1, s[2].0, s[2].1, s[3].0, s[3].1
        ));
    }
    out
}

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// The curve of `s` at `t`.
fn bezier_at(s: &Seg, t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let k = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    (
        (0..4).map(|i| k[i] * s[i].0).sum(),
        (0..4).map(|i| k[i] * s[i].1).sum(),
    )
}

/// `s` cut at `t` (de Casteljau).
fn split_segment(s: &Seg, t: f32) -> (Seg, Seg) {
    let ab = lerp(s[0], s[1], t);
    let bc = lerp(s[1], s[2], t);
    let cd = lerp(s[2], s[3], t);
    let abc = lerp(ab, bc, t);
    let bcd = lerp(bc, cd, t);
    let mid = lerp(abc, bcd, t);
    ([s[0], ab, abc, mid], [mid, bcd, cd, s[3]])
}

/// Points along the curve, for dashing it.
pub(super) fn flatten(segs: &[Seg]) -> Vec<(f32, f32)> {
    const STEPS: usize = 16;
    let mut pts: Vec<(f32, f32)> = segs.first().map(|s| s[0]).into_iter().collect();
    for s in segs {
        pts.extend((1..=STEPS).map(|i| bezier_at(s, i as f32 / STEPS as f32)));
    }
    pts
}

/// Closed shapes between the balance curve and the zero line, one per stretch on `positive`'s
/// side. A monotone segment that crosses 0 does it once: it is cut there (de Casteljau) and each
/// half goes to its side.
pub(super) fn balance_fill(
    series: &[i64],
    span: usize,
    range: (i64, i64),
    w: f32,
    h: f32,
    positive: bool,
) -> String {
    let zero_y = balance_pos(0.0, range.0, range.1) * h;
    // -1 on this side of the zero line, 0 on it, 1 on the other side
    let side = |y: f32| {
        if y == zero_y {
            0
        } else if (y < zero_y) == positive {
            -1
        } else {
            1
        }
    };
    let flush = |run: &mut Vec<Seg>, out: &mut String| {
        // a run lying on the zero line has no area
        if run.iter().any(|s| s.iter().any(|p| p.1 != zero_y)) {
            let (first, last) = (run[0][0].0, run[run.len() - 1][3].0);
            out.push_str(&format!(
                "{} L {last:.1} {zero_y:.1} L {first:.1} {zero_y:.1} Z ",
                curve_path(run)
            ));
        }
        run.clear();
    };
    let (mut out, mut run) = (String::new(), Vec::<Seg>::new());
    for s in smooth_segments(&balance_points(series, span, range, w, h)) {
        let (a, b) = (side(s[0].1), side(s[3].1));
        let piece = if a <= 0 && b <= 0 {
            Some(s)
        } else if a >= 0 && b >= 0 {
            None
        } else {
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..30 {
                let mid = (lo + hi) / 2.0;
                if side(bezier_at(&s, mid).1) == a {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let (left, right) = split_segment(&s, (lo + hi) / 2.0);
            Some(if a < 0 { left } else { right })
        };
        match piece {
            Some(p) => {
                if run.last().is_some_and(|l| l[3] != p[0]) {
                    flush(&mut run, &mut out);
                }
                run.push(p);
            }
            None => flush(&mut run, &mut out),
        }
    }
    flush(&mut run, &mut out);
    out.trim_end().to_string()
}

/// One drawable layer of the balance chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BalanceLayer {
    /// The smoothed cumulative net of the month.
    Current,
    /// The dashed curve of the month before.
    Previous,
    /// The area between the current curve and zero, above zero.
    Gain,
    /// The same, below zero: the overdraft.
    Overdraft,
    /// The zero line.
    Zero,
    /// The lines of every tick but zero.
    Grid,
}

/// A tick of an axis: its value (Y) or day (X) and its position from the top (Y) or the left
/// (X), in 0..=1.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisLabel {
    pub value: i64,
    pub pos: f32,
}

/// The Balance of the period chart: the cumulative net of the month so far, the one of the month
/// before, and the number of days the X axis spans.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BalanceChart {
    pub current: Vec<i64>,
    pub previous: Vec<i64>,
    pub span: usize,
}

impl BalanceChart {
    /// Path commands of `layer` in a `w` x `h` plot.
    pub fn layer(&self, layer: BalanceLayer, w: f32, h: f32) -> String {
        let (current, previous, span) = (&self.current, &self.previous, self.span);
        let (lo, hi, step) = balance_axis(current, previous);
        let pts = |s: &[i64]| balance_points(s, span, (lo, hi), w, h);
        let line = |y: f32| format!("M 0 {y:.1} L {w:.1} {y:.1}");
        match layer {
            BalanceLayer::Current if current.len() == 1 => {
                format!("M 0 {0:.1} L 1 {0:.1}", pts(current)[0].1)
            }
            BalanceLayer::Current => curve_path(&smooth_segments(&pts(current))),
            BalanceLayer::Previous => {
                dashed_path(&flatten(&smooth_segments(&pts(previous))), 3.0, 3.0)
            }
            BalanceLayer::Gain | BalanceLayer::Overdraft => {
                balance_fill(current, span, (lo, hi), w, h, layer == BalanceLayer::Gain)
            }
            BalanceLayer::Zero => line(balance_pos(0.0, lo, hi) * h),
            // every tick but 0, which has its own line
            BalanceLayer::Grid if step > 0 => (lo..=hi)
                .step_by(step as usize)
                .filter(|v| *v != 0)
                .map(|v| line(balance_pos(v as f64, lo, hi) * h))
                .collect::<Vec<_>>()
                .join(" "),
            BalanceLayer::Grid => String::new(),
        }
    }

    /// Where the zero line falls, 0..=1 from the top of the plot.
    pub fn zero_pos(&self) -> f32 {
        let (lo, hi, _) = balance_axis(&self.current, &self.previous);
        balance_pos(0.0, lo, hi)
    }

    /// The Y ticks, highest first.
    pub fn y_labels(&self) -> Vec<AxisLabel> {
        let (lo, hi, step) = balance_axis(&self.current, &self.previous);
        if step == 0 {
            return vec![AxisLabel {
                value: 0,
                pos: balance_pos(0.0, lo, hi),
            }];
        }
        (0..=(hi - lo) / step)
            .rev()
            .map(|i| lo + i * step)
            .map(|v| AxisLabel {
                value: v,
                pos: balance_pos(v as f64, lo, hi),
            })
            .collect()
    }

    /// The X ticks: every 5th day (`value` is the day), or day 1 in a very short span.
    pub fn x_labels(&self) -> Vec<AxisLabel> {
        let span = self.span;
        let mut days: Vec<usize> = (5..=span).step_by(5).collect();
        if days.is_empty() {
            days.push(1);
        }
        days.into_iter()
            .map(|d| AxisLabel {
                value: d as i64,
                pos: (d - 1) as f32 / span.saturating_sub(1).max(1) as f32,
            })
            .collect()
    }
}

/// "1,2M", "7,5k", "-2,5k", "340": `amount` in whole units of `c` (USD cents become dollars),
/// no symbol, at most 2 decimals, with the decimal mark of `style`.
pub fn compact_amount(amount: i64, c: &Currency, style: AmountStyle) -> String {
    let units = amount as f64 / 10f64.powi(c.decimals as i32);
    let (n, suffix) = match units.abs() {
        a if a >= 1e6 => (units / 1e6, "M"),
        a if a >= 1e3 => (units / 1e3, "k"),
        _ => (units, ""),
    };
    let text = format!("{n:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    let mark = if style == AmountStyle::Spanish {
        ","
    } else {
        "."
    };
    format!("{}{suffix}", text.replace('.', mark))
}

/// SVG path of donut slice `index` in a `w` x `h` box (24px stroke); `None` is the whole ring.
/// Slices start at the top and go clockwise.
pub fn donut_path(shares: &[f32], w: f32, h: f32, index: Option<usize>) -> String {
    let r = (w.min(h) / 2.0 - 13.0).max(1.0);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let at = |turn: f32| {
        let a = (turn - 0.25) * std::f32::consts::TAU;
        (cx + r * a.cos(), cy + r * a.sin())
    };
    let (from, to) = match index {
        None => (0.0, 1.0),
        Some(i) => {
            let Some(&share) = shares.get(i) else {
                return String::new();
            };
            let start: f32 = shares[..i].iter().sum();
            (start, start + share)
        }
    };
    let (x0, y0) = at(from);
    if to - from > 0.9999 {
        // one arc cannot close a circle: two halves
        let (x1, y1) = at(from + 0.5);
        return format!(
            "M {x0:.1} {y0:.1} A {r:.1} {r:.1} 0 1 1 {x1:.1} {y1:.1} A {r:.1} {r:.1} 0 1 1 {x0:.1} {y0:.1}"
        );
    }
    let (x1, y1) = at(to);
    let large = (to - from > 0.5) as u8;
    format!("M {x0:.1} {y0:.1} A {r:.1} {r:.1} 0 {large} 1 {x1:.1} {y1:.1}")
}
