//! The chart, drawn the way the website draws it.
//!
//! monkeytype plots two things on one pair of axes: burst as a filled area in
//! the background, and wpm as a line over the top of it. Sharing the scale is
//! the point — the gap between the line and the fill is how you see that you
//! are coasting on an average.
//!
//! A terminal has no lines and no fills, so the two are separated by glyph and
//! colour instead:
//!
//! - **burst** is a solid block filled from the baseline up, in the muted
//!   colour. A second with nothing in it is one dim block at the baseline, which
//!   is what makes a stall read as a dip rather than as a missing column.
//! - **wpm** is drawn as a line in the accent colour, connected between
//!   columns with box-drawing pieces so a run of unequal seconds still looks
//!   like one stroke.
//! - **errors** get a row of their own underneath, one tick per second, the way
//!   the site draws them below the plot rather than on an axis.
//!
//! The y-scale is shared and starts at zero, so a chart of two series cannot
//! quietly exaggerate one of them.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::config::theme::Theme;
use crate::stats::Chart;

/// The tallest a chart is ever drawn.
///
/// A test over an hour would otherwise ask for 3600 columns, and a terminal is
/// rarely wider than a few hundred. Long tests are drawn compressed rather than
/// clipped: every second still contributes, and nothing silently disappears off
/// the right edge.
const MAX_COLUMNS: u16 = 200;

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
    let columns = columns_for(chart, area.width);
    let scale = scale_for(chart);
    let baseline = area.y + area.height - 1;

    for column in 0..columns {
        let x = area.x + column;
        let burst = level(chart.burst_at(column, columns), scale, area.height);
        // Filled from the baseline up to the second's own speed.
        for row in 0..=burst {
            put(
                buf,
                x,
                baseline - row,
                '█',
                Style::default().fg(theme.muted),
            );
        }
    }

    // The wpm line goes on last so it is never buried by the fill.
    let mut previous: Option<u16> = None;
    for column in 0..columns {
        let x = area.x + column;
        let y = baseline - level(chart.wpm_at(column, columns), scale, area.height);
        let glyph = match previous {
            // Connected, so a jump between two seconds reads as one stroke
            // rather than as two separate dashes.
            Some(before) if y < before => '╭',
            Some(before) if y > before => '╯',
            _ => '─',
        };
        put(
            buf,
            x,
            y,
            glyph,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );
        previous = Some(y);
    }
}

fn render_errors(chart: &Chart, area: Rect, buf: &mut Buffer, theme: Theme) {
    let columns = columns_for(chart, area.width);
    for column in 0..columns {
        if chart.err_at(column, columns) > 0 {
            put(
                buf,
                area.x + column,
                area.y,
                '▎',
                Style::default().fg(theme.incorrect),
            );
        }
    }
}

/// How many terminal columns the chart is drawn across.
///
/// One column per second while it fits. Past [`MAX_COLUMNS`] the seconds are
/// merged into equal groups and the *worst* of each group is drawn, because a
/// smoothed average would hide exactly the stalls a chart exists to show.
fn columns_for(chart: &Chart, width: u16) -> u16 {
    width.min(MAX_COLUMNS).min(chart.len() as u16)
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

/// A value's row above the baseline, `0` being the bottom row.
fn level(value: f64, scale: f64, height: u16) -> u16 {
    if height <= 1 {
        return 0;
    }
    let rows = f64::from(height - 1);
    let scaled = (value / scale * rows).round();
    scaled.clamp(0.0, rows) as u16
}

fn put(buf: &mut Buffer, x: u16, y: u16, glyph: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_char(glyph).set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn use_theme() -> Theme {
        crate::config::theme::ThemeName::Gruvbox.resolve()
    }

    /// Draws a chart in a `width` x `height` box and returns it as text rows.
    ///
    /// The buffer is filled directly rather than through a `Terminal`: the chart
    /// writes single cells, and a full render cycle would exercise ratatui's
    /// diffing rather than anything of this widget's.
    fn draw(chart: &Chart, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
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

    #[test]
    fn an_empty_chart_draws_nothing() {
        let rows = draw(&Chart::default(), 10, 6);
        for row in &rows {
            assert!(row.trim().is_empty(), "{row:?}");
        }
    }

    #[test]
    fn a_flat_chart_is_a_flat_line() {
        // Every second at 60 wpm, on a 3-row plot: the line sits on the top row
        // and the fill reaches the baseline.
        let chart = chart_of(&[60.0; 5], &[60.0; 5], &[0; 5]);
        let rows = draw(&chart, 5, 4);
        assert_eq!(rows[0], "─────", "the wpm line is the top plot row");
        assert_eq!(rows[2], "█████", "the fill is the bottom plot row");
        assert_eq!(rows[3], "     ", "the error row is clear");
    }

    #[test]
    fn a_stall_dips_to_the_baseline() {
        // Fast, nothing, fast again. The empty second gets one dim block where
        // the others get three, so the dip is visible rather than a gap.
        let chart = chart_of(&[80.0, 0.0, 80.0], &[80.0, 40.0, 80.0], &[0, 0, 0]);
        let rows = draw(&chart, 3, 4);
        assert_eq!(rows[2], "███", "every second touches the baseline");
        assert_eq!(rows[0], "─ ╭", "the wpm line steps down ...");
        assert_eq!(rows[1], "█╯█", "... and back up over the fill");
    }

    #[test]
    fn the_wpm_line_sits_on_top_of_the_burst_fill() {
        // A stall in the middle of four good seconds. The average barely moves,
        // so the line is flat — and the fill is not, which is the whole reason
        // the two share a scale instead of each getting their own.
        let chart = chart_of(&[60.0, 60.0, 0.0, 60.0], &[45.0, 45.0, 30.0, 45.0], &[0; 4]);
        let rows = draw(&chart, 4, 5);
        assert_eq!(rows[1], "────", "a steady average is a flat line");
        assert_eq!(rows[0], "██ █", "the three fast seconds reach the top");
        assert_eq!(rows[2], "██ █", "and the stall is a gap, not a dip");
        assert_eq!(rows[3], "████", "the stall still touches the baseline");
    }

    #[test]
    fn errors_get_their_own_row_underneath() {
        let chart = chart_of(&[60.0; 4], &[60.0; 4], &[0, 3, 0, 0]);
        let rows = draw(&chart, 4, 5);
        assert_eq!(rows[4].chars().filter(|c| *c == '▎').count(), 1);
        assert!(
            !rows[..4].iter().any(|r| r.contains('▎')),
            "the ticks are not in the plot: {rows:?}"
        );
    }

    #[test]
    fn a_one_row_chart_shows_the_errors_and_nothing_else() {
        // A one-row plot is indistinguishable from a blank screen, so the row
        // goes to the ticks.
        let chart = chart_of(&[60.0; 3], &[60.0; 3], &[1, 0, 0]);
        let rows = draw(&chart, 3, 1);
        assert_eq!(rows[0], "▎  ");
    }

    #[test]
    fn a_two_row_chart_lets_the_line_win_the_shared_row() {
        // One plot row cannot hold a line and the area under it, and the line is
        // the more useful of the two: it is the number you are chasing.
        let chart = chart_of(&[60.0; 2], &[60.0; 2], &[0; 2]);
        let rows = draw(&chart, 2, 2);
        assert_eq!(rows[0], "──", "the plot row is the wpm line");
        assert_eq!(rows[1], "  ", "and the error row below it is clear");
    }

    #[test]
    fn a_chart_wider_than_the_terminal_is_clipped_not_wrapped() {
        let chart = chart_of(&[60.0; 50], &[60.0; 50], &[0; 50]);
        let rows = draw(&chart, 10, 3);
        assert_eq!(rows[0].chars().count(), 10, "no wrapping, no overflow");
    }

    #[test]
    fn a_chart_narrower_than_the_terminal_leaves_the_rest_alone() {
        let chart = chart_of(&[60.0; 3], &[60.0; 3], &[0; 3]);
        let rows = draw(&chart, 20, 3);
        assert_eq!(rows[0].trim_end().chars().count(), 3);
    }

    #[test]
    fn an_all_zero_chart_still_draws_a_line() {
        // Nothing typed yet, but the test has run: the axis must not vanish.
        // Three rows means a two-row plot and an error row, so the line is on
        // the plot's baseline, not on the bottom of the box.
        let chart = chart_of(&[0.0; 3], &[0.0; 3], &[0; 3]);
        let rows = draw(&chart, 3, 3);
        assert_eq!(rows[1], "───", "a zero average sits on the baseline");
        assert_eq!(rows[0], "   ", "and nothing is above it");
    }

    #[test]
    fn a_zero_width_area_is_not_drawn_on() {
        let chart = chart_of(&[60.0], &[60.0], &[0]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 0, 4));
        render(&chart, Rect::new(0, 0, 0, 4), &mut buf, use_theme());
        assert!(buf.content.is_empty());
    }
}
