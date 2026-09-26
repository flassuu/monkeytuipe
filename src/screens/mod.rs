//! Screens.
//!
//! A screen owns only its own view state and never touches [`App`] directly.
//! Instead it converts input into [`Effect`]s, which [`App`](crate::app::App)
//! applies. That keeps the data flow one-directional and lets a screen be
//! unit-tested without an `App`.

pub mod config_bar;
pub mod results;
pub mod settings;
pub mod typing;

use ratatui::Frame;

use crate::action::Action;
use crate::app::App;

/// Something a screen asks the app to do.
///
/// Not `Copy`: several of these carry a value the user typed or picked out of a
/// list, and a screen that could only hand over effects it still owned could not
/// hand over a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Leave the app.
    Quit,
    /// Move to a screen by kind.
    Switch(ScreenKind),
    /// Throw away the current test and start a fresh one.
    RestartTest,
    /// Feed one character to the engine.
    Type(char),
    /// Remove the last character typed.
    Backspace,
    /// Jump over the active word.
    SkipWord,
    /// Left/right on a settings row.
    Adjust(Row),
    /// Space on a settings row.
    Toggle(Row),
    /// Move the settings bar's selection left (`-1`) or right (`1`).
    MoveBar(i8),
    /// Change the selected bar field: up (`-1`) or down (`1`).
    ChangeBar(i8),
    /// Switch to a word list, fetching it if it is not in the binary.
    SetLanguage(String),
    /// Store an ApeKey the user typed.
    SetApeKey(String),
    /// Store a custom passage the user typed.
    SetCustomText(String),
    /// Open the text editor for a settings row, seeded with its current value.
    ///
    /// The text comes back with the effect rather than being read where the
    /// effect is made, because a screen has no `App` and the config is the app's.
    /// A screen that read the config itself would have two sources of truth for
    /// the same value, which is the way a settings screen starts lying.
    OpenEditor(Row),
    /// Say something on the status line and carry on.
    ShowMessage(String),
}

/// A screen identified by kind, for code that needs to name one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenKind {
    #[default]
    Typing,
    Settings,
    /// What the last test came to.
    Results,
}

impl ScreenKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Typing => "typing",
            Self::Settings => "settings",
            Self::Results => "results",
        }
    }
}

/// One editable settings row.
///
/// Each variant is handled explicitly in `App::apply` so that adding a setting
/// later cannot silently become a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Theme,
    Language,
    Punctuation,
    Numbers,
    ApeKey,
    SubmitResults,
    /// A passage the user typed, so it is not one of the bar's short choices.
    CustomText,
    /// Held by the settings bar, not by this screen. The variants exist because
    /// the bar's fields are rows too, and one enum is one place to check that a
    /// setting is not shown twice.
    Mode,
    Difficulty,
    QuoteLength,
    Blind,
    Back,
}

/// The active screen: its view state plus its behaviour.
pub trait Screen {
    /// Draws the screen.
    fn render(&self, app: &App, frame: &mut Frame);

    /// Applies one action and reports what the app should do.
    fn handle(&mut self, action: Action) -> Vec<Effect>;
}

/// Holds the active screen's state, so `App` never has to borrow itself twice.
#[derive(Debug)]
pub enum ScreenState {
    Typing(typing::Typing),
    Settings(settings::Settings),
    Results(results::Results),
}

impl Default for ScreenState {
    fn default() -> Self {
        Self::Typing(typing::Typing)
    }
}

impl ScreenState {
    pub fn kind(&self) -> ScreenKind {
        match self {
            Self::Typing(_) => ScreenKind::Typing,
            Self::Settings(_) => ScreenKind::Settings,
            Self::Results(_) => ScreenKind::Results,
        }
    }

    /// Seeds the text editor for `row` with the value the app holds.
    pub fn open_editor(&mut self, row: Row, text: String) {
        if let Self::Settings(state) = self {
            state.set_editor(row, text);
        }
    }

    /// Whether the screen has a text field that a printable key belongs to.
    ///
    /// The same rule as the typing screen, for the same reason: `h` is bound to
    /// `left`, so a keybind table shadows the letter `h` and there is no way to
    /// type a word containing it. A screen that is collecting text says so, and
    /// the resolution order inverts.
    pub fn wants_text(&self) -> bool {
        match self {
            Self::Typing(_) => true,
            Self::Settings(state) => state.wants_text(),
            Self::Results(_) => false,
        }
    }

    /// What the settings screen is showing, if it is the active screen.
    pub fn settings_view(&self) -> Option<&settings::View> {
        match self {
            Self::Settings(state) => Some(state.view()),
            _ => None,
        }
    }

    /// The row the settings screen has selected, if it is the active screen.
    pub fn selected_row(&self) -> Option<Row> {
        match self {
            Self::Settings(state) => state.selected_row(),
            Self::Typing(_) | Self::Results(_) => None,
        }
    }

    pub fn render(&self, app: &App, frame: &mut Frame) {
        match self {
            Self::Typing(screen) => screen.render(app, frame),
            Self::Settings(screen) => screen.render(app, frame),
            Self::Results(screen) => screen.render(app, frame),
        }
    }

    pub fn handle(&mut self, action: Action) -> Vec<Effect> {
        match self {
            Self::Typing(screen) => screen.handle(action),
            Self::Settings(screen) => screen.handle(action),
            Self::Results(screen) => screen.handle(action),
        }
    }
}
