//! Typing screen: the main test view.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::config::theme::Theme;
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
            // Everything else belongs to the engine, which is not wired up yet.
            _ => Vec::new(),
        }
    }
}

/// The word list, with the character under the caret drawn as a block.
///
/// When the words no longer fit, the view scrolls so the active word stays visible.
fn render_words(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let words = app.words();
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
            spans.extend(active_word(word.as_str(), theme));
        } else {
            let color = if index < active {
                theme.muted
            } else {
                theme.foreground
            };
            spans.push(Span::styled(word.as_str(), Style::default().fg(color)));
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
        area,
    );
}

/// The active word, with its first character as the block caret.
///
/// The caret is drawn inline rather than on a row of its own, which is what
/// monkeytype does and what keeps the caret attached to its character once the
/// line wraps.
fn active_word(word: &str, theme: Theme) -> Vec<Span<'static>> {
    let mut chars = word.chars();
    let mut spans = Vec::new();

    let head = chars.next();
    let tail: String = chars.collect();

    if let Some(head) = head {
        spans.push(Span::styled(head.to_string(), theme.caret()));
    }
    if !tail.is_empty() {
        spans.push(Span::styled(
            tail,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

fn render_stats(app: &App, frame: &mut Frame, area: Rect, theme: Theme) {
    let line = Line::from(vec![
        Span::styled("wpm ", theme.chrome()),
        Span::styled(app.wpm().to_string(), theme.value()),
        Span::styled("  acc ", theme.chrome()),
        Span::styled(format!("{:.0}%", app.accuracy()), theme.value()),
        Span::styled("  time ", theme.chrome()),
        Span::styled(format!("{:>3}s", app.remaining_secs()), theme.value()),
        Span::styled(
            "  t start · r restart · , settings · q quit",
            theme.chrome(),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}
