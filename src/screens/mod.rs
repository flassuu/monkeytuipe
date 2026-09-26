//! Screens.
//!
//! A screen owns only its own view state and never touches [`App`] directly.
//! Instead it converts input into [`Effect`]s, which [`App`](crate::app::App)
//! applies. That keeps the data flow one-directional and lets a screen be
//! unit-tested without an `App`.

pub mod settings;
pub mod typing;

use ratatui::Frame;

use crate::action::Action;
use crate::app::App;

/// Something a screen asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// A screen identified by kind, for code that needs to name one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenKind {
    #[default]
    Typing,
    Settings,
}

impl ScreenKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Typing => "typing",
            Self::Settings => "settings",
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
        }
    }

    /// The row the settings screen has selected, if it is the active screen.
    pub fn selected_row(&self) -> Option<Row> {
        match self {
            Self::Settings(state) => state.selected_row(),
            Self::Typing(_) => None,
        }
    }

    pub fn render(&self, app: &App, frame: &mut Frame) {
        match self {
            Self::Typing(screen) => screen.render(app, frame),
            Self::Settings(screen) => screen.render(app, frame),
        }
    }

    pub fn handle(&mut self, action: Action) -> Vec<Effect> {
        match self {
            Self::Typing(screen) => screen.handle(action),
            Self::Settings(screen) => screen.handle(action),
        }
    }
}
