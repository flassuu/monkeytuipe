//! Results screen: what the test came to.
//!
//! The order is the order things are read in: how fast, how accurate, the chart
//! that shows the shape of it, then the breakdown underneath for when the
//! headline number is not the interesting one.
//!
//! Everything is recomputed from the event log rather than snapshotted when the
//! test ended. A result is a pure function of the log, so there is nothing to go
//! stale and nothing to keep in sync with the engine.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::config::theme::Theme;
use crate::screens::{Effect, Screen, ScreenKind};
use crate::stats::{KeyUse, TestResult};
use crate::widgets;

/// Rows for the headline figures.
const HEADLINE_ROWS: u16 = 2;

/// Rows for the chart. More than the typing screen gives it, because by now
/// nothing else on screen is competing for the eye.
const CHART_ROWS: u16 = 12;

/// Rows for the per-key breakdown.
const KEYS_ROWS: u16 = 3;

/// How many keys the breakdown shows.
///
/// The site draws the whole keyboard. Here the keys a test actually used are the
/// interesting ones, and there is rarely room for more than this.
const MAX_KEYS: usize = 40;

#[derive(Debug, Default)]
pub struct Results;

impl Screen for Results {
    fn render(&self, app: &App, frame: &mut Frame) {
        let theme = app.theme();
        let area = frame.area();
        frame.render_widget(Paragraph::new("").style(theme.base()), area);

        // Nothing to report until a test has run, which is the state the screen
        // is opened in if the user asks for it early.
        let Some(result) = app.result() else {
            return;
        };

        render_headline(&result, theme, area, frame);

        // Below the headline: the chart, then the breakdown, then the footer.
        // The chart is the first band to lose rows on a short terminal, because
        // it is the only one that still means something with less of it.
        let below = area.y + HEADLINE_ROWS.min(area.height);
        let free = area.y + area.height - below;
        let keys_height = KEYS_ROWS.min(free.saturating_sub(2));
        let chart_height = free
            .saturating_sub(keys_height)
            .saturating_sub(1)
            .min(CHART_ROWS);

        if chart_height > 0 {
            widgets::render(
                &result.chart,
                Rect::new(area.x, below, area.width, chart_height),
                frame.buffer_mut(),
                theme,
            );
        }
        if keys_height > 0 {
            render_keys(
                &result,
                theme,
                Rect::new(area.x, below + chart_height + 1, area.width, keys_height),
                frame,
            );
        }
        if area.height > below + chart_height + keys_height {
            render_footer(
                Rect::new(area.x, area.y + area.height - 1, area.width, 1),
                theme,
                frame,
            );
        }
    }

    fn handle(&mut self, action: Action) -> Vec<Effect> {
        match action {
            // Leaving for a new test restarts it, so the user is not dropped
            // back onto a finished test with a fresh word list and no clock.
            Action::Restart | Action::StartTest | Action::Back => {
                vec![Effect::Switch(ScreenKind::Typing), Effect::RestartTest]
            }
            Action::Settings => vec![Effect::Switch(ScreenKind::Settings)],
            Action::Quit => vec![Effect::Quit],
            // A finished test is a result, not a place to type. Anything that
            // would edit it is dropped, so a stray keypress cannot alter a score
            // that is already on screen.
            Action::Char(_)
            | Action::Backspace
            | Action::Skip
            | Action::Up
            | Action::Down
            | Action::Left
            | Action::Right
            | Action::Select => Vec::new(),
        }
    }
}

/// The big numbers, over two rows: speed on one, accuracy on the other.
///
/// Centred, and clipped rather than wrapped — a counter row that wrapped would
/// be two rows and would push the chart off a short terminal.
fn render_headline(result: &TestResult, theme: Theme, area: Rect, frame: &mut Frame) {
    let shown = area.height.min(HEADLINE_ROWS);
    if shown == 0 {
        return;
    }

    let speed = figures(
        &[
            ("wpm", format!("{:.0}", result.wpm)),
            ("raw", format!("{:.0}", result.raw_wpm)),
            ("chars", result.chars.correct_word.to_string()),
        ],
        theme,
    );
    render_line(area, speed, frame);

    if shown < 2 {
        return;
    }
    let quality = figures(
        &[
            ("acc", format!("{:.1}%", result.accuracy)),
            ("cons", format!("{:.0}%", result.consistency)),
            ("time", format!("{:.1}s", result.duration_secs)),
        ],
        theme,
    );
    render_line(Rect::new(area.x, area.y + 1, area.width, 1), quality, frame);
}

/// A row of `label value` pairs with the values in bold.
fn figures(pairs: &[(&str, String)], theme: Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (label, value) in pairs {
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(format!("{label} "), theme.chrome()));
        spans.push(Span::styled(
            value.clone(),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans).alignment(Alignment::Center)
}

fn render_line(area: Rect, line: Line<'static>, frame: &mut Frame) {
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

/// The per-key breakdown: which characters the test used, and which of them were
/// pressed in the wrong place.
///
/// Laid out on a fixed-width grid, so a long list wraps predictably and the
/// columns stay in step. A key with a count of wrong uses after it is drawn in
/// the error colour, which is the site's letter table reduced to what a terminal
/// can honestly show.
fn render_keys(result: &TestResult, theme: Theme, area: Rect, frame: &mut Frame) {
    let shown: Vec<KeyUse> = result.keys.keys.iter().take(MAX_KEYS).copied().collect();
    if shown.is_empty() {
        return;
    }

    // The cell text is built first and the grid measured from it, rather than
    // estimating a width from the key and the count separately — which is how a
    // cell ends up two columns wider than anything in it.
    let cells: Vec<(String, bool)> = shown
        .iter()
        .map(|key| {
            let mut text = label_of(*key);
            if key.wrong() > 0 {
                text.push_str(&key.wrong().to_string());
            }
            (text, key.wrong() > 0)
        })
        .collect();
    let width = cells
        .iter()
        .map(|(text, _)| text.chars().count())
        .max()
        .unwrap_or(1)
        .max(1)
        + 1;
    let per_line = (area.width as usize / width).max(1);

    let mut lines: Vec<Line> = Vec::new();
    for row in cells.chunks(per_line) {
        let mut spans: Vec<Span> = Vec::new();
        for (text, wrong) in row {
            spans.push(Span::styled(
                format!("{text:<width$}"),
                if *wrong {
                    Style::default().fg(theme.incorrect)
                } else {
                    Style::default().fg(theme.muted)
                },
            ));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// How a key is written in the grid. The space gets a visible glyph, because a
/// literal one would be indistinguishable from the gap between cells.
fn label_of(key: KeyUse) -> String {
    if key.key == ' ' {
        "\u{2423}".to_owned()
    } else {
        key.key.to_string()
    }
}

fn render_footer(area: Rect, theme: Theme, frame: &mut Frame) {
    let line = Line::from(Span::styled(
        " ctrl+r again · esc back · ctrl+c quit ",
        theme.chrome(),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ch: char, pressed: u32, correct: u32) -> KeyUse {
        KeyUse {
            key: ch,
            pressed,
            correct,
        }
    }

    fn pad(text: &str, width: usize) -> String {
        format!("{text:<width$}")
    }

    /// The width of one cell in the key grid, given the cells on show.
    fn grid_cell(cells: &[(String, bool)]) -> usize {
        cells
            .iter()
            .map(|(text, _)| text.chars().count())
            .max()
            .unwrap_or(1)
            .max(1)
            + 1
    }

    /// The cell text for one key: the key, plus the count of wrong uses if it
    /// has any.
    fn cell_of(key: KeyUse) -> (String, bool) {
        let mut text = label_of(key);
        if key.wrong() > 0 {
            text.push_str(&key.wrong().to_string());
        }
        (text, key.wrong() > 0)
    }

    #[test]
    fn a_key_with_mistakes_needs_room_for_the_count() {
        let plain = [cell_of(key('a', 10, 10))];
        let wrong = [cell_of(key('a', 10, 9))];
        assert!(grid_cell(&wrong) > grid_cell(&plain));
    }

    #[test]
    fn a_cell_is_never_wider_than_its_contents_plus_a_gap() {
        // Measuring from the key and the count separately rather than from the
        // finished text is how a cell ends up padded two columns too wide.
        let cells = [cell_of(key('a', 10, 10)), cell_of(key('b', 5, 3))];
        let width = grid_cell(&cells);
        for (text, _) in &cells {
            let padded = pad(text, width);
            assert_eq!(padded.chars().count(), width, "cell for {text:?}");
            assert!(
                padded.starts_with(text.as_str()),
                "padding went to the wrong side: {padded:?}"
            );
        }
    }

    #[test]
    fn the_grid_fits_as_many_keys_per_row_as_it_can() {
        let cells: Vec<(String, bool)> = ('a'..='z').map(|c| cell_of(key(c, 1, 1))).collect();
        let width = grid_cell(&cells);
        for area_width in [1usize, 2, 5, 20, 80] {
            let per_line = (area_width / width).max(1);
            assert!(
                per_line * width <= area_width || area_width < width,
                "at width {area_width} a row of {per_line} cells of {width} does not fit"
            );
        }
    }

    #[test]
    fn the_space_gets_a_visible_label() {
        // A literal space in the grid would be indistinguishable from the gap
        // between cells.
        assert_eq!(label_of(key(' ', 3, 3)), "\u{2423}");
        assert_eq!(label_of(key('x', 3, 3)), "x");
    }
}
