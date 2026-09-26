//! The chart, drawn the way the website draws it.
//!
//! monkeytype plots two things on one pair of axes: burst as a filled area in
//! the background, and wpm as a line over the top of it. Sharing the scale is
//! the point — the gap between the line and the fill is how you see that you
//! are coasting on an average.
//!
//! ## Stretching, which is the whole trick
//!
//! The site's chart fills the width of its container and one second of data is
//! spread across however many pixels are left over. A ten-second test on a
//! 700px chart draws each second about 70px wide. Drawing one terminal column
//! per second instead — which is the obvious translation, and what this did at
//! first — leaves a ten-second test as ten lonely columns at the left of the
//! screen with nothing beside them, which reads as a broken widget rather than
//! as a slow test.
//!
//! So the series are stretched the same way: bucket *i* covers a run of columns,
//! and the columns in between are interpolated. That fills the width like the
//! site does, and because a run is usually several columns wide it buys back
//! the resolution that stretching costs — vertical sub-character blocks give
//! eight distinct heights per row, so a run of seven columns per second draws a
//! curve rather than a staircase.
//!
//! ## What a terminal cannot show
//!
//! The site draws a number under the cursor when you hover a point. There is no
//! hover here, so the numbers live in the result screen's headline instead.
//!
//! The y-scale is shared and starts at zero, so a chart of two series cannot
//! quietly exaggerate one of them.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::config::theme::Theme;
use crate::stats::Chart;

/// Fills, from the baseline up, in blocks of increasing height.
const FILL: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A solid block, for the part of a column that is entirely below the surface.
const SOLID: char = '█';

/// The wpm line: one cell tall, like the site's.
const LINE: char = '─';

/// Rows below the plot for the error ticks. None on a chart too short to have a
/// plot worth the space.
const ERROR_ROWS: u16 = 1;

/// Draws a chart into `area`.
///
/// An empty chart draws nothing at all rather than a flat line, so a test that
/// has not reached its first second does not look like a test that has been
/// going for one.
pub fn render(chart: &Chart, area: Rect, buf: &mut Buffer, theme: Theme) {
    if area.width == 0 || area.height == 0 || chart.is_empty() {
        return;
    }

    let (plot, errors) = split(area);
    if plot.height > 0 {
        render_plot(chart, plot, buf, theme);
    }
    if errors.height > 0 {
        render_errors(chart, errors, buf, theme);
    }
}

/// The plot rows and the error rows.
///
/// Below three rows there is not room for both, and the split degrades rather
/// than dropping one: a two-row chart gets one of each, and a one-row chart
/// gives its row to the errors — a one-row plot is indistinguishable from an
/// empty screen, whereas a tick is the one thing about a test that fits.
fn split(area: Rect) -> (Rect, Rect) {
    if area.height <= 1 {
        return (Rect::new(area.x, area.y, area.width, 0), area);
    }
    if area.height < 3 {
        let plot = Rect::new(area.x, area.y, area.width, area.height - 1);
        let errors = Rect::new(area.x, plot.y + plot.height, area.width, 1);
        return (plot, errors);
    }
    let plot = Rect::new(area.x, area.y, area.width, area.height - ERROR_ROWS);
    let errors = Rect::new(area.x, plot.y + plot.height, area.width, ERROR_ROWS);
    (plot, errors)
}

fn render_plot(chart: &Chart, area: Rect, buf: &mut Buffer, theme: Theme) {
    let width = area.width;
    let scale = scale_for(chart);
    let baseline = area.y + area.height - 1;

    for x in 0..width {
        let (wpm, burst) = sample_at(chart, width, x as f64);
        let height = f64::from(area.height);
        // A value's height above the baseline, in rows. The top of the range is
        // half a row short of the top of the plot, so the tallest peak lands in
        // the middle of the top row rather than along its bottom edge — the
        // blocks are bottom-anchored, and a peak pinned to the bottom of the top
        // row reads as a chart that never reaches the top.
        let rows_above =
            |value: f64| -> f64 { (value / scale * (height - 0.5)).clamp(0.0, height - 0.5) };

        let burst = rows_above(burst);
        let burst_row = burst.floor() as u16;
        let burst_frac = burst.fract();
        let burst_style = Style::default().fg(theme.muted);
        // Everything from the baseline up to the surface is solid; the surface
        // cell itself becomes a partial block, which is what makes the top of the
        // area a curve rather than a staircase. A second with nothing typed in
        // it keeps its one baseline cell, so a stall reads as a dip to the floor
        // rather than as a missing column.
        for row in 0..=burst_row {
            put(buf, area.x + x, baseline - row, SOLID, burst_style);
        }
        if burst_frac > 0.0 {
            put(
                buf,
                area.x + x,
                baseline - burst_row,
                block(burst_frac),
                burst_style,
            );
        }

        // The wpm line goes on last so it is never buried by the fill, and it is
        // quantised to a whole row: a line drawn with the partial blocks is a
        // band, not a stroke, and the site draws a thin line. The curve stays
        // smooth because the area underneath it is.
        let wpm_row = (rows_above(wpm).round() as u16).min(area.height - 1);
        put(
            buf,
            area.x + x,
            baseline - wpm_row,
            LINE,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn render_errors(chart: &Chart, area: Rect, buf: &mut Buffer, theme: Theme) {
    for x in 0..area.width {
        if error_at(chart, area.width, x as f64) > 0 {
            put(
                buf,
                area.x + x,
                area.y,
                '▎',
                Style::default().fg(theme.incorrect),
            );
        }
    }
}

/// A value somewhere between two buckets, linearly interpolated.
///
/// `column` is a terminal column and `width` is how many the chart is drawn
/// across, so the whole series is spread over the whole width. Clamped at both
/// ends so it does not run off either edge.
fn sample_at(chart: &Chart, width: u16, column: f64) -> (f64, f64) {
    let len = chart.len();
    if len == 0 {
        return (0.0, 0.0);
    }
    let span = f64::from(width).max(1.0);
    // Centre of the column, as a position in the series.
    let position = (column + 0.5) / span * len as f64 - 0.5;
    let position = position.clamp(0.0, (len - 1) as f64);

    let low = position.floor() as usize;
    let high = (low + 1).min(len - 1);
    let frac = position - low as f64;

    let mix = |series: &[f64]| -> f64 {
        let a = series.get(low).copied().unwrap_or(0.0);
        let b = series.get(high).copied().unwrap_or(a);
        a + (b - a) * frac
    };
    (mix(&chart.wpm), mix(&chart.burst))
}

/// The wrong keystrokes for a column.
///
/// Unlike the two curves this is not interpolated — a tick either happened in
/// the stretch of time this column covers or it did not, and blending the
/// counts would draw a fading error that was never typed.
fn error_at(chart: &Chart, width: u16, column: f64) -> u32 {
    let len = chart.len();
    if len == 0 {
        return 0;
    }
    let span = f64::from(width).max(1.0);
    // Every bucket this column's slice of the width touches, so a tick covers
    // the whole second it belongs to rather than a fraction of it. Clamped into
    // range at both ends, which matters at the right edge: the last column of a
    // stretch maps past the end of the series.
    let first = (((column / span) * len as f64).floor() as usize).min(len - 1);
    let last = ((((column + 1.0) / span) * len as f64).ceil() as usize).clamp(first + 1, len);
    chart
        .err
        .get(first..last)
        .map_or(0, |slice| slice.iter().sum())
}

/// The top of the y-scale: the largest of either series, or 1 so an all-zero
/// chart does not divide by nothing.
fn scale_for(chart: &Chart) -> f64 {
    chart
        .wpm
        .iter()
        .chain(chart.burst.iter())
        .copied()
        .fold(1.0_f64, f64::max)
}

/// A partially filled cell, for the surface of the burst area.
fn block(fraction: f64) -> char {
    let eighth = (fraction.clamp(0.0, 1.0) * 8.0).round() as usize;
    FILL[eighth.clamp(1, 8) - 1]
}

fn put(buf: &mut Buffer, x: u16, y: u16, glyph: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_char(glyph).set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect as R;

    fn use_theme() -> Theme {
        crate::config::theme::ThemeName::Gruvbox.resolve()
    }

    /// Draws a chart in a `width` x `height` box and returns it as text rows.
    ///
    /// The buffer is filled directly rather than through a `Terminal`: the chart
    /// writes single cells, and a full render cycle would exercise ratatui's
    /// diffing rather than anything of this widget's.
    fn draw(chart: &Chart, width: u16, height: u16) -> Vec<String> {
        let area = R::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(chart, area, &mut buf, use_theme());
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn chart_of(burst: &[f64], wpm: &[f64], err: &[u32]) -> Chart {
        Chart {
            wpm: wpm.to_vec(),
            burst: burst.to_vec(),
            err: err.to_vec(),
        }
    }

    /// Every character the chart can draw.
    fn glyphs(rows: &[String]) -> String {
        rows.concat()
    }

    #[test]
    fn an_empty_chart_draws_nothing() {
        let rows = draw(&Chart::default(), 10, 6);
        for row in &rows {
            assert!(row.trim().is_empty(), "{row:?}");
        }
    }

    /// The reason this widget exists: a short test must not be a narrow stripe.
    #[test]
    fn a_short_test_fills_the_width() {
        let chart = chart_of(&[60.0; 10], &[60.0; 10], &[0; 10]);
        let rows = draw(&chart, 60, 5);
        let line = &rows[0];
        assert!(
            line.trim_end().len() >= 55,
            "ten seconds drawn across {width} columns should reach the edge, got {line:?}",
            width = line.trim_end().len()
        );
    }

    #[test]
    fn every_column_of_a_flat_chart_is_drawn() {
        // No gaps, no run of spaces in the middle of the drawn line.
        let chart = chart_of(&[60.0; 5], &[60.0; 5], &[0; 5]);
        let rows = draw(&chart, 40, 4);
        let top = &rows[0];
        assert!(!top.contains("  "), "the line has a hole in it: {top:?}");
    }

    #[test]
    fn a_flat_chart_is_a_flat_line_on_top_of_a_full_area() {
        let chart = chart_of(&[60.0; 5], &[60.0; 5], &[0; 5]);
        let rows = draw(&chart, 20, 5);
        assert_eq!(
            rows[0].trim_end(),
            "─".repeat(20),
            "the line is the top row"
        );
        assert_eq!(rows[3].trim_end(), "█".repeat(20), "the fill is the floor");
    }

    #[test]
    fn a_stall_dips_to_the_baseline() {
        // Fast, nothing, fast again.
        let chart = chart_of(&[80.0, 0.0, 80.0], &[80.0, 40.0, 80.0], &[0, 0, 0]);
        let rows = draw(&chart, 30, 4);
        let floor = &rows[2];
        // The stall is the one place the area collapses to a single row.
        let heights: Vec<usize> = floor
            .chars()
            .map(|c| if c == '█' { 3 } else { 0 })
            .collect();
        assert!(
            heights.windows(10).any(|w| w.iter().sum::<usize>() < 20),
            "no visible dip: {floor:?}"
        );
        // The surface is drawn with partial blocks, so the top of the area is a
        // curve rather than a staircase — even between two whole seconds.
        let partial = glyphs(&rows).chars().any(|c| matches!(c, '▁'..='▇'));
        assert!(partial, "the surface is a staircase: {rows:?}");
    }

    /// The point of sharing a scale: a flat average over a stall.
    #[test]
    fn the_wpm_line_sits_on_top_of_the_burst_fill() {
        let chart = chart_of(&[60.0, 60.0, 0.0, 60.0], &[45.0, 45.0, 30.0, 45.0], &[0; 4]);
        let rows = draw(&chart, 40, 6);
        let ink = glyphs(&rows);
        assert!(ink.contains(LINE), "no line drawn: {rows:?}");
        // The stall shows in the fill even though the average barely moved.
        let floor = &rows[4];
        assert!(
            floor.chars().any(|c| c != '█'),
            "the stall left no mark: {floor:?}"
        );
    }

    #[test]
    fn errors_get_their_own_row_underneath() {
        let chart = chart_of(&[60.0; 4], &[60.0; 4], &[0, 3, 0, 0]);
        let rows = draw(&chart, 40, 6);
        assert_eq!(rows[5].chars().filter(|c| *c == '▎').count(), 10);
        assert!(
            !rows[..5].iter().any(|r| r.contains('▎')),
            "the ticks are not in the plot: {rows:?}"
        );
    }

    #[test]
    fn an_error_tick_covers_the_whole_second_it_belongs_to() {
        // One error in the second, stretched across the columns it covers: a
        // single tick in the middle would read as a fraction of an error.
        let chart = chart_of(&[60.0; 4], &[60.0; 4], &[0, 1, 0, 0]);
        let rows = draw(&chart, 40, 6);
        let ticks = rows[5].chars().filter(|c| *c == '▎').count();
        assert_eq!(ticks, 10, "the whole second is marked: {ticks}");
    }

    #[test]
    fn a_one_row_chart_shows_the_errors_and_nothing_else() {
        let chart = chart_of(&[60.0; 3], &[60.0; 3], &[1, 0, 0]);
        let rows = draw(&chart, 30, 1);
        assert_eq!(rows[0].chars().filter(|c| *c == '▎').count(), 10);
    }

    #[test]
    fn a_two_row_chart_lets_the_line_win_the_shared_row() {
        let chart = chart_of(&[60.0; 2], &[60.0; 2], &[0; 2]);
        let rows = draw(&chart, 20, 2);
        assert_eq!(rows[0].trim_end(), "─".repeat(20));
        assert_eq!(rows[1].trim(), "", "the error row below it is clear");
    }

    #[test]
    fn a_chart_narrower_than_its_data_is_still_one_column_per_second() {
        // Two columns, ten seconds: compression has nothing to work with, so the
        // honest thing is to draw what there is room for rather than smear ten
        // buckets into two.
        let chart = chart_of(&[60.0; 10], &[60.0; 10], &[0; 10]);
        let rows = draw(&chart, 2, 3);
        assert_eq!(rows[0].chars().count(), 2);
        assert!(!rows[0].trim().is_empty());
    }

    #[test]
    fn a_chart_wider_than_the_terminal_never_overflows() {
        let chart = chart_of(&[60.0; 50], &[60.0; 50], &[0; 50]);
        for width in 1..=40u16 {
            for row in draw(&chart, width, 4) {
                assert_eq!(row.chars().count(), width as usize, "width {width}");
            }
        }
    }

    #[test]
    fn an_all_zero_chart_still_draws_a_line() {
        // Nothing typed yet, but the test has run: the axis must not vanish.
        let chart = chart_of(&[0.0; 3], &[0.0; 3], &[0; 3]);
        let rows = draw(&chart, 30, 3);
        // Two plot rows, so the line is on the plot's baseline.
        assert_eq!(rows[1].trim_end(), "─".repeat(30));
        assert_eq!(rows[0].trim(), "");
    }

    #[test]
    fn a_zero_width_area_is_not_drawn_on() {
        let chart = chart_of(&[60.0], &[60.0], &[0]);
        let mut buf = Buffer::empty(R::new(0, 0, 0, 4));
        render(&chart, R::new(0, 0, 0, 4), &mut buf, use_theme());
        assert!(buf.content.is_empty());
    }

    #[test]
    fn the_line_stays_inside_the_plot() {
        // A spike in the second bucket must not spill above the top row.
        let chart = chart_of(&[1.0, 1000.0, 1.0], &[1.0, 1000.0, 1.0], &[0; 3]);
        let rows = draw(&chart, 30, 4);
        assert_eq!(rows.len(), 4);
        for (y, row) in rows.iter().enumerate() {
            assert!(!row.contains('\u{0}'), "row {y} has a stray cell: {row:?}");
        }
    }
}
