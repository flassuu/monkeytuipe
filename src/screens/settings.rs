//! Settings screen.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::screens::{Effect, Row, Screen, ScreenKind};

/// The rows, in display order.
pub const ROWS: [Row; 7] = [
    Row::Theme,
    Row::Language,
    Row::Punctuation,
    Row::Numbers,
    Row::ApeKey,
    Row::SubmitResults,
    Row::Back,
];

#[derive(Debug, Default)]
pub struct Settings {
    selected: usize,
}

impl Settings {
    pub fn selected_row(&self) -> Option<Row> {
        ROWS.get(self.selected).copied()
    }
}

impl Screen for Settings {
    fn render(&self, app: &App, frame: &mut Frame) {
        let theme = app.theme();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" settings ", theme.heading()))
            .style(theme.base());
        let area = block.inner(frame.area());
        frame.render_widget(block, frame.area());

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(2), Constraint::Fill(1)])
            .split(area);

        let items: Vec<ListItem> = ROWS
            .iter()
            .map(|row| ListItem::new(label(app, *row, theme.muted)))
            .collect();
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› ");
        let mut state = ListState::default();
        state.select(Some(self.selected.min(ROWS.len() - 1)));
        frame.render_stateful_widget(list, rows[0], &mut state);

        let hint = Line::from(Span::styled(
            "←→ change · space toggle · ↑↓ move · esc back",
            theme.chrome(),
        ));
        frame.render_widget(Paragraph::new(hint), rows[1]);
    }

    fn handle(&mut self, action: Action) -> Vec<Effect> {
        let last = ROWS.len() - 1;
        let selected_row = self.selected_row().unwrap_or(Row::Back);
        match action {
            Action::Quit => vec![Effect::Quit],
            Action::Up => {
                self.selected = if self.selected == 0 {
                    last
                } else {
                    self.selected - 1
                };
                Vec::new()
            }
            Action::Down => {
                self.selected = if self.selected == last {
                    0
                } else {
                    self.selected + 1
                };
                Vec::new()
            }
            Action::Left | Action::Right => vec![Effect::Adjust(selected_row)],
            Action::Select => vec![Effect::Toggle(selected_row)],
            Action::Back | Action::Settings => vec![Effect::Switch(ScreenKind::Typing)],
            _ => Vec::new(),
        }
    }
}

/// One settings row: the name on the left, the current value on the right.
fn label(app: &App, row: Row, muted: Color) -> Line<'static> {
    let name: &'static str = match row {
        Row::Theme => "theme",
        Row::Language => "language",
        Row::Punctuation => "punctuation",
        Row::Numbers => "numbers",
        Row::ApeKey => "ape key",
        Row::SubmitResults => "submit results",
        Row::Back => "back to typing",
    };
    let value: String = match row {
        Row::Theme => app.config.theme.label().to_owned(),
        Row::Language => app.config.test.language.clone(),
        Row::Punctuation => on_off(app.config.test.punctuation),
        Row::Numbers => on_off(app.config.test.numbers),
        Row::ApeKey => {
            if app.config.ape_key.is_empty() {
                "not set".to_owned()
            } else {
                "set".to_owned()
            }
        }
        Row::SubmitResults => on_off(app.config.submit_results),
        Row::Back => String::new(),
    };
    Line::from(vec![
        Span::raw(name),
        Span::raw("  "),
        Span::styled(value, Style::default().fg(muted)),
    ])
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_owned()
}
