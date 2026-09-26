//! Typing screen: the main test view.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::config::theme::Theme;
use crate::engine::{Word, WordState};
use crate::screens::{Effect, Screen, ScreenKind};

#[derive(Debug, Default)]
pub struct Typing;

impl Screen for Typing {
    fn render(&self, app: &App, frame: &mut Frame) {
        let theme = app.theme();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" monkeytuipe ", theme.heading()))
            .style(theme.base());
        let area = block.inner(frame.area());
        frame.render_widget(block, frame.area());

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(area);

        render_words(app, frame, rows[0], theme);
        render_stats(app, frame, rows[1], theme);
    }

    fn handle(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::StartTest | Action::Restart => vec![Effect::RestartTest],
            Action::Settings => vec![Effect::Switch(ScreenKind::Settings)],
            Action::Quit => vec![Effect::Quit],
            Action::Char(c) => vec![Effect::Type(c)],
            Action::Backspace => vec![Effect::Backspace],
            Action::Skip => vec![Effect::SkipWord],
            // Navigation belongs to the settings screen; the words pane scrolls
            // itself, so these have nothing to do here.
            Action::Up
            | Action::Down
            | Action::Left
            | Action::Right
            | Action::Select
            | Action::Back => Vec::new(),
        }
    }
}

/// The word list, coloured per character, with the caret on the next character
/// to type.
///
/// When the words no longer fit, the view scrolls so the active word stays visible.
fn render_words(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let words = app.test().words();
    if words.is_empty() {
        return;
    }

    let visible = (area.height as usize).max(1);
    let first = app
        .scroll_offset(visible)
        .min(words.len().saturating_sub(visible));
    let active = app.cursor_word();

    let mut spans: Vec<Span> = Vec::new();
    for (index, word) in words.iter().enumerate().skip(first).take(visible) {
        if index > first {
            spans.push(Span::raw(" "));
        }
        if index == active {
            spans.extend(active_word(word, theme));
        } else {
            spans.push(Span::styled(
                word.text(),
                settled_style(word.state(), theme),
            ));
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
        area,
    );
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

fn render_stats(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let started = app.test().is_started();
    let line = Line::from(vec![
        Span::styled("wpm ", theme.chrome()),
        Span::styled(format!("{:.0}", app.wpm()), theme.value()),
        Span::styled("  acc ", theme.chrome()),
        Span::styled(format!("{:.0}%", app.accuracy()), theme.value()),
        Span::styled("  time ", theme.chrome()),
        Span::styled(app.countdown(), theme.value()),
        Span::styled(
            if started {
                "  ·  tab skip · ctrl+r restart · f2 settings"
            } else {
                "  ·  type to start · ctrl+c quit"
            },
            theme.chrome(),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}
