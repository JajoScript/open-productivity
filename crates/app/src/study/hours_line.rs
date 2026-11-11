// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The cumulative-hours line as SVG path commands: pure geometry, no rules.

/// A polyline in a 100 x 100 box, left to right, height 1.0 at the top. One point draws a
/// flat line across the box; no points draw nothing.
pub fn line_commands(heights: &[f32]) -> String {
    let y = |h: f32| 100.0 - h.clamp(0.0, 1.0) * 100.0;
    match heights {
        [] => String::new(),
        [h] => format!("M 0 {0:.2} L 100 {0:.2}", y(*h)),
        _ => {
            let last = (heights.len() - 1) as f32;
            heights
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    let cmd = if i == 0 { 'M' } else { 'L' };
                    format!("{cmd} {:.2} {:.2}", i as f32 / last * 100.0, y(*h))
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_the_box_with_the_top_at_full_height() {
        assert_eq!(
            line_commands(&[0.0, 0.5, 1.0]),
            "M 0.00 100.00 L 50.00 50.00 L 100.00 0.00"
        );
    }

    #[test]
    fn one_point_is_a_flat_line_and_none_is_empty() {
        assert_eq!(line_commands(&[0.0]), "M 0 100.00 L 100 100.00");
        assert_eq!(line_commands(&[]), "");
    }
}
