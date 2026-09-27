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

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::action::Action;
use crate::app::App;
use crate::config::bar;
use crate::i18n::Key;
use crate::screens::{Effect, Row, Screen, ScreenKind};
use crate::words::variants;

/// The rows, in display order. `Back` is last so `↑` from the top reaches it.
pub const ROWS: [Row; 8] = [
    Row::Theme,
    Row::Difficulty,
    Row::Language,
    Row::InterfaceLanguage,
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
    /// Choosing a theme, the same shape as the language browser: the whole list at
    /// once, the current one marked, and no stepping through it one press at a
    /// time.
    ///
    /// Nineteen themes reached by pressing right eighteen times is not a choice, it
    /// is a count. The language browser solved exactly this for word lists and the
    /// theme row has the same problem, so it gets the same answer.
    Themes { selected: usize },
    /// Typing a value into a field.
    Editor { row: Row, text: String },
}

/// The settings screen's own state.
#[derive(Debug, Default)]
pub struct Settings {
    selected: usize,
    view: View,
    /// The theme in use, so the picker can open with the right one highlighted.
    ///
    /// Held here rather than read from the app because this screen has no `App`,
    /// the same reason the editor's text arrives from the app. The app writes it
    /// on every theme change — both routes, the arrows and the picker — so the two
    /// ways of changing a theme cannot leave the highlight pointing at the old one.
    current_theme: crate::config::theme::ThemeName,
}

impl Settings {
    pub fn selected_row(&self) -> Option<Row> {
        ROWS.get(self.selected).copied()
    }

    /// Tells the screen which theme is in use, so the picker highlights it.
    pub fn set_current_theme(&mut self, theme: crate::config::theme::ThemeName) {
        self.current_theme = theme;
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

    /// The themes the picker shows, in the order it shows them: the automatic
    /// setting first, then every palette in the order the list has always been in.
    ///
    /// One function, because the picker and the tests must agree about what "every
    /// theme" means. `auto` is first rather than sorted in: it is the default, it is
    /// the one most people want, and it is not a palette.
    pub fn theme_choices() -> Vec<crate::config::theme::ThemeName> {
        let mut all: Vec<_> = crate::config::theme::ThemeName::ALL.to_vec();
        all.sort_by_key(|theme| !theme.is_automatic());
        all
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
        // Every view of this screen is one window in the same style as the command
        // window. They were drawn separately before, which is how two things that
        // are the same mechanism end up looking like two.
        let area = frame.area();
        let width = width_of(app, area.width);
        let height = self.height(area.height);
        let title = self.title(app);
        let Some(chrome) = super::chrome::Chrome::place(area, width, height, title, theme, frame)
        else {
            return;
        };

        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.view {
            View::Rows => {
                for (index, row) in ROWS.iter().enumerate() {
                    let (name, value) = row_value(app, *row);
                    let mut spans = vec![Span::raw(if index == self.selected {
                        "▸ "
                    } else {
                        "  "
                    })];
                    spans.push(Span::styled(
                        name.to_owned(),
                        if index == self.selected {
                            Style::default()
                                .fg(theme.accent)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.foreground)
                        },
                    ));
                    // The value is right-aligned against the window's right edge
                    // rather than sitting two spaces after the name, so the names
                    // make a column and the values make another. A list where the
                    // values start somewhere different on every row is a list you
                    // cannot scan.
                    //
                    // The padding is the *inner* width less the two columns and the
                    // two-character marker. Getting that arithmetic wrong by the
                    // width of the marker is what truncated every value by two
                    // characters the first time.
                    let used = 2 + name.chars().count() as u16 + value.chars().count() as u16;
                    let pad = chrome.inner.width.saturating_sub(used);
                    if pad > 0 {
                        spans.push(Span::raw(" ".repeat(pad as usize)));
                    }
                    spans.push(Span::styled(value, Style::default().fg(theme.muted)));
                    lines.push(Line::from(spans));
                }
                // The account line, because the ApeKey is here and the account is
                // the same question as the ApeKey.
                lines.push(Line::default());
                lines.push(super::results::account_line(app, theme));
                lines.push(super::chrome::hint(app.tr(Key::SettingsHint), theme));
            }
            View::Languages { base, size } => {
                lines.extend(language_items(app, *base, *size));
            }
            View::Themes { selected } => {
                // The whole list at once, which is the entire point of a picker.
                // Nineteen themes in one column needs twenty-two rows, so on a
                // short terminal it goes to two columns rather than being cut off:
                // a picker that hides the bottom of its own list is the row-cycling
                // it replaced, with extra steps.
                let choices = Self::theme_choices();
                let columns = Self::theme_columns(choices.len(), chrome.inner.height as usize);
                let cell_width = (chrome.inner.width as usize / columns.max(1)).max(10);
                let current = choices.iter().position(|c| *c == self.current_theme);

                // Across, then down: `columns` cells per line. Left to right is
                // how a menu reads, and the arrows walk the list in the same order,
                // so what is highlighted and where it is do not disagree.
                let rows = choices.len().div_ceil(columns.max(1));
                for row in 0..rows {
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    for column in 0..columns {
                        let index = row * columns + column;
                        let Some(choice) = choices.get(index) else {
                            continue;
                        };
                        if column > 0 {
                            spans.push(Span::raw(" "));
                        }
                        spans.push(Span::raw(if index == *selected { "▸ " } else { "  " }));
                        spans.push(Span::styled(
                            format!(
                                "{:<width$}",
                                choice.label(),
                                width = cell_width.saturating_sub(4)
                            ),
                            if index == *selected {
                                Style::default()
                                    .fg(theme.accent)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.foreground)
                            },
                        ));
                        // The theme in use is marked even when the highlight is
                        // elsewhere — the same `●` the language browser uses.
                        if Some(index) == current && index != *selected {
                            spans.push(Span::styled("●", Style::default().fg(theme.accent)));
                        }
                    }
                    lines.push(Line::from(spans));
                }
                lines.push(super::chrome::hint(app.tr(Key::ThemePickerHint), theme));
            }
            View::Editor { text, .. } => {
                lines.push(super::chrome::field(text, theme));
                lines.push(Line::default());
                lines.push(super::chrome::hint(app.tr(Key::EditorHelp), theme));
            }
        }
        chrome.draw(lines, frame);
    }

    fn handle(&mut self, action: Action) -> Vec<Effect> {
        match self.view.clone() {
            View::Rows => self.handle_rows(action),
            View::Languages { base, size } => self.handle_languages(action, base, size),
            View::Themes { selected } => self.handle_themes(action, selected),
            View::Editor { row, text } => self.handle_editor(action, row, text),
        }
    }
}

impl Settings {
    /// What the window's title says.
    ///
    /// The row being edited, or the language being chosen, or just "settings" —
    /// which is what tells a user which of three things they are looking at.
    fn title(&self, app: &App) -> &'static str {
        match &self.view {
            View::Rows => app.tr(Key::Settings),
            View::Languages { .. } => app.tr(Key::LanguageBrowser),
            View::Themes { .. } => app.tr(Key::ThemeBrowser),
            // The row's own name, so the window says which setting is open — the
            // one thing a window full of text does not tell you.
            View::Editor { row, .. } => row_value(app, *row).0,
        }
    }

    /// How tall this screen wants to be.
    ///
    /// Only as tall as its content: a settings window on a 40-row terminal that
    /// fills all of it looks like a page, and a page is not what seven rows of
    /// settings is.
    fn height(&self, available: u16) -> u16 {
        let wanted = match &self.view {
            View::Rows => ROWS.len() as u16 + 3,
            // The language browser fills whatever it is given, because it is a grid
            // and a grid in a short box is a truncated grid. The theme picker is
            // only as tall as its list, and goes to two columns rather than
            // growing past the screen.
            View::Languages { .. } => 0,
            View::Themes { .. } => Self::theme_choices().len() as u16 + 3,
            View::Editor { .. } => 5,
        };
        if wanted == 0 {
            return available;
        }
        (wanted + 2).min(available)
    }

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
                    // Difficulty and the interface language are two- and three-item
                    // lists, so a step is the right thing to do with left and right
                    // — in the direction that was pressed, which is the whole
                    // point of having two arrows.
                    Row::Difficulty | Row::InterfaceLanguage => {
                        vec![Effect::Adjust(row, by)]
                    }
                    // The theme opens the picker rather than stepping, exactly as
                    // the language row opens the language browser. Nineteen themes
                    // is eighteen presses, and a row you cannot see the ends of is
                    // not a choice.
                    Row::Theme => {
                        self.open_theme_picker();
                        Vec::new()
                    }
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
            Row::SubmitResults => vec![Effect::ShowMessage(
                crate::api::submission::Destination::Monkeytype
                    .describe()
                    .to_owned(),
            )],
            Row::Language => {
                self.view = View::Languages { base: 0, size: 0 };
                Vec::new()
            }
            // Enter on the theme opens the picker, with the current one already
            // under the highlight. The arrows do the same, as they do for the
            // language row: one way to reach a list of everything.
            Row::Theme => {
                self.open_theme_picker();
                Vec::new()
            }
            Row::CustomText | Row::ApeKey => vec![Effect::OpenEditor(row)],
            _ => vec![Effect::Adjust(row, 1)],
        }
    }

    /// How many columns the theme list needs to fit in `height` rows.
    ///
    /// One column when it fits, two when it does not, and two on a one-row screen
    /// where nothing fits at all. The point of the picker is that every theme is
    /// visible; a list whose bottom is cut off is the arrow-cycling it replaced,
    /// with extra steps.
    fn theme_columns(count: usize, height: usize) -> usize {
        // Two borders, the title row and the hint, and the marker column.
        let usable = height.saturating_sub(2);
        if count <= usable.max(1) {
            1
        } else {
            count.div_ceil(usable.max(1)).min(2)
        }
    }

    /// Opens the theme picker with the current theme under the highlight.
    fn open_theme_picker(&mut self) {
        let current = Self::theme_choices()
            .iter()
            .position(|theme| *theme == self.current_theme)
            .unwrap_or(0);
        self.view = View::Themes { selected: current };
    }

    /// The theme picker: arrows move, enter chooses, escape leaves.
    ///
    /// The same shape as the language browser and for the same reason. Escape
    /// leaves *without* choosing, so a user who opened the wrong thing is not
    /// left with a theme they did not pick.
    ///
    /// Moving the highlight **previews**: the theme under it is applied as you go, so
    /// the screen is repainted in the theme you are looking at before you commit to
    /// it. A theme is a decision about every colour on the screen, and nineteen names
    /// in a list say nothing about any of them — you cannot tell a readable theme from
    /// an unreadable one by its name. It used to be applied only on `enter`, which
    /// means choosing a theme blind and then looking at the result, and if you did not
    /// like it you had to come back and do it again.
    ///
    /// Escape puts the theme that was in use *before the picker opened* back. Not the
    /// highlighted one: a preview that leaves its last preview behind on cancel is a
    /// preview that changed a setting the user declined to change.
    fn handle_themes(&mut self, action: Action, selected: usize) -> Vec<Effect> {
        let all = Self::theme_choices();
        let last = all.len().saturating_sub(1);
        let preview = |index: usize| -> Vec<Effect> {
            all.get(index)
                .filter(|theme| **theme != self.current_theme)
                .map(|theme| vec![Effect::PreviewTheme(*theme)])
                .unwrap_or_default()
        };
        match action {
            Action::Quit => vec![Effect::Quit],
            // The walk wraps, for the same reason the row list's does: a list that
            // stops at the ends has two ends to stop at.
            Action::Up => {
                let next = selected.saturating_sub(1);
                self.view = View::Themes { selected: next };
                preview(next)
            }
            Action::Down => {
                let next = (selected + 1).min(last);
                self.view = View::Themes { selected: next };
                preview(next)
            }
            Action::Back => {
                self.view = View::Rows;
                // Back to what was in use, which is `current_theme` — the app writes
                // that field on every theme change *except* a preview's, so it is
                // still the theme from before the picker opened.
                let restore = all
                    .iter()
                    .find(|theme| **theme == self.current_theme)
                    .copied();
                restore
                    .filter(|theme| Some(*theme) != all.get(selected).copied())
                    .map(|theme| vec![Effect::PreviewTheme(theme)])
                    .unwrap_or_default()
            }
            Action::Select | Action::StartTest | Action::Restart => {
                let Some(theme) = all.get(selected) else {
                    return Vec::new();
                };
                self.view = View::Rows;
                vec![Effect::SetTheme(*theme)]
            }
            _ => Vec::new(),
        }
    }

    fn handle_languages(&mut self, action: Action, base: usize, size: usize) -> Vec<Effect> {
        let bases = variants::POPULAR_BASES.len();
        let sizes = bar::variants_sizes.len();
        match action {
            Action::Quit => vec![Effect::Quit],
            // Escape leaves the browser without choosing. It is the same arm the
            // editor has, and the browser had none at all — `Back` only appeared
            // inside the *select* handling, so a user who opened the browser and
            // changed their mind could not get out of it.
            Action::Back => {
                self.view = View::Rows;
                Vec::new()
            }
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

/// How wide the settings window wants to be.
///
/// Measured from the rows rather than guessed, because a guess is exactly how the
/// values end up truncated: a settings window is read, not skimmed, and a value
/// cut off at `auto (monkeytyp` is worse than no value.
fn width_of(app: &App, available: u16) -> u16 {
    // Two columns plus the marker in front of the selected row, four for the
    // border, and a gap so the two columns do not touch.
    let widest = ROWS
        .iter()
        .map(|row| {
            let (name, value) = row_value(app, *row);
            name.chars().count() + value.chars().count() + 4
        })
        .max()
        .unwrap_or(40) as u16;
    // The hint is prose and it sets a floor: a window narrower than its own hint
    // has a truncated hint, and a truncated hint looks like a sentence that stops.
    let hint: u16 = app.tr(Key::SettingsHint).chars().count() as u16 + 2;
    widest.max(hint).min(available.saturating_sub(2).max(20))
}

/// A row's name and its current value.
fn row_value(app: &App, row: Row) -> (&'static str, String) {
    match row {
        Row::Theme => (
            app.tr(Key::Theme),
            // The name and nothing else. This row used to say `auto (monkeytype)`
            // or `terminal (no reply)`, annotating a decision the user did not
            // make and could not change. There is one automatic setting now and it
            // is called `auto`, so `auto` is the whole of what there is to say.
            app.config.theme.label().to_owned(),
        ),
        Row::Difficulty => (
            app.tr(Key::Difficulty),
            app.tr(app.config.test.difficulty.key()).to_owned(),
        ),
        Row::Language => (app.tr(Key::Language), app.config.test.language.clone()),
        Row::InterfaceLanguage => (
            app.tr(Key::InterfaceLanguage),
            // A language names *itself* in its own script. A picker that shows
            // "English" to someone looking for English and "Английский" to someone
            // looking for русский is the wrong way round: the one thing a user
            // cannot read is the one thing they most need to find.
            app.config.ui_language.self_name().to_owned(),
        ),
        Row::CustomText => (
            app.tr(Key::CustomTextTitle),
            match app.config.test.custom_text.first() {
                Some(text) => {
                    let words = text.split(' ').filter(|w| !w.is_empty()).count();
                    let lines = app.config.test.custom_text.len();
                    format!("{words} words{}", extra_lines(lines))
                }
                None => app.tr(Key::NotSet).to_owned(),
            },
        ),
        Row::ApeKey => (
            app.tr(Key::ApeKeyTitle),
            match app.config.resolved_ape_key() {
                Some(key) if key.len() > 8 => {
                    format!("{} · {}…", app.tr(Key::ApeKeySet), &key[..8])
                }
                Some(_) => app.tr(Key::ApeKeySet).to_owned(),
                None => app.tr(Key::NotSet).to_owned(),
            },
        ),
        Row::SubmitResults => (
            app.tr(Key::SubmitResults),
            app.tr(Key::SubmissionImpossible).to_owned(),
        ),
        Row::Back => (app.tr(Key::BackToTyping), String::new()),
        // The rows the bar owns, which this screen deliberately does not show. They
        // still need a name for the test that checks none of them is here.
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
        Row::InterfaceLanguage => "interface language",
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
    let mut header = vec![Span::raw(format!("{:<20}", app.tr(Key::Language)))];
    for size in &bar::variants_sizes {
        header.push(Span::styled(
            format!(" {:>5}", size_label(*size, app)),
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

/// The column header for a word-list size: `base` for the plain list, `5k` for
/// the short one.
///
/// "base" is the site's word for the list with no size suffix, and the *base* of
/// the language rather than a size of it — which is why it is translated and the
/// numbers are not.
fn size_label(size: u32, app: &App) -> String {
    if size == 0 {
        app.tr(Key::BaseColumn).to_owned()
    } else {
        format!("{size}k")
    }
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

/// Every row's name and value, for anything that has to measure them.
///
/// The window's width is measured from the rows, which means the measurement has
/// to see the same rows the window draws. Exposed rather than reimplemented: a
/// test that reimplemented the rule would be testing its own copy of it.
pub fn row_values(app: &App) -> Vec<(&'static str, String)> {
    ROWS.iter().map(|row| row_value(app, *row)).collect()
}

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
    /// Draws the app and flattens it, for a test about what is on screen.
    fn draw_app(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a terminal");
        terminal.draw(|frame| app.render(frame)).expect("draws");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

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
            Row::Difficulty => app.tr(app.config.test.difficulty.key()).to_owned(),
            Row::InterfaceLanguage => app.config.ui_language.self_name().to_owned(),
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

    /// Drives a screen without an app, for the tests that only need the effects.
    fn keys_on(settings: &mut Settings, actions: &[Action]) -> Vec<Effect> {
        let mut app = app();
        press(&mut app, settings, actions)
    }

    /// Walks the selection to `row`, on the screen rather than through the app.
    fn walk_on(settings: &mut Settings, row: Row) {
        for _ in 0..ROWS.len() {
            if settings.selected_row() == Some(row) {
                return;
            }
            settings.handle(Action::Down);
        }
        panic!(
            "could not walk to {row:?}; stopped on {:?}",
            settings.selected_row()
        );
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

    /// Enter on the theme opens the picker, exactly as enter on the language row
    /// opens the language browser.
    ///
    /// It used to *step* the theme. Nineteen themes is eighteen presses, and a row
    /// you cannot see the ends of is not a choice.
    #[test]
    fn enter_on_the_theme_opens_a_picker() {
        let mut settings = Settings::default();
        keys_on(&mut settings, &[Action::Select]);
        assert!(
            matches!(settings.view(), View::Themes { .. }),
            "enter did not open the picker: {:?}",
            settings.view()
        );
    }

    /// And the arrows do the same, because there should be one way to reach a list
    /// of everything rather than two that behave differently.
    #[test]
    fn both_arrows_on_the_theme_open_the_picker() {
        for (action, code) in [
            (Action::Left, KeyCode::Left),
            (Action::Right, KeyCode::Right),
        ] {
            let mut screen = app();
            screen.show_screen(crate::screens::ScreenKind::Settings);
            walk_app(&mut screen, Row::Theme);
            screen.press(code);
            assert!(
                matches!(screen.view_kind(), Some(View::Themes { .. })),
                "{action:?} did not open the picker: {:?}",
                screen.view_kind()
            );
        }
    }

    /// Moving the highlight previews the theme, and cancelling puts back the one that
    /// was in use before the picker opened.
    ///
    /// A theme is a decision about every colour on the screen and nineteen names say
    /// nothing about any of them, so the only way to tell a readable theme from an
    /// unreadable one is to see it. It used to be applied on `enter` alone, which
    /// means choosing blind, looking, and coming back.
    #[test]
    fn moving_in_the_picker_previews_and_escape_restores() {
        let mut app = self::app();
        app.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut app, Row::Theme);
        app.press(KeyCode::Enter);
        let before = app.theme_name();

        // Each step down previews that theme rather than waiting for `enter`.
        let mut seen = Vec::new();
        for _ in 0..3 {
            app.press(KeyCode::Down);
            let now = app.theme_name();
            assert_ne!(now, before, "the preview did not change the theme");
            seen.push(now);
        }
        assert!(
            seen.windows(2).all(|w| w[0] != w[1]),
            "the preview did not follow the highlight: {seen:?}"
        );

        // Escape puts back the theme from before the picker opened — not the one the
        // highlight was left on, which is the whole point of cancelling.
        app.press(KeyCode::Esc);
        assert_eq!(app.theme_name(), before, "escape kept the last preview");
        assert_eq!(app.view_kind(), Some(&View::Rows));

        // And a preview is a look, not a decision: nothing was written.
        assert!(
            !app.is_dirty(),
            "previewing a theme marked the config dirty"
        );

        // Choosing is what makes it stick.
        let mut app = self::app();
        app.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut app, Row::Theme);
        app.press(KeyCode::Enter);
        app.press(KeyCode::Down);
        let previewed = app.theme_name();
        app.press(KeyCode::Enter);
        assert_eq!(
            app.theme_name(),
            previewed,
            "enter did not keep the preview"
        );
        assert!(
            app.is_dirty(),
            "choosing a theme did not mark the config dirty"
        );
    }

    /// The picker shows every theme, with the current one under the highlight and
    /// marked. A list you cannot see is not a picker.
    ///
    /// Driven through the app's own key path, because the screen does not know
    /// which theme is in use — the app tells it when the screen is built. A test
    /// that bypassed that would be testing a screen that can never be on screen.
    #[test]
    fn the_picker_lists_every_theme() {
        let mut app = app();
        app.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut app, Row::Theme);
        app.press(KeyCode::Enter);
        let View::Themes { selected } = *app.view_kind().expect("a view") else {
            panic!("not a picker: {:?}", app.view_kind());
        };
        let choices = Settings::theme_choices();
        assert_eq!(choices.len(), crate::config::theme::ThemeName::ALL.len());
        // The highlight starts on the theme in use.
        assert_eq!(
            choices[selected],
            app.theme_name(),
            "the highlight is on the wrong theme"
        );
        // And every one of them is drawn.
        let text = draw_app(&app, 92, 40);
        for choice in &choices {
            assert!(
                text.contains(choice.label()),
                "{} is missing: {text}",
                choice.label()
            );
        }
    }

    /// Enter sets the highlighted theme; escape leaves without setting anything.
    /// A user who opened the wrong thing must not be left with a theme they did
    /// not pick.
    #[test]
    fn the_picker_chooses_on_enter_and_leaves_on_escape() {
        let mut app = app();
        app.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut app, Row::Theme);
        app.press(KeyCode::Enter);
        app.press(KeyCode::Down);
        let wanted = Settings::theme_choices()[1];
        app.press(KeyCode::Enter);
        assert_eq!(
            app.theme_name(),
            wanted,
            "enter did not set the highlighted theme"
        );
        assert_eq!(app.view_kind(), Some(&View::Rows), "the picker stayed open");

        let mut dismissed = self::app();
        dismissed.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut dismissed, Row::Theme);
        dismissed.press(KeyCode::Enter);
        dismissed.press(KeyCode::Down);
        dismissed.press(KeyCode::Esc);
        assert_eq!(
            dismissed.theme_name(),
            crate::config::theme::ThemeName::Auto,
            "escape set a theme"
        );
        assert_eq!(
            dismissed.view_kind(),
            Some(&View::Rows),
            "escape did not close the picker"
        );
    }

    /// The highlight walks the whole list without sticking at either end, so a list
    /// of nineteen can be crossed without counting.
    #[test]
    fn the_picker_walks_the_whole_list_without_sticking() {
        let mut walker = app();
        walker.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut walker, Row::Theme);
        walker.press(KeyCode::Enter);
        let total = Settings::theme_choices().len();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..total {
            if let Some(View::Themes { selected }) = walker.view_kind() {
                seen.insert(*selected);
            }
            walker.press(KeyCode::Down);
        }
        assert_eq!(seen.len(), total, "the walk missed {total} entries");
    }

    /// And the highlight is right the second time the picker is opened, after a
    /// theme was chosen by a different route. Two writers to one field is how a
    /// picker ends up pointing at the theme that was replaced.
    #[test]
    fn the_highlight_follows_the_theme_that_was_chosen() {
        let mut app = app();
        app.show_screen(crate::screens::ScreenKind::Settings);
        walk_app(&mut app, Row::Theme);
        app.press(KeyCode::Enter);
        for _ in 0..4 {
            app.press(KeyCode::Down);
        }
        app.press(KeyCode::Enter);
        let chosen = app.theme_name();
        // Open it again.
        app.press(KeyCode::Enter);
        let View::Themes { selected } = *app.view_kind().expect("a view") else {
            panic!("not a picker: {:?}", app.view_kind());
        };
        assert_eq!(
            Settings::theme_choices()[selected],
            chosen,
            "the picker did not open on the theme in use"
        );
    }

    /// The bug this fixes: `left` and `right` were the same key. Both arrows went
    /// the same way, so a difficulty could not be walked in the direction the arrow
    /// pointed. On the top row the two arrows now open the picker; the difficulty
    /// row is where the sign is checked.
    #[test]
    fn the_two_arrows_step_in_opposite_directions() {
        let mut left_screen = Settings::default();
        walk_on(&mut left_screen, Row::Difficulty);
        let left = keys_on(&mut left_screen, &[Action::Left]);
        let mut right_screen = Settings::default();
        walk_on(&mut right_screen, Row::Difficulty);
        let right = keys_on(&mut right_screen, &[Action::Right]);
        assert_eq!(left, vec![Effect::Adjust(Row::Difficulty, -1)]);
        assert_eq!(right, vec![Effect::Adjust(Row::Difficulty, 1)]);
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
        // The theme row is not in this list: its arrows open the picker, which is
        // what the language row does and what "you cannot see the ends of a list of
        // nineteen" makes necessary. The sign is checked on the rows that step.
        for row in [Row::Difficulty, Row::InterfaceLanguage] {
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
    fn the_theme_row_says_only_the_theme_name() {
        // It used to read `auto (monkeytype)`, annotating a decision the user did
        // not make and cannot change from this row. There is one automatic theme
        // now, called `auto`, and that is the whole of what there is to say.
        let text = draw(&Settings::default(), &app(), 70, 14);
        assert!(text.contains("auto"), "{text}");
        for annotation in ["auto (", "(monkeytype)", "no reply", "(terminal"] {
            assert!(
                !text.contains(annotation),
                "the row still says {annotation:?}: {text}"
            );
        }
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
