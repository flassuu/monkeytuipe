//! Semantic actions: what a key press *means*, independent of which key produced it.
//!
//! Resolution from a raw [`crossterm::event::KeyEvent`] happens in
//! [`App::resolve`](crate::app::App::resolve), driven by the user's keybind table.

/// A resolved input, either a UI command or a literal typed character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    Up,
    Down,
    Left,
    Right,
    Select,
    /// Leave the current screen, or cancel the current input.
    Back,
    /// Open the settings screen.
    Settings,
    /// Start a test with the current configuration.
    StartTest,
    /// Restart the test with the same configuration and a fresh word set.
    Restart,
    /// A literal character was typed (one `char`, not a full `String`).
    Char(char),
}
