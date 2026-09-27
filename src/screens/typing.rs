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
use crate::screens::{topbar, Effect, Screen, ScreenKind};
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
        topbar::render(app, rows.bar, theme, frame);
        // The site's out-of-focus warning, which in a terminal is the one before a
        // test has started: the words are there, and something says that a key is
        // what starts them. The words are drawn dimmed rather than covered, so
        // they are still readable — a user about to type them should not be stopped
        // from reading ahead.
        let awaiting = !app.test().is_started();
        let below = render_words(app, frame, rows.words, theme);
        if awaiting {
            render_awaiting_key(app, frame, rows.words, below, theme);
        }
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
            // Enter presses the selected bar button and every change takes effect
            // immediately, so there is nothing to confirm.
            Action::Select => vec![Effect::PressBar],
            // Escape opens the command list, which is what it is bound to on the
            // site when `quickRestart` is off — the default.
            Action::Back | Action::Command => vec![Effect::OpenCommands],
            // Shift+Enter ends a zen test, which is the only way out of one: it
            // has no length, so there is nothing to run out of.
            Action::Finish => vec![Effect::FinishTest],
        }
    }
}

/// How many rows the top bar needs at this width.
///
/// One, or none: the bar is a single row on the site and a wrapped one is two
/// things to read rather than one. A terminal too narrow for it gets no bar,
/// which is a trade worth making — the words and the counters are the test.
///
/// Public so a test can ask the same question the screen does: a test that
/// assumed one row would be checking a layout the screen never draws.
pub fn bar_rows(app: &App, width: u16) -> u16 {
    if width == 0 {
        return 0;
    }
    app.bar().rows(width)
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
/// Draws the words, and returns the row just below the block it drew.
///
/// The hint that says a key starts the test goes on that row, which is the whole
/// reason this returns anything: a box over the words hides them, and a user about
/// to type them is allowed to read them.
///
/// Before the first keystroke the words need no special treatment. An untouched
/// word is already drawn in the muted colour, so the block reads as "nothing
/// matched yet" on its own, and the caret on the active word is still drawn —
/// which is what a user about to type is looking for.
fn render_words(app: &App, frame: &mut Frame, area: Rect, theme: Theme) -> u16 {
    let words = app.test().words();
    if words.is_empty() || area.width == 0 || area.height == 0 {
        return area.y;
    }

    // Blind mode shows only the word being typed, so the pane is one word wide
    // and the wrapping below has nothing to do. The words are all still there —
    // this is a screen setting, not an engine one.
    let shown: &[Word] = if app.is_blind() {
        words
            .get(app.cursor_word())
            .map_or(&[][..], std::slice::from_ref)
    } else {
        words
    };
    if shown.is_empty() {
        return area.y;
    }
    // Zen shows what was typed, not what was there to type: there is no target
    // and nothing to be wrong about. The wrapping has to measure the same thing
    // it draws, or the lines will not line up.
    let zen = app.test_mode() == crate::engine::Mode::Zen;
    let widths: Vec<usize> = shown
        .iter()
        .map(|w| {
            if zen {
                w.input().chars().count()
            } else {
                w.text().chars().count()
            }
        })
        .collect();

    let lines = wrap_words(&widths, area.width as usize);
    let height = area.height as usize;

    // Scroll so the active word's line is in the middle, and centre the block
    // when there are fewer lines than the pane is tall.
    // The active word's index within what is being shown, which in blind mode is
    // the only word there is.
    let active = app.cursor_word().saturating_sub(first_word(shown, words));
    let active_line = lines
        .iter()
        .position(|line| line.first <= active && active < line.first + line.count)
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
            let word = &shown[index];
            if offset > 0 {
                spans.push(Span::raw(" "));
            }
            if index == active {
                spans.extend(active_word(word, theme, zen));
            } else if zen {
                // A zen word is shown exactly as it was typed, in the plain text
                // colour. There is no correct and no incorrect here — the site
                // draws the whole line in `--text-color` and never marks an error.
                spans.push(Span::styled(
                    word.input(),
                    Style::default().fg(theme.foreground),
                ));
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
    // The row below the block, clamped to the pane. `padding` and `drawn` are
    // counts in rows, so the sum is where the last line of text ended.
    let used = padding + drawn.min(lines.len() - first_line);
    (area.y + u16::try_from(used).unwrap_or(u16::MAX)).min(area.y + area.height)
}

/// Which word of the full list the shown slice starts at.
///
/// The pane draws a contiguous run, so the mapping back to the engine's indices
/// is the run's first index — which is the active word's own index in blind mode.
fn first_word(shown: &[Word], all: &[Word]) -> usize {
    if shown.is_empty() || all.is_empty() {
        return 0;
    }
    all.iter()
        .position(|word| std::ptr::eq(word, &shown[0]))
        .unwrap_or(0)
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

/// The active word, character by character, with the caret on the next one.
///
/// The site's rules, from `test-ui.ts`:
///
/// - a character that was **not typed yet** is drawn in the plain text colour,
///   with no state class at all;
/// - one typed **correctly** is drawn in the correct colour;
/// - one typed **wrongly** is drawn in the error colour — and it shows the
///   *expected* character, not the one that was pressed. That is the site's
///   choice, and it is the right one: the point of a red character is that the
///   right one goes there instead, and showing what was actually pressed would
///   make a correctly-spelled word look wrong in a different way each time;
/// - one typed **past the end** of the word is an extra, and is drawn in the
///   error colour showing what was typed;
/// - a **space** is drawn as `_`, because a literal space is indistinguishable
///   from the gap between two words.
fn active_word(word: &Word, theme: Theme, zen: bool) -> Vec<Span<'static>> {
    let typed: Vec<char> = word.input().chars().collect();
    // In zen there is no target: the text being typed *is* the test, so the
    // characters drawn are the ones that came out and none of them can be wrong.
    let target: Vec<char> = if zen {
        Vec::new()
    } else {
        word.text().chars().collect()
    };
    let caret = typed.len();
    let width = target.len().max(typed.len());
    let mut spans = Vec::with_capacity(width + 1);

    for i in 0..width {
        let expected = target.get(i).copied();
        let actual = typed.get(i).copied();
        let (shown, style) = match (expected, actual) {
            // Zen: whatever came out, in the text colour, with nothing to compare
            // it against. The site draws the whole line the same way.
            (None, Some(actual)) if zen => (actual, Style::default().fg(theme.foreground)),
            // Typed, and it was the right character.
            (Some(expected), Some(actual)) if expected == actual => {
                (expected, Style::default().fg(theme.foreground))
            }
            // Typed, and it was not: show what should have been there.
            (Some(expected), Some(_)) => (expected, Style::default().fg(theme.incorrect)),
            // Typed past the end of the word: an extra, shown as typed.
            (None, Some(actual)) => (actual, Style::default().fg(theme.incorrect)),
            // Not typed yet.
            (Some(expected), None) => (expected, Style::default().fg(theme.muted)),
            (None, None) => (' ', Style::default().fg(theme.muted)),
        };
        let style = if i == caret {
            // The caret is an inverse block, so it needs the background swap.
            theme.caret()
        } else {
            style
        };
        // A literal space is indistinguishable from the gap between two words, so
        // the site draws it as an underscore.
        let glyph: String = if shown == ' ' {
            "_".to_owned()
        } else {
            shown.to_string()
        };
        spans.push(Span::styled(glyph, style));
    }

    if caret >= width {
        // The caret has run past the last character, which is where it sits once
        // a whole word is typed. Park it on a blank rather than let it vanish.
        spans.push(Span::styled(" ", theme.caret()));
    }

    spans
}

/// The line under the words that says a key starts the test.
///
/// The site has this as `OutOfFocusWarning` — "Click here or press any key to
/// focus" — but for a different reason: it appears when the *browser window* stops
/// having focus, because a click is how you get it back. A terminal has no window
/// to lose focus and no click to bring it back, so the same affordance earns its
/// keep somewhere else: before the first keystroke, where the words are on screen
/// and nothing says that typing is how they start.
///
/// It goes on the row *below* the words rather than over them. A box drawn over
/// the words hides them, and a user about to type them is allowed to read them;
/// the words also get a thin outline in a small terminal, where a three-row box
/// would be the only thing on screen. When the pane is full and there is no row
/// left, it is centred on the words and takes their place — which is the one
/// honest option, and better than a hint pushed off the bottom of the screen.
///
/// Shown only before the test starts. Once there is input there is no question to
/// answer, and an overlay over a test in progress is a thing in the way of the one
/// thing the screen is for.
///
/// It says *press any key* rather than "click here", because there is no click —
/// and *any* key, because that is true: every printable key starts the test, and
/// so does a space, which is what someone who has read the words will press first.
fn render_awaiting_key(app: &App, frame: &mut Frame, area: Rect, below: u16, theme: Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = if below < area.y + area.height {
        below
    } else {
        // No free row. Take a row from the words rather than falling off the screen.
        area.y + area.height.saturating_sub(1)
    };
    let at = Rect::new(area.x, line, area.width, 1);
    frame.render_widget(
        Paragraph::new(app.tr(crate::i18n::Key::PressAnyKey))
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center),
        at,
    );
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
    // Escape is the command list — the same binding the site has when
    // `quickRestart` is off, which is the default — so it is worth saying, or the
    // list is a thing that exists and nothing points at it.
    // Before the test starts the line under the words already says to type, so
    // saying it here too would be the same sentence twice on one screen.
    let hint = if app.test().is_started() {
        app.tr(crate::i18n::Key::StartedHints)
    } else {
        app.tr(crate::i18n::Key::WaitingHints)
    };
    let mut spans = Vec::new();
    for (label, value) in [
        (app.tr(crate::i18n::Key::Wpm), format!("{:.0}", app.wpm())),
        (
            app.tr(crate::i18n::Key::Acc),
            format!("{:.0}%", app.accuracy()),
        ),
        (app.tr(crate::i18n::Key::Time), app.countdown()),
    ] {
        spans.push(Span::styled(format!("{label} "), theme.chrome()));
        spans.push(Span::styled(value, theme.value()));
        spans.push(Span::raw("  "));
    }
    // The mode, on the same line as the counters, because that is where the eye
    // already is and the mode is the thing that decides what the next key does.
    spans.push(Span::styled("· ", theme.chrome()));
    spans.push(Span::styled(
        format!("[{}] ", app.mode().label()),
        theme.chrome(),
    ));
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
