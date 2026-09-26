//! Typing screen: the main test view.
//!
//! Laid out the way the website is — words in the middle, live chart under
//! them, counters on one line along the bottom — because that arrangement is
//! what lets you read a test at a glance. A box drawn around everything and a
//! status bar in the corner made a thing that should feel like typing feel like
//! a program.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::config::theme::Theme;
use crate::engine::{Word, WordState};
use crate::screens::{config_bar, Effect, Screen, ScreenKind};
use crate::widgets;

/// How many rows the live chart takes on the typing screen.
///
/// Enough for the two series to be told apart. The results screen gives it
/// more, because by then there is nothing else competing for the eye.
const CHART_ROWS: u16 = 6;

/// The fewest rows the word pane is allowed to be squeezed down to.
const MIN_WORD_ROWS: u16 = 4;

/// The widest the chart is ever drawn, so it stays a chart rather than a
/// full-bleed texture on a wide terminal.
const CHART_WIDTH: u16 = 72;

/// Rows left blank between the words and the chart, and between the chart and
/// the counters, so the three bands do not run into each other.
const GAP: u16 = 1;

#[derive(Debug, Default)]
pub struct Typing;

/// The bands the typing screen is drawn in.
///
/// Exposed because "where are the words on screen" is a question the render
/// tests need answered, and hard-coding row arithmetic in two places is how a
/// layout change breaks tests that were not testing the layout at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypingRows {
    pub header: Rect,
    pub bar: Rect,
    pub words: Rect,
    pub chart: Rect,
    pub counters: Rect,
}

/// Splits a screen into the typing screen's bands.
///
/// `bar_rows` is how many rows the settings bar needs, which depends on how many
/// fields the current mode has and how wide the terminal is — so it is measured
/// from the cells rather than assumed. A bar given one row and needing two gets
/// truncated, and a truncated settings bar hides settings.
///
/// The words come first, because a test with no room for the words is a test you
/// cannot see and the chart is decoration. So the chart is drawn out of what is
/// left after the words have a usable window, and on a terminal too short to
/// hold both it is dropped entirely rather than squeezed — ratatui's own solver
/// would give the fixed-height rows priority and leave the words nothing.
///
/// The header and the counters always keep their row, because they are the only
/// place the status and the numbers live.
pub fn rows_for(area: Rect, bar_rows: u16) -> TypingRows {
    let bottom = area.y + area.height;

    let header = Rect::new(area.x, area.y, area.width, 1.min(area.height));
    let bar = Rect::new(
        area.x,
        area.y + header.height,
        area.width,
        bar_rows.min(area.height.saturating_sub(header.height)),
    );
    let cursor = area.y + header.height + bar.height;

    let counters = Rect::new(
        area.x,
        bottom.saturating_sub(1),
        area.width,
        1.min(area.height),
    );
    let free = bottom
        .saturating_sub(cursor)
        .saturating_sub(counters.height);

    let wants_chart = free >= MIN_WORD_ROWS + CHART_ROWS + 2 * GAP;
    let chart_height = if wants_chart { CHART_ROWS } else { 0 };
    let gaps = if wants_chart { 2 * GAP } else { 0 };
    let words_height = free.saturating_sub(chart_height + gaps);

    let words = Rect::new(area.x, cursor, area.width, words_height);
    let chart = Rect::new(
        area.x,
        words.y + words.height + GAP,
        area.width,
        chart_height,
    );

    TypingRows {
        header,
        bar,
        words,
        chart,
        counters,
    }
}

impl Screen for Typing {
    fn render(&self, app: &App, frame: &mut Frame) {
        let theme = app.theme();
        frame.render_widget(Paragraph::new("").style(theme.base()), frame.area());

        let rows = rows_for(frame.area(), bar_rows(app, frame.area().width));
        render_header(app, frame, rows.header, theme);
        config_bar::render(app, rows.bar, theme, frame);
        render_words(app, frame, rows.words, theme);
        render_chart(app, frame, rows.chart, theme);
        render_counters(app, frame, rows.counters, theme);
    }

    fn handle(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::StartTest | Action::Restart => vec![Effect::RestartTest],
            Action::Settings => vec![Effect::Switch(ScreenKind::Settings)],
            Action::Quit => vec![Effect::Quit],
            Action::Char(c) => vec![Effect::Type(c)],
            Action::Backspace => vec![Effect::Backspace],
            Action::Skip => vec![Effect::SkipWord],
            // The arrows drive the settings bar, which is always on screen, so
            // there is no mode to enter first. That is the trade the website
            // makes with a mouse click and a terminal makes with a key that was
            // doing nothing anyway.
            Action::Up => vec![Effect::ChangeBar(-1)],
            Action::Down => vec![Effect::ChangeBar(1)],
            Action::Left => vec![Effect::MoveBar(-1)],
            Action::Right => vec![Effect::MoveBar(1)],
            // Enter and Escape are the settings screen's; there is nothing to
            // confirm here because every change takes effect immediately.
            Action::Select | Action::Back => Vec::new(),
        }
    }
}

/// How many rows the settings bar needs at this width.
///
/// Public so a test can ask the same question the screen does: a test that
/// assumed one row would be checking a layout the screen never draws.
pub fn bar_rows(app: &App, width: u16) -> u16 {
    let cells = config_bar::cells(app);
    if cells.is_empty() || width == 0 {
        return 0;
    }
    (config_bar::layout(&cells, width as usize, u16::MAX as usize).len() as u16)
        .min(config_bar::ROWS)
}

/// The name on the left, whatever is downloading on the right.
///
/// The word list used to go here, but it is in the settings bar now, and a
/// language named twice in two rows is one named in the wrong place if the two
/// ever disagree.
fn render_header(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let left = Span::styled(" monkeytuipe ", theme.heading());
    let status = app
        .quote_status()
        .or_else(|| app.pending_language().map(|id| format!("{id} ↓")));
    let right = match status {
        Some(text) => Span::styled(format!(" {text} "), theme.chrome()),
        None => Span::raw(""),
    };
    let gap = (area.width as usize)
        .saturating_sub(1 + 13)
        .saturating_sub(right.content.chars().count());
    frame.render_widget(
        Paragraph::new(Line::from(vec![left, Span::raw(" ".repeat(gap)), right])),
        area,
    );
}

/// One line of the word pane: which words it holds and how wide it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WordLine {
    /// Index of the first word on the line.
    first: usize,
    /// How many words it holds.
    count: usize,
    /// Display width, used for the horizontal centring.
    width: usize,
}

/// Wraps a run of words into lines of at most `width` columns.
///
/// A word is never split, which is what the website does and what keeps the
/// caret attached to its character: if a word is wider than the whole pane it
/// gets a line to itself and overflows rather than being torn in half.
///
/// Wrapping is done here rather than left to ratatui because the pane has to
/// know where the active word *landed* — that is what decides which lines are
/// worth drawing — and a widget that wraps on its own gives no answer.
fn wrap_words(widths: &[usize], width: usize) -> Vec<WordLine> {
    if widths.is_empty() {
        return Vec::new();
    }
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut x = 0usize;

    for (index, word_width) in widths.iter().copied().enumerate() {
        let needed = if index == start { 0 } else { 1 } + word_width;
        if x + needed > width && index > start {
            lines.push(WordLine {
                first: start,
                count: index - start,
                width: x,
            });
            start = index;
            x = word_width;
        } else {
            x += needed;
        }
    }
    lines.push(WordLine {
        first: start,
        count: widths.len() - start,
        width: x,
    });
    lines
}

/// The word list, centred both ways, coloured per character, with the caret on
/// the next character to type.
///
/// The active line sits in the middle of the pane, and the block is centred in
/// it — the website's arrangement, and the reason a test reads as a thing being
/// typed rather than as a list being consumed from the top.
fn render_words(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let words = app.test().words();
    if words.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let lines = wrap_words(&app.word_widths(), area.width as usize);
    let height = area.height as usize;

    // Scroll so the active word's line is in the middle, and centre the block
    // when there are fewer lines than the pane is tall.
    let active_line = lines
        .iter()
        .position(|line| {
            let first = line.first;
            first <= app.cursor_word() && app.cursor_word() < first + line.count
        })
        .unwrap_or(0);
    let drawn = height.min(lines.len());
    let first_line = active_line.saturating_sub((drawn.saturating_sub(1)) / 2);
    let padding = (height - drawn.min(lines.len() - first_line)) / 2;

    let mut out: Vec<Line> = Vec::new();
    for _ in 0..padding {
        out.push(Line::default());
    }
    for line in &lines[first_line..(first_line + drawn).min(lines.len())] {
        let mut spans: Vec<Span> = Vec::new();
        for offset in 0..line.count {
            let index = line.first + offset;
            let word = &words[index];
            if offset > 0 {
                spans.push(Span::raw(" "));
            }
            if index == app.cursor_word() {
                spans.extend(active_word(word, theme));
            } else {
                spans.push(Span::styled(
                    word.text(),
                    settled_style(word.state(), theme),
                ));
            }
        }
        out.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(out).alignment(Alignment::Center), area);
}

/// The style of a word that is no longer being typed.
fn settled_style(state: WordState, theme: Theme) -> Style {
    match state {
        WordState::Correct => Style::default().fg(theme.correct),
        WordState::Incorrect => Style::default().fg(theme.incorrect),
        // Skipped words recede rather than being flagged as mistakes.
        WordState::Skipped | WordState::Untouched | WordState::Typing => {
            Style::default().fg(theme.muted)
        }
    }
}

/// The active word: every character coloured by whether it matched, with the
/// caret drawn inline on the next one.
///
/// The caret is inline rather than on a row of its own, which is what monkeytype
/// does and what keeps it attached to its character once the line wraps.
fn active_word(word: &Word, theme: Theme) -> Vec<Span<'static>> {
    let typed: Vec<char> = word.input().chars().collect();
    let target: Vec<char> = word.text().chars().collect();
    let caret = typed.len();
    let width = target.len().max(typed.len());
    let mut spans = Vec::with_capacity(width + 1);

    for i in 0..width {
        let style = if i == caret {
            // The caret is an inverse block, so it needs the background swap.
            theme.caret()
        } else if let Some(&actual) = typed.get(i) {
            match target.get(i) {
                Some(&expected) if actual == expected => Style::default().fg(theme.correct),
                _ => Style::default().fg(theme.incorrect),
            }
        } else {
            Style::default().fg(theme.muted)
        };
        // Show the target character where there is one, so the caret always sits
        // on what the typist is being asked for, and the typed character where
        // they have overrun the word.
        let shown = target
            .get(i)
            .or_else(|| typed.get(i))
            .copied()
            .unwrap_or(' ');
        spans.push(Span::styled(shown.to_string(), style));
    }

    if caret >= width {
        // The caret has run past the last character, which is where it sits once
        // a whole word is typed. Park it on a blank rather than let it vanish.
        spans.push(Span::styled(" ", theme.caret()));
    }

    spans
}

/// The live chart, centred and capped in width.
fn render_chart(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let chart = app.chart();
    if chart.is_empty() {
        return;
    }
    let width = CHART_WIDTH.min(area.width);
    let left = area.x + (area.width - width) / 2;
    let chart_area = Rect::new(left, area.y, width, area.height);
    widgets::render(&chart, chart_area, frame.buffer_mut(), theme);
}

/// The counters, all on one line and centred, the way the site shows them.
fn render_counters(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let hint = if app.test().is_started() {
        "tab skip · ctrl+r restart · f2 settings"
    } else {
        "type to start · ctrl+c quit"
    };

    let mut spans = Vec::new();
    for (label, value) in [
        ("wpm", format!("{:.0}", app.wpm())),
        ("acc", format!("{:.0}%", app.accuracy())),
        ("time", app.countdown()),
    ] {
        spans.push(Span::styled(format!("{label} "), theme.chrome()));
        spans.push(Span::styled(value, theme.value()));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(format!("·  {hint}"), theme.chrome()));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines `wrap_words` produced, as `(first word, word count, width)`.
    fn lines_of(widths: &[usize], width: usize) -> Vec<(usize, usize, usize)> {
        wrap_words(widths, width)
            .into_iter()
            .map(|line| (line.first, line.count, line.width))
            .collect()
    }

    #[test]
    fn words_that_fit_stay_on_one_line() {
        assert_eq!(
            lines_of(&[3, 5, 5, 3], 40),
            [(0, 4, 3 + 1 + 5 + 1 + 5 + 1 + 3)]
        );
    }

    #[test]
    fn a_line_breaks_before_it_overflows() {
        // 3 + 1 + 5 = 9; a third word would need 15, which does not fit in 14.
        assert_eq!(lines_of(&[3, 5, 5, 3], 14), [(0, 2, 9), (2, 2, 9)]);
        assert_eq!(lines_of(&[3, 5, 5, 3], 15), [(0, 3, 15), (3, 1, 3)]);
    }

    #[test]
    fn a_word_exactly_the_width_of_the_pane_still_fits() {
        assert_eq!(lines_of(&[10], 10), [(0, 1, 10)]);
        assert_eq!(lines_of(&[10, 3], 10), [(0, 1, 10), (1, 1, 3)]);
    }

    #[test]
    fn a_word_wider_than_the_pane_gets_a_line_to_itself() {
        // Tearing a word across lines would detach the caret from its character.
        assert_eq!(
            lines_of(&[2, 30, 2], 10),
            [(0, 1, 2), (1, 1, 30), (2, 1, 2)]
        );
    }

    #[test]
    fn every_word_lands_on_exactly_one_line() {
        let widths = [3, 5, 5, 3, 8, 1, 1, 1, 9, 2];
        for width in 1..=40usize {
            let lines = wrap_words(&widths, width);
            let mut seen: Vec<usize> = lines
                .iter()
                .flat_map(|l| l.first..l.first + l.count)
                .collect();
            seen.sort_unstable();
            assert_eq!(
                seen,
                (0..widths.len()).collect::<Vec<_>>(),
                "width {width} lost or duplicated a word"
            );
        }
    }

    #[test]
    fn the_lines_never_exceed_the_width_except_for_an_oversized_word() {
        let widths = [3, 5, 5, 3, 8, 1, 1, 1, 9, 2];
        for width in 1..=40usize {
            for line in wrap_words(&widths, width) {
                let oversized = line.count == 1 && widths[line.first] > width;
                assert!(
                    oversized || line.width <= width,
                    "width {width}: line {:?} overflows",
                    (line.first, line.count, line.width)
                );
            }
        }
    }

    #[test]
    fn no_words_makes_no_lines() {
        assert!(wrap_words(&[], 40).is_empty());
    }

    #[test]
    fn a_zero_width_pane_still_terminates() {
        // Width 0 would otherwise wrap after every word, forever.
        assert_eq!(lines_of(&[3, 5], 0).len(), 2);
    }
}
