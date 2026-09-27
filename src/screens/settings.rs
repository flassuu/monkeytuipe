//! Settings screen: the values the settings bar has no room for.
//!
//! The bar has a test's *shape* — what kind of test, how long, what it may
//! contain. This screen has the things that are either free text or a list far
//! too long to cycle: the word list, the custom passage, the ApeKey, the theme.
//! Anything the bar can do is not repeated here, because a setting in two places
//! is a setting that can disagree with itself.
//!
//! ## Three views, not one list
//!
//! A language is forty bases times five sizes, and a passage is a sentence. Both
//! do not fit in a row with a value on the right, so the screen has three states
//! and a key means something different in each. That is more code than a list of
//! rows, and less than a second screen would be: the user never loses the frame,
//! and `esc` always backs out of exactly one thing.
//!
//! ## Why the language browser shows sizes as a second column
//!
//! Upstream publishes `english_1k` through `english_25k` as separate ids with no
//! size-switching logic, so a client has to model the size itself. A picker that
//! listed all of them flat would be two hundred rows, nearly all of them the same
//! language. Two columns — bases down the side, sizes across the top — is the
//! shape the data actually has.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::config::bar;
use crate::config::theme::ThemeName;
use crate::screens::{Effect, Row, Screen, ScreenKind};
use crate::words::variants;

/// The rows, in display order. `Back` is last so `↑` from the top reaches it.
pub const ROWS: [Row; 7] = [
    Row::Theme,
    Row::Difficulty,
    Row::Language,
    Row::CustomText,
    Row::ApeKey,
    Row::SubmitResults,
    Row::Back,
];

/// What the screen is currently showing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum View {
    /// The row list.
    #[default]
    Rows,
    /// Choosing a word list: bases down the left, sizes across the top.
    Languages { base: usize, size: usize },
    /// Typing a value into a field.
    Editor { row: Row, text: String },
}

/// The settings screen's own state.
#[derive(Debug, Default)]
pub struct Settings {
    selected: usize,
    view: View,
}

impl Settings {
    pub fn selected_row(&self) -> Option<Row> {
        ROWS.get(self.selected).copied()
    }

    /// Which view is on screen, for the tests and for the app's key handling.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// Whether a printable key should be typed rather than acted on.
    ///
    /// Only in the editor. The row list is navigation, and stealing its keys to
    /// make a letter work somewhere else would be the wrong trade.
    pub fn wants_text(&self) -> bool {
        matches!(self.view, View::Editor { .. })
    }

    /// Opens the text editor for a row, seeded with the value the app holds.
    ///
    /// The text arrives from the app rather than being read here: this screen has
    /// no `App`, and the config is the app's. Reading it here would give the same
    /// value two owners, which is how a settings screen starts showing one thing
    /// and applying another.
    pub fn set_editor(&mut self, row: Row, text: String) {
        self.view = View::Editor { row, text };
    }
}

impl Screen for Settings {
    fn render(&self, app: &App, frame: &mut Frame) {
        let theme = app.theme();
        let area = block_of(app, frame);

        match &self.view {
            View::Rows => {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Fill(1), Constraint::Length(1)])
                    .split(area);
                frame.render_widget(Paragraph::new(row_items(app, self.selected)), rows[0]);
                let _ = &theme;
                // The account line is here because the ApeKey is here: the two
                // are the same setting and the same question.
                if rows[0].height > ROWS.len() as u16 + 1 {
                    let account =
                        Rect::new(rows[0].x, rows[0].y + rows[0].height - 1, rows[0].width, 1);
                    frame.render_widget(
                        Paragraph::new(super::results::account_line(app, theme)),
                        account,
                    );
                }
                frame.render_widget(
                    Paragraph::new(hint(&theme, "←→ change · enter open · ↑↓ move · esc back")),
                    rows[1],
                );
            }
            View::Languages { base, size } => {
                frame.render_widget(Paragraph::new(language_items(app, *base, *size)), area);
            }
            View::Editor { row, text } => {
                let lines = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(3),
                        Constraint::Length(1),
                    ])
                    .split(area);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!(" {} ", editor_title(*row)),
                            Style::default()
                                .fg(theme.accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("   "),
                        Span::styled(editor_help(*row), Style::default().fg(theme.muted)),
                    ])),
                    lines[0],
                );
                // No inner box: the screen already has one, and a box inside a
                // box reads as a dialog inside a dialog.
                frame.render_widget(
                    Paragraph::new(text.as_str())
                        .wrap(Wrap { trim: false })
                        .style(theme.value()),
                    lines[1],
                );
                frame.render_widget(
                    Paragraph::new(hint(&theme, "enter save · esc cancel")),
                    lines[2],
                );
            }
        }
    }

    fn handle(&mut self, action: Action) -> Vec<Effect> {
        match self.view.clone() {
            View::Rows => self.handle_rows(action),
            View::Languages { base, size } => self.handle_languages(action, base, size),
            View::Editor { row, text } => self.handle_editor(action, row, text),
        }
    }
}

impl Settings {
    fn handle_rows(&mut self, action: Action) -> Vec<Effect> {
        let last = ROWS.len() - 1;
        let row = self.selected_row().unwrap_or(Row::Back);
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
            Action::Left | Action::Right => {
                let by = if matches!(action, Action::Left) {
                    -1
                } else {
                    1
                };
                match row {
                    // Theme and difficulty are short ordered lists, so a step is the
                    // right thing to do with left and right — in the direction that
                    // was pressed, which is the whole point of having two arrows.
                    Row::Theme | Row::Difficulty => vec![Effect::Adjust(row, by)],
                    // The others are not "one step from the last": they are a list or
                    // a sentence, and stepping through a list of two hundred by
                    // pressing right is a way of never choosing one.
                    Row::Language => {
                        self.view = View::Languages { base: 0, size: 0 };
                        Vec::new()
                    }
                    Row::CustomText | Row::ApeKey => vec![Effect::OpenEditor(row)],
                    // The submit toggle says why it cannot submit; it is not a switch
                    // that does nothing.
                    Row::SubmitResults => vec![Effect::ShowMessage(
                        crate::api::submission::Destination::Monkeytype
                            .describe()
                            .to_owned(),
                    )],
                    Row::Back => Vec::new(),
                    _ => Vec::new(),
                }
            }
            Action::Select => self.open(row),
            Action::Back | Action::Settings => vec![Effect::Switch(ScreenKind::Typing)],
            _ => Vec::new(),
        }
    }

    /// Enter opens a row; for a toggle it toggles, for anything with a view it
    /// shows the view.
    fn open(&mut self, row: Row) -> Vec<Effect> {
        match row {
            Row::Back => vec![Effect::Switch(ScreenKind::Typing)],
            Row::Theme => vec![Effect::Adjust(row, 1)],
            Row::SubmitResults => vec![Effect::ShowMessage(
                crate::api::submission::Destination::Monkeytype
                    .describe()
                    .to_owned(),
            )],
            Row::Language => {
                self.view = View::Languages { base: 0, size: 0 };
                Vec::new()
            }
            Row::CustomText | Row::ApeKey => vec![Effect::OpenEditor(row)],
            _ => vec![Effect::Adjust(row, 1)],
        }
    }

    fn handle_languages(&mut self, action: Action, base: usize, size: usize) -> Vec<Effect> {
        let bases = variants::POPULAR_BASES.len();
        let sizes = bar::variants_sizes.len();
        match action {
            Action::Quit => vec![Effect::Quit],
            Action::Up => {
                self.view = View::Languages {
                    base: base.saturating_sub(1),
                    size,
                };
                Vec::new()
            }
            Action::Down => {
                self.view = View::Languages {
                    base: (base + 1) % bases,
                    size,
                };
                Vec::new()
            }
            Action::Left => {
                self.view = View::Languages {
                    base,
                    size: size.saturating_sub(1),
                };
                Vec::new()
            }
            Action::Right => {
                self.view = View::Languages {
                    base,
                    size: (size + 1) % sizes,
                };
                Vec::new()
            }
            Action::Select | Action::StartTest | Action::Restart => {
                let language = variants::variants(variants::POPULAR_BASES[base])[size].clone();
                self.view = View::Rows;
                vec![Effect::SetLanguage(language)]
            }
            Action::Back => {
                self.view = View::Rows;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn handle_editor(&mut self, action: Action, row: Row, mut text: String) -> Vec<Effect> {
        match action {
            Action::Quit => vec![Effect::Quit],
            // Text first, so `esc` on its own does not save by accident.
            Action::Char(c) => {
                if text.chars().count() < max_len(row) {
                    text.push(c);
                }
                self.view = View::Editor { row, text };
                Vec::new()
            }
            Action::Backspace => {
                text.pop();
                self.view = View::Editor { row, text };
                Vec::new()
            }
            Action::Select => {
                self.view = View::Rows;
                vec![match row {
                    Row::ApeKey => Effect::SetApeKey(text.trim().to_owned()),
                    _ => Effect::SetCustomText(text),
                }]
            }
            Action::Back => {
                self.view = View::Rows;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// A field the editor will not let grow without bound.
///
/// Both fields end up in a TOML file and, for the ApeKey, in a header. A
/// megabyte-long line in either is a mistake, and the limit is here rather than
/// at the point of saving so the user finds out while typing.
fn max_len(row: Row) -> usize {
    match row {
        // An ApeKey is 24 bytes, base64url-encoded.
        Row::ApeKey => 64,
        // A passage is a paragraph; the website's own custom text is a few
        // hundred characters and its quote files top out around 500.
        _ => 4_000,
    }
}

fn editor_title(row: Row) -> &'static str {
    match row {
        Row::ApeKey => "ape key",
        _ => "custom text",
    }
}

/// What the editor is for, said where the user is looking.
fn editor_help(row: Row) -> String {
    match row {
        Row::ApeKey => {
            "reads: profile and personal bests. submissions need a browser login.".to_owned()
        }
        _ => "one passage per line, typed in full. the first line is the test.".to_owned(),
    }
}

fn block_of(app: &App, frame: &mut Frame) -> Rect {
    let theme = app.theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" settings ", theme.heading()))
        .style(theme.base());
    let area = block.inner(frame.area());
    frame.render_widget(block, frame.area());
    area
}

/// The row list, with the selected row marked.
fn row_items(app: &App, selected: usize) -> Vec<Line<'static>> {
    let theme = app.theme();
    let items: Vec<Line> = ROWS
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let (name, value) = row_value(app, *row);
            let style = if index == selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            Line::from(vec![
                Span::styled(format!("{name}  "), style),
                Span::styled(value, Style::default().fg(theme.muted)),
            ])
        })
        .collect();
    items
}

/// A row's name and its current value.
fn row_value(app: &App, row: Row) -> (&'static str, String) {
    match row {
        Row::Theme => (
            "theme",
            match app.config.theme {
                // `auto` is not a theme, it is a decision, so it says what it
                // decided rather than what it is.
                // `auto` is a decision, not a theme, so it says what it decided.
                ThemeName::Auto => {
                    format!("auto ({})", ThemeName::auto_for(app.terminal()).label())
                }
                other => other.label().to_owned(),
            },
        ),
        Row::Difficulty => ("difficulty", app.config.test.difficulty.label().to_owned()),
        Row::Language => ("language", app.config.test.language.clone()),
        Row::CustomText => (
            "custom text",
            match app.config.test.custom_text.first() {
                Some(text) => {
                    let words = text.split(' ').filter(|w| !w.is_empty()).count();
                    let lines = app.config.test.custom_text.len();
                    format!("{words} words{}", extra_lines(lines))
                }
                None => "not set".to_owned(),
            },
        ),
        Row::ApeKey => (
            "ape key",
            match app.config.resolved_ape_key() {
                Some(key) if key.len() > 8 => format!("set · {}…", &key[..8]),
                Some(_) => "set".to_owned(),
                None => "not set".to_owned(),
            },
        ),
        Row::SubmitResults => ("submit results", "not possible — see below".to_owned()),
        Row::Back => ("back to typing", String::new()),
        _ => (row_label(row), String::new()),
    }
}

fn extra_lines(lines: usize) -> String {
    if lines > 1 {
        format!(" + {} more", lines - 1)
    } else {
        String::new()
    }
}

fn row_label(row: Row) -> &'static str {
    match row {
        Row::Theme => "theme",
        Row::Language => "language",
        Row::CustomText => "custom text",
        Row::ApeKey => "ape key",
        Row::SubmitResults => "submit results",
        Row::Punctuation => "punctuation",
        Row::Numbers => "numbers",
        Row::Mode => "mode",
        Row::Difficulty => "difficulty",
        Row::QuoteLength => "quote length",
        Row::Blind => "blind",
        Row::Back => "back to typing",
    }
}

/// The language browser: bases down the left, sizes across the top.
///
/// The sizes are labelled with what they are — `base`, `1k`, `5k` — because a
/// column of bare `english english_1k english_5k` is not a grid, it is a list
/// with the interesting part repeated.
fn language_items(app: &App, selected_base: usize, selected_size: usize) -> Vec<Line<'static>> {
    let theme = app.theme();
    let cached = |id: &str| crate::words::fetch::is_available_offline(id);
    let current = app.config.test.language.clone();
    let mut lines: Vec<Line> = Vec::new();

    // The header row: the sizes, in the order the bar cycles them.
    let mut header = vec![Span::raw(format!("{:<20}", "language"))];
    for size in &bar::variants_sizes {
        header.push(Span::styled(
            format!(" {:>5}", size_label(*size)),
            Style::default().fg(theme.muted),
        ));
    }
    lines.push(Line::from(header));

    for (index, base) in variants::POPULAR_BASES.iter().enumerate() {
        let mut row = vec![Span::styled(
            format!("{base:<20}", base = base.replace('_', " ")),
            if index == selected_base {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            },
        )];
        for (size_index, size) in bar::variants_sizes.iter().enumerate() {
            let id = if *size == 0 {
                base.to_string()
            } else {
                format!("{base}_{size}k")
            };
            let here = id == current;
            let style = if here {
                Style::default()
                    .fg(theme.extra)
                    .add_modifier(Modifier::BOLD)
            } else if index == selected_base && size_index == selected_size {
                Style::default().fg(theme.accent)
            } else if variants::embedded(&id) {
                // Already in the binary, so picking it works with no network.
                Style::default().fg(theme.correct)
            } else if cached(&id) {
                Style::default().fg(theme.muted)
            } else {
                // Not embedded and not cached: it will be downloaded.
                Style::default().fg(theme.muted)
            };
            row.push(Span::styled(
                format!(" {:>5}", if here { "●" } else { " " }),
                style,
            ));
        }
        lines.push(Line::from(row));
    }
    lines.push(Line::from(Span::styled(
        "  ←→ size · ↑↓ language · enter choose · esc back",
        theme.chrome(),
    )));
    lines
}

fn size_label(size: u32) -> String {
    if size == 0 {
        "base".to_owned()
    } else {
        format!("{size}k")
    }
}

fn hint(theme: &crate::config::theme::Theme, text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {text} "),
        Style::default().fg(theme.muted),
    ))
}

/// The rows the bar already covers, which this screen deliberately omits.
///
/// Not dead: the test on it is the thing that keeps a duplicate from being
/// added back, and a duplicate is how a setting ends up showing one value and
/// applying another.
///
/// `Difficulty` is deliberately *not* in this list, and that is the site's
/// arrangement rather than an oversight: the test-screen bar has punctuation,
/// numbers, the mode and the length, and difficulty lives in the settings panel
/// — here, on this screen.
#[cfg(test)]
const NOT_HERE: [Row; 5] = [
    Row::Punctuation,
    Row::Numbers,
    Row::Mode,
    Row::QuoteLength,
    Row::Blind,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    use crate::config::Config;

    fn app() -> App {
        App::new(Config::default(), PathBuf::from("/nonexistent/config.toml"))
    }

    /// A row's current value, read out of the app, for a test about direction.
    /// Walks the *app's* selection to `row`, through the real key path.
    fn walk_app(app: &mut App, row: Row) {
        for _ in 0..ROWS.len() {
            if app.selected_row() == Some(row) {
                return;
            }
            app.press(KeyCode::Down);
        }
        panic!(
            "could not walk to {row:?}; stopped on {:?}",
            app.selected_row()
        );
    }

    /// A row's current value, read out of the app, for a test about direction.
    fn value_of(app: &App, row: Row) -> String {
        match row {
            Row::Theme => app.config.theme.label().to_owned(),
            Row::Difficulty => app.config.test.difficulty.label().to_owned(),
            other => panic!("{other:?} is not a value a direction applies to"),
        }
    }

    fn draw(settings: &Settings, app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a terminal");
        terminal
            .draw(|frame| settings.render(app, frame))
            .expect("draws");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Drives the screen the way the app does: keys in, and the effects applied
    /// back — including the one that seeds the editor, which is the app's job
    /// because the app owns the config.
    fn press(app: &mut App, settings: &mut Settings, actions: &[Action]) -> Vec<Effect> {
        let mut effects = Vec::new();
        for action in actions {
            for effect in settings.handle(action.clone()) {
                if let Effect::OpenEditor(row) = &effect {
                    let text = match row {
                        Row::CustomText => app.config.test.custom_text.join("\n"),
                        Row::ApeKey => app.config.ape_key.clone(),
                        _ => String::new(),
                    };
                    settings.set_editor(*row, text);
                }
                effects.push(effect);
            }
        }
        effects
    }

    /// Every action, so a test can drive the screen without knowing how keys map.
    fn keys_for(actions: &[Action]) -> Vec<Effect> {
        let mut app = app();
        let mut settings = Settings::default();
        press(&mut app, &mut settings, actions)
    }

    /// Drives a screen the user is actually looking at.
    fn press_on(app: &mut App, settings: &mut Settings, actions: &[Action]) -> Vec<Effect> {
        press(app, settings, actions)
    }

    /// The settings the bar owns must not also be here, or one of the two goes
    /// stale and the screen shows a value it does not apply.
    #[test]
    fn nothing_the_bar_owns_is_repeated_here() {
        for row in NOT_HERE {
            assert!(
                !ROWS.contains(&row),
                "{row:?} is in both the bar and the settings screen"
            );
        }
    }

    #[test]
    fn back_is_the_last_row() {
        assert_eq!(ROWS.last(), Some(&Row::Back));
    }

    #[test]
    fn up_from_the_top_reaches_back() {
        // Wrapping, so the last row is one key away from the first.
        let effects = keys_for(&[Action::Up]);
        assert!(effects.is_empty(), "up changed something: {effects:?}");
    }

    /// The second row by position rather than by name, because that is what a
    /// single `down` reaches — and the row *is* difficulty, which is where the
    /// website keeps it.
    #[test]
    fn down_from_the_top_moves_to_the_second_row() {
        let mut app = app();
        let mut settings = Settings::default();
        press_on(&mut app, &mut settings, &[Action::Down]);
        assert_eq!(settings.selected_row(), Some(ROWS[1]));
        assert_eq!(
            settings.selected_row(),
            Some(Row::Difficulty),
            "difficulty should be the second row"
        );
    }

    #[test]
    fn escape_leaves_the_screen() {
        let effects = keys_for(&[Action::Back]);
        assert_eq!(effects, vec![Effect::Switch(ScreenKind::Typing)]);
    }

    #[test]
    fn enter_on_the_theme_cycles_it() {
        let effects = keys_for(&[Action::Select]);
        assert_eq!(effects, vec![Effect::Adjust(Row::Theme, 1)]);
    }

    /// The bug this fixes: `left` and `right` were the same key. Both arrows went
    /// the same way, so a theme could be cycled forwards but not back, and a
    /// difficulty could not be walked in the direction the arrow pointed.
    /// On the top row, which is where `keys_for` starts. The two arrows produce
    /// two different effects, with opposite signs.
    #[test]
    fn the_two_arrows_step_in_opposite_directions() {
        let left = keys_for(&[Action::Left]);
        let right = keys_for(&[Action::Right]);
        assert_eq!(left, vec![Effect::Adjust(Row::Theme, -1)]);
        assert_eq!(right, vec![Effect::Adjust(Row::Theme, 1)]);
        assert_ne!(left, right, "the two arrows did the same thing");
    }

    /// And the direction survives all the way to the value, which is the part that
    /// matters: an effect that carries a sign nobody reads is the same bug one
    /// layer down.
    ///
    /// Driven through the app, not the screen, because the screen only produces
    /// effects. The value is the app's, and a test that stops at the effect cannot
    /// see a sign dropped on the floor.
    #[test]
    fn the_arrows_reach_the_value_in_their_own_directions() {
        for row in [Row::Theme, Row::Difficulty] {
            let mut app = app();
            app.show_screen(crate::screens::ScreenKind::Settings);
            walk_app(&mut app, row);

            let start = value_of(&app, row);
            app.press(KeyCode::Right);
            let after_right = value_of(&app, row);
            assert_ne!(after_right, start, "{row:?}: right did nothing");

            app.press(KeyCode::Left);
            assert_eq!(
                value_of(&app, row),
                start,
                "{row:?}: left did not undo right, so the two arrows are one key"
            );
        }
    }

    /// And each direction comes back round on its own.
    #[test]
    fn a_cycled_row_comes_back_round_in_both_directions() {
        let mut app = app();
        app.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut app, Row::Difficulty);
        let start = value_of(&app, Row::Difficulty);
        for _ in 0..crate::config::bar::DIFFICULTIES.len() {
            app.press(KeyCode::Right);
        }
        assert_eq!(
            value_of(&app, Row::Difficulty),
            start,
            "right does not come back round"
        );
        for _ in 0..crate::config::bar::DIFFICULTIES.len() {
            app.press(KeyCode::Left);
        }
        assert_eq!(
            value_of(&app, Row::Difficulty),
            start,
            "left does not come back round"
        );
    }

    /// A list of two hundred must not be stepped through with one key.
    #[test]
    fn the_language_row_opens_a_browser_rather_than_stepping() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::Language);
        press_on(&mut app, &mut settings, &[Action::Right]);
        assert!(
            matches!(settings.view(), View::Languages { .. }),
            "{:?}",
            settings.view()
        );
    }

    #[test]
    fn the_browser_chooses_a_language_and_closes() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::Language);
        settings.handle(Action::Select);
        assert!(matches!(settings.view(), View::Languages { .. }));
        settings.handle(Action::Down); // the second base
        settings.handle(Action::Right); // the 1k size
        let effects = settings.handle(Action::Select);
        // The second base is whatever the list says it is. Hard-coding `russian`
        // here meant the test broke the first time the list was reordered, and
        // said nothing about whether the browser picks the right one.
        let second = variants::POPULAR_BASES[1];
        assert_eq!(
            effects,
            vec![Effect::SetLanguage(format!("{second}_1k"))],
            "the browser picked the wrong language"
        );
        assert_eq!(settings.view(), &View::Rows, "the browser stayed open");
    }

    /// The bases are the offered ones, in order, and the walk wraps rather than
    /// stopping, so the last one reaches the first.
    #[test]
    fn the_browser_never_escapes_its_lists() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::Language);
        press_on(&mut app, &mut settings, &[Action::Select]);
        for _ in 0..200 {
            press_on(&mut app, &mut settings, &[Action::Up, Action::Right]);
        }
        if let View::Languages { base, size } = *settings.view() {
            assert!(base < variants::POPULAR_BASES.len());
            assert!(size < bar::variants_sizes.len());
        } else {
            panic!("the browser closed itself: {:?}", settings.view());
        }
    }

    #[test]
    fn escape_leaves_the_browser_without_choosing() {
        let mut app = app();
        let mut settings = Settings::default();
        // Opening a view is a walk to its row and a select, not a fixed number of
        // downs, so a new row above it does not quietly open the wrong thing.
        walk_to(&mut app, &mut settings, Row::Language);
        press_on(&mut app, &mut settings, &[Action::Select]);
        let effects = press_on(&mut app, &mut settings, &[Action::Back]);
        assert!(effects.is_empty(), "{effects:?}");
        assert_eq!(settings.view(), &View::Rows);
    }

    /// Typing in an editor is text, not navigation — which is the rule the typing
    /// screen follows and the one a settings screen is easiest to get wrong.
    #[test]
    fn typing_in_an_editor_fills_it() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::CustomText);
        press_on(&mut app, &mut settings, &[Action::Select]);
        for c in "hello".chars() {
            press_on(&mut app, &mut settings, &[Action::Char(c)]);
        }
        let View::Editor { row, text } = settings.view() else {
            panic!("not an editor: {:?}", settings.view());
        };
        assert_eq!(*row, Row::CustomText);
        assert_eq!(text, "hello");
    }

    /// Walks the selection to `row`.
    ///
    /// Counting downs breaks every time a row is added above, and the failure
    /// then looks like a bug in whatever the test was about. Walking by name is
    /// what a user does and it survives a new row. Bounded by the row count, so a
    /// row that cannot be reached fails the test instead of hanging it.
    fn walk_to(app: &mut App, settings: &mut Settings, row: Row) {
        for _ in 0..ROWS.len() {
            if settings.selected_row() == Some(row) {
                return;
            }
            press_on(app, settings, &[Action::Down]);
        }
        panic!(
            "could not walk to {row:?}; stopped on {:?}",
            settings.selected_row()
        );
    }

    /// Opens the editor on `row`.
    fn open_editor(app: &mut App, settings: &mut Settings, row: Row) {
        walk_to(app, settings, row);
        press_on(app, settings, &[Action::Select]);
    }

    #[test]
    fn backspace_in_an_editor_removes_one_character() {
        let mut app = app();
        let mut settings = Settings::default();
        open_editor(&mut app, &mut settings, Row::CustomText);
        for c in "hi!".chars() {
            press_on(&mut app, &mut settings, &[Action::Char(c)]);
        }
        press_on(&mut app, &mut settings, &[Action::Backspace]);
        let View::Editor { text, .. } = settings.view() else {
            panic!("not an editor");
        };
        assert_eq!(text, "hi");
    }

    #[test]
    fn an_editor_saves_on_enter_and_discards_on_escape() {
        let mut app = app();
        let mut settings = Settings::default();
        open_editor(&mut app, &mut settings, Row::CustomText);
        for c in "kept".chars() {
            press_on(&mut app, &mut settings, &[Action::Char(c)]);
        }
        let effects = press_on(&mut app, &mut settings, &[Action::Select]);
        assert_eq!(effects, vec![Effect::SetCustomText("kept".to_owned())]);
        assert_eq!(settings.view(), &View::Rows);

        open_editor(&mut app, &mut settings, Row::CustomText);
        for c in "thrown away".chars() {
            press_on(&mut app, &mut settings, &[Action::Char(c)]);
        }
        let effects = press_on(&mut app, &mut settings, &[Action::Back]);
        assert!(effects.is_empty(), "escape saved something: {effects:?}");
        assert_eq!(settings.view(), &View::Rows);
    }

    /// `h` is bound to `left`, so a keybind table shadows the letter. A screen
    /// collecting text has to say so, or no word containing an `h` can be typed.
    #[test]
    fn a_bound_letter_is_still_text_in_an_editor() {
        let mut app = app();
        let mut settings = Settings::default();
        open_editor(&mut app, &mut settings, Row::CustomText);
        for c in "a short passage".chars() {
            press_on(&mut app, &mut settings, &[Action::Char(c)]);
        }
        let View::Editor { text, .. } = settings.view() else {
            panic!("not an editor: {:?}", settings.view());
        };
        assert_eq!(text, "a short passage", "a bound letter was swallowed");
    }

    /// And on the row list the binding still wins, because there is no field to
    /// type into.
    #[test]
    fn a_bound_letter_still_navigates_on_the_row_list() {
        let mut app = app();
        let mut settings = Settings::default();
        assert!(!settings.wants_text());
        walk_to(&mut app, &mut settings, Row::Language);
        assert_eq!(settings.selected_row(), Some(Row::Language));
    }

    /// A megabyte in one config line is a mistake, and it should be found while
    /// typing rather than at the point of saving.
    #[test]
    fn an_editor_will_not_grow_without_bound() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::CustomText);
        press_on(&mut app, &mut settings, &[Action::Select]);
        for _ in 0..(max_len(Row::CustomText) + 100) {
            press_on(&mut app, &mut settings, &[Action::Char('x')]);
        }
        let View::Editor { text, .. } = settings.view() else {
            panic!("not an editor");
        };
        assert_eq!(text.chars().count(), max_len(Row::CustomText));
    }

    /// The submit row must say why it cannot submit rather than offering a
    /// switch that does nothing.
    #[test]
    fn the_submit_row_explains_itself_instead_of_toggling() {
        let mut app = app();
        let mut settings = Settings::default();
        // Walk down to the row rather than assigning, so the test breaks if the
        // row moves.
        walk_to(&mut app, &mut settings, Row::SubmitResults);
        let effects = press_on(&mut app, &mut settings, &[Action::Select]);
        let Effect::ShowMessage(message) = effects.first().expect("a message") else {
            panic!("{effects:?}");
        };
        assert!(message.contains("ApeKey"), "{message}");

        let text = draw(&settings, &app, 70, 12);
        assert!(text.contains("not possible"), "{text}");
    }

    #[test]
    fn the_row_list_shows_every_row() {
        let text = draw(&Settings::default(), &app(), 70, 14);
        for row in ROWS {
            assert!(
                text.contains(row_label(row)),
                "{} is missing: {text}",
                row_label(row)
            );
        }
    }

    #[test]
    fn the_theme_row_says_what_auto_decided() {
        let text = draw(&Settings::default(), &app(), 70, 14);
        assert!(text.contains("auto ("), "{text}");
    }

    #[test]
    fn the_browser_shows_the_size_grid_and_the_current_language() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::Language);
        press_on(&mut app, &mut settings, &[Action::Select]);
        let text = draw(&settings, &app, 90, 46);
        assert!(text.contains("base"), "no size header: {text}");
        assert!(text.contains("1k"), "no 1k column: {text}");
        assert!(text.contains("25k"), "no 25k column: {text}");
        assert!(text.contains("english"), "no english row: {text}");
        assert!(text.contains("russian"), "no russian row: {text}");
        // The language in use is marked rather than merely present.
        assert!(text.contains('●'), "nothing is marked as current: {text}");
    }

    #[test]
    fn the_editor_shows_what_it_is_editing() {
        let mut app = app();
        let mut settings = Settings::default();
        walk_to(&mut app, &mut settings, Row::CustomText);
        press_on(&mut app, &mut settings, &[Action::Select]);
        for c in "a passage".chars() {
            press_on(&mut app, &mut settings, &[Action::Char(c)]);
        }
        let text = draw(&settings, &app, 70, 14);
        assert!(text.contains("custom text"), "{text}");
        assert!(text.contains("a passage"), "{text}");
        assert!(text.contains("enter save"), "{text}");
    }

    #[test]
    fn every_view_renders_on_a_tiny_terminal() {
        for (width, height) in [(1u16, 1u16), (4, 3), (10, 5), (20, 8), (200, 60)] {
            let mut app = app();
            let mut settings = Settings::default();
            // Every row, opened, at every size. Three rows would have missed
            // whatever the fourth one does.
            for row in ROWS {
                walk_to(&mut app, &mut settings, row);
                press_on(&mut app, &mut settings, &[Action::Select]);
                let _ = draw(&settings, &app, width, height);
                let _ = press_on(&mut app, &mut settings, &[Action::Back]);
            }
            let _ = draw(&settings, &app, width, height);
        }
    }
}
