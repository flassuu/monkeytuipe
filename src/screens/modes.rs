//! Who has the keyboard.
//!
//! The app has two modes, and the distinction is the same one vim draws: in one,
//! keys are *text*; in the other, keys are *commands*. Without it there is only
//! ever one honest answer to "what does `j` do" — and that answer has to be
//! "nothing", because `j` is a letter somebody wants to type.
//!
//! ## The two modes
//!
//! - **Input** — the command window is open and the field has the keyboard.
//!   Everything typed goes into it, including `j`, `k` and `q`. Escape leaves.
//! - **Navigation** — the window is closed and the screen has the keyboard.
//!   Arrows move, `hjkl` move, letters are commands.
//!
//! ## Getting between them
//!
//! | key | from | to |
//! |---|---|---|
//! | `tab` | either | the other |
//! | `i` | navigation | input |
//!
//! **Escape is deliberately not in this table.** It used to be, and having it here
//! was the bug behind both of the reports that led to this being written down:
//! a key this module claims is a key no screen can ever see, so a screen could
//! not be given escape back. It is now resolved by the *screen* — the command
//! window first, then an editor, then the settings screen, then the command line
//! — because "escape closes the topmost thing" is a question about what is on
//! screen, and only the screen knows that.
//!
//! `tab` toggles because it is the one key that is in neither mode's vocabulary:
//! on the typing screen it is *skip*, which is a real command and has to keep
//! working. So `tab` is only a mode switch where skipping makes no sense — in the
//! settings menu and the command window's own list.
//!
//! ## Why letters are commands in navigation mode
//!
//! The typing screen is the awkward one. There, a printable key is always text,
//! because a typist has to be able to type `t`, `,` and `q` and all three are
//! words as much as they are commands. So on the typing screen navigation mode
//! gets the arrows and the mode-switching keys and *nothing* else, and letters
//! stay text. On every other screen — the settings menu, the results — a letter
//! that is not part of a value being edited is a command, and the full `hjkl`
//! set works.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Which screen the keyboard rules apply to.
///
/// The rule is not "is the test running" — it is "is there a field on screen". A
/// settings row that collects text has the keyboard for the same reason the
/// command window does, and a screen that does not have one can use letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The typing screen, where every printable key is text.
    Typing,
    /// A screen with a value being typed into it.
    Editing,
    /// A screen with nothing being typed into it: settings, results.
    Browsing,
}

impl Surface {
    /// Whether printable keys are text here.
    ///
    /// `true` for typing, because that is the entire point of the typing screen,
    /// and `true` for editing, because there is a field and the field is the point.
    /// `false` only for browsing, where there is nothing to type into.
    pub fn types_letters(self) -> bool {
        matches!(self, Self::Typing | Self::Editing)
    }
}

/// The mode the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Keys move things around. Letters are commands.
    #[default]
    Navigation,
    /// Keys go into the command window. Letters are text.
    Input,
}

impl Mode {
    /// The other mode.
    pub fn other(self) -> Self {
        match self {
            Self::Navigation => Self::Input,
            Self::Input => Self::Navigation,
        }
    }

    /// The label the status line shows.
    ///
    /// Two letters, in the style vim uses, because that is the convention and a
    /// user who knows one knows the other. `INS` rather than `TYPE`, because the
    /// mode is about the keyboard and not about the test.
    pub fn label(self) -> &'static str {
        match self {
            Self::Navigation => "NAV",
            Self::Input => "INS",
        }
    }

    /// Whether a plain letter is *text* rather than a command, in this mode on
    /// this surface.
    ///
    /// This is the predicate that was missing, and it is why navigation on the
    /// main screen did not work.
    ///
    /// [`Surface::types_letters`] asked only about the surface, so the typing
    /// screen said yes in *every* mode — and in control mode `h`, `j`, `k` and `l`
    /// were still the first four letters of the next word. There was no way to
    /// drive the bar from the typing screen at all, because the one screen with a
    /// bar was the one screen where letters could not be commands.
    ///
    /// The screen and the mode are two different questions and the answer is their
    /// conjunction: a *field* always owns its letters, because there is somewhere
    /// for them to go; a *test* owns them only in typing mode, because in control
    /// mode the keyboard is the user interface rather than the input.
    pub fn owns_letters(self, surface: Surface) -> bool {
        match surface {
            // A field is a field in either mode. There is no control-mode way to
            // type a space into the word-count box, and there should not be.
            Surface::Editing => true,
            Surface::Typing => self == Self::Input,
            Surface::Browsing => false,
        }
    }
}

/// A key, reduced to the few facts the mode rules care about.
///
/// Reducing at the edge means the rules are a total function over a small type
/// instead of a pile of `matches!` on `KeyEvent`, and every combination is either
/// handled or explicitly ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Whether the terminal sent this as a key press rather than a release.
    ///
    /// Windows terminals report both, and acting on a release fires every
    /// binding twice.
    pub press: bool,
}

impl Key {
    /// Reads a crossterm event.
    pub fn from_event(event: KeyEvent) -> Self {
        Self {
            code: event.code,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
            press: event.kind == KeyEventKind::Press,
        }
    }

    /// A bare key, for tests.
    pub fn bare(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
            press: true,
        }
    }

    /// The crossterm modifiers this key carries.
    pub fn modifiers(self) -> KeyModifiers {
        let mut modifiers = KeyModifiers::NONE;
        if self.ctrl {
            modifiers |= KeyModifiers::CONTROL;
        }
        if self.alt {
            modifiers |= KeyModifiers::ALT;
        }
        if self.shift {
            modifiers |= KeyModifiers::SHIFT;
        }
        modifiers
    }

    /// A key with a modifier, for tests.
    pub fn with(code: KeyCode, ctrl: bool, alt: bool, shift: bool) -> Self {
        Self {
            code,
            ctrl,
            alt,
            shift,
            press: true,
        }
    }

    /// Whether this key is `c`, unshifted and unmodified.
    pub fn is_char(self, c: char) -> bool {
        matches!(self.code, KeyCode::Char(actual) if actual == c) && !self.ctrl && !self.alt
    }

    /// The letter this key is, lowercased, if it is a plain letter.
    ///
    /// Case-insensitive on purpose: `J` and `j` are the same command, and a
    /// terminal that reports `J` for shift+j should not lose the binding.
    pub fn letter(self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt => Some(c.to_ascii_lowercase()),
            _ => None,
        }
    }
}

/// What a key means, once the mode has had its say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Go to the other mode.
    Switch,
    /// Open the command window and go to input mode.
    OpenInput,
    /// Close the command window and go to navigation mode.
    CloseInput,
    /// Move the selection.
    Move(Direction),
    /// A letter that is a command, in navigation mode on a browsing screen.
    Command(char),
    /// A letter that is text, and must stay text.
    Text(char),
    /// Jump over the word, which is what `tab` means on the typing screen.
    Skip,
    /// The key has no job here.
    Nothing,
}

/// A direction to move in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// The opposite, for a key that toggles rather than moves.
    pub fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// The keys that switch mode, as the interface spells them.
///
/// The one place this string is written down. The mode layer decides what the key
/// *means* and the typing screen's overlay says what to *press*, and if those were
/// two literals then correcting one would leave the other telling a user to press
/// a key that does nothing — which is the worst thing a hint can do.
///
/// It is not in the keybind table on purpose: [`intent`] is the only answer to
/// "what does this key mean", and a table underneath it would be a second answer.
pub const SWITCH_KEYS: &str = "shift+enter";

/// Decides what a key means.
///
/// The three inputs are everything there is to know: which mode the app is in,
/// which screen it is on, and whether the command window is open. Nothing is
/// looked up from a keybind table, because the mode *is* the answer to "what does
/// this key mean" and a table underneath it would be a second answer.
pub fn intent(key: Key, mode: Mode, surface: Surface, window_open: bool) -> Intent {
    if !key.press {
        // A key release, or a repeat event on a terminal that sends both. Not a
        // command, and not text either — typing a letter twice because the
        // terminal said so would be a bug in the terminal's users.
        return Intent::Nothing;
    }
    if key.ctrl || key.alt {
        // Modified keys are the app's own: `ctrl+c` quits, `ctrl+r` restarts, and
        // those are configured. They are not modes and not text.
        return Intent::Nothing;
    }

    // Arrows mean the same thing in every mode and on every screen. In input mode
    // the window takes them — to move its own highlight — so they are not
    // consumed here when there is one.
    let arrow = match key.code {
        KeyCode::Up => Some(Direction::Up),
        KeyCode::Down => Some(Direction::Down),
        KeyCode::Left => Some(Direction::Left),
        KeyCode::Right => Some(Direction::Right),
        _ => None,
    };
    if let Some(direction) = arrow {
        return if window_open && mode == Mode::Input {
            Intent::Nothing
        } else {
            Intent::Move(direction)
        };
    }

    // Shift+Enter switches mode, everywhere, and is nothing else.
    //
    // It used to be reserved for ending a zen test, which meant the one key the
    // user is asked to press to *start* typing was also a key that could end a
    // test. A key that means one thing in one place and another in the next is
    // the definition of a key nobody can press with confidence. Zen ends on `esc`
    // instead, at the bottom of the existing escape stack.
    if key.code == KeyCode::Enter && key.shift {
        return Intent::Switch;
    }
    // Tab is a mode switch only where skipping a word would be meaningless. On the
    // typing screen it is *skip*, and taking it away would break the one command
    // every monkeytype user knows.
    if key.code == KeyCode::Tab {
        return if surface == Surface::Typing {
            Intent::Skip
        } else if window_open || mode == Mode::Input {
            Intent::CloseInput
        } else {
            Intent::Switch
        };
    }

    // `i` enters input mode, the way it does in vim — but only where a letter is
    // not text. On the typing screen `i` is the seventh most common letter in the
    // language, and a binding that swallows it would be the worst bug in this
    // file: it would not look like a broken keybinding, it would look like a
    // keyboard that drops characters. Inside a field it is a letter like any
    // other.
    //
    // Note what decides this: the *mode*, not the surface. In control mode on the
    // typing screen a letter is not text — the keyboard is the interface — so `i`
    // opens the command window there, and in typing mode it types an `i`.
    if key.is_char('i') && !window_open && mode == Mode::Navigation && !mode.owns_letters(surface) {
        return Intent::OpenInput;
    }

    // In input mode everything printable is the field's.
    if window_open || mode.owns_letters(surface) {
        return match key.code {
            KeyCode::Char(c) => Intent::Text(c),
            _ => Intent::Nothing,
        };
    }

    // Navigation, no window. The vim keys, on a surface where letters are not
    // text — which now includes the typing screen in control mode, and that is
    // the whole point: the bar is on the typing screen, so the bar is reachable
    // with the same keys on the same screen.
    {
        let direction = match key.letter() {
            Some('k') => Some(Direction::Up),
            Some('j') => Some(Direction::Down),
            Some('h') => Some(Direction::Left),
            Some('l') => Some(Direction::Right),
            _ => None,
        };
        if let Some(direction) = direction {
            return Intent::Move(direction);
        }
    }

    match key.code {
        KeyCode::Char(c) if !mode.owns_letters(surface) => Intent::Command(c.to_ascii_lowercase()),
        KeyCode::Char(c) => Intent::Text(c),
        _ => Intent::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The intent for a key, in one mode on one surface with the window as given.
    fn what(key: Key, mode: Mode, surface: Surface, window: bool) -> Intent {
        intent(key, mode, surface, window)
    }

    fn plain(code: KeyCode) -> Key {
        Key::bare(code)
    }

    fn letter(c: char) -> Key {
        Key::bare(KeyCode::Char(c))
    }

    // ---- escape crosses the modes -------------------------------------

    // ---- letters are text or commands, and that depends on the mode ----

    #[test]
    fn in_input_mode_a_letter_goes_into_the_field() {
        assert_eq!(
            what(letter('q'), Mode::Input, Surface::Browsing, true),
            Intent::Text('q')
        );
    }

    /// The whole reason the mode exists. `q` is a word and it is `quit`.
    #[test]
    fn in_navigation_mode_the_same_letter_is_a_command() {
        assert_eq!(
            what(letter('q'), Mode::Navigation, Surface::Browsing, false),
            Intent::Command('q')
        );
    }

    /// On the typing screen a letter is text in *typing* mode, and a typist has to
    /// be able to type `t`, `,` and `q`.
    #[test]
    fn the_typing_screen_takes_a_letter_as_text_in_typing_mode() {
        for c in ['t', ',', 'q', 'j', 'k', 'h', 'l', 'i'] {
            let got = what(letter(c), Mode::Input, Surface::Typing, false);
            assert!(
                matches!(got, Intent::Text(ch) if ch == c),
                "{c} in typing mode on the typing screen was {got:?}"
            );
        }
    }

    /// And in *control* mode on the same screen it is a command. This is the fix.
    ///
    /// It used to be text in both modes, because the rules asked only about the
    /// surface — and so the one screen with a settings bar was the one screen
    /// where no letter could be a command. There was no mode you could reach the
    /// bar from, which is what "navigation on the main screen does not work" meant
    /// all along.
    #[test]
    fn control_mode_on_the_typing_screen_takes_a_letter_as_a_command() {
        // The four vim letters are excluded: they *move*, which is the whole reason
        // control mode is worth having, and
        // `the_typing_screen_does_move_the_selection_with_hjkl_in_control_mode`
        // is where they are pinned. `i` is excluded too, because in control mode it
        // opens the command window — which is correct, and is the point: a letter
        // that is not text is available to be a command.
        for c in ['t', ',', 'q'] {
            let got = what(letter(c), Mode::Navigation, Surface::Typing, false);
            assert!(
                matches!(got, Intent::Command(ch) if ch == c),
                "{c} in control mode on the typing screen was {got:?}"
            );
        }
    }

    /// So the vim keys *do* move the bar there, which is the point of control mode
    /// being reachable from the main screen at all.
    #[test]
    fn the_typing_screen_does_move_the_selection_with_hjkl_in_control_mode() {
        for (c, direction) in [
            ('h', Direction::Left),
            ('j', Direction::Down),
            ('k', Direction::Up),
            ('l', Direction::Right),
        ] {
            assert_eq!(
                what(letter(c), Mode::Navigation, Surface::Typing, false),
                Intent::Move(direction),
                "{c} in control mode did not move the bar"
            );
        }
    }

    /// And they still do *not* move it in typing mode, where they are the first
    /// four letters of the next word. `h` moving the selection is worth less than
    /// `h` being typeable, and that has not changed.
    #[test]
    fn the_typing_screen_does_not_move_the_selection_with_hjkl() {
        for c in ['h', 'j', 'k', 'l'] {
            let got = what(letter(c), Mode::Input, Surface::Typing, false);
            assert!(
                matches!(got, Intent::Text(ch) if ch == c),
                "{c} in typing mode moved the selection: {got:?}"
            );
        }
    }

    // ---- the vim keys --------------------------------------------------

    #[test]
    fn hjkl_move_on_a_screen_where_letters_are_commands() {
        for (c, direction) in [
            ('k', Direction::Up),
            ('j', Direction::Down),
            ('h', Direction::Left),
            ('l', Direction::Right),
        ] {
            assert_eq!(
                what(letter(c), Mode::Navigation, Surface::Browsing, false),
                Intent::Move(direction),
                "{c} did not move {direction:?}"
            );
        }
    }

    /// Case does not matter, because a terminal that reports `J` for shift+j
    /// should not lose the binding.
    #[test]
    fn the_vim_keys_are_case_insensitive() {
        assert_eq!(
            what(letter('J'), Mode::Navigation, Surface::Browsing, false),
            Intent::Move(Direction::Down)
        );
    }

    /// A screen with nothing to select still has to give the keys *some* meaning
    /// rather than dropping them silently, so they are still moves. What the screen
    /// does with a move is its own business.
    #[test]
    fn a_vim_key_is_still_a_move_on_a_screen_with_nothing_to_select() {
        assert_eq!(
            what(letter('j'), Mode::Navigation, Surface::Browsing, false),
            Intent::Move(Direction::Down)
        );
    }

    /// And in input mode `hjkl` are four letters, because the field is where they
    /// belong.
    #[test]
    fn hjkl_are_letters_in_input_mode() {
        for c in ['h', 'j', 'k', 'l'] {
            assert_eq!(
                what(letter(c), Mode::Input, Surface::Browsing, true),
                Intent::Text(c),
                "{c} moved instead of being typed"
            );
        }
    }

    // ---- tab -----------------------------------------------------------

    /// Tab is *skip* on the typing screen. It is the one command every monkeytype
    /// user knows and it is not available to be a mode switch.
    #[test]
    fn tab_skips_a_word_on_the_typing_screen() {
        assert_eq!(
            what(
                plain(KeyCode::Tab),
                Mode::Navigation,
                Surface::Typing,
                false
            ),
            Intent::Skip
        );
    }

    #[test]
    fn tab_switches_modes_where_skipping_would_mean_nothing() {
        assert_eq!(
            what(
                plain(KeyCode::Tab),
                Mode::Navigation,
                Surface::Browsing,
                false
            ),
            Intent::Switch
        );
        assert_eq!(
            what(plain(KeyCode::Tab), Mode::Input, Surface::Browsing, true),
            Intent::CloseInput
        );
    }

    // ---- i -------------------------------------------------------------

    #[test]
    fn i_enters_input_mode_and_is_a_letter_inside_it() {
        assert_eq!(
            what(letter('i'), Mode::Navigation, Surface::Browsing, false),
            Intent::OpenInput
        );
        assert_eq!(
            what(letter('i'), Mode::Input, Surface::Browsing, true),
            Intent::Text('i')
        );
    }

    /// On the typing screen `i` is a letter, always. It is the most common letter
    /// in the language and it is not a command.
    #[test]
    fn i_is_a_letter_on_the_typing_screen() {
        // In typing mode, on the typing screen: `i` is the seventh most common
        // letter in the language and it is typed.
        assert_eq!(
            what(letter('i'), Mode::Input, Surface::Typing, false),
            Intent::Text('i'),
            "i opened the command window instead of being typed"
        );
        // And in an editor, which is also a field — in *either* mode, because a
        // field has somewhere to put a letter and the mode does not change that.
        assert_eq!(
            what(letter('i'), Mode::Navigation, Surface::Editing, false),
            Intent::Text('i')
        );
    }

    /// The same, for every letter the vim keys use. On the typing screen all of
    /// them are words.
    #[test]
    fn every_vim_letter_is_a_letter_where_letters_are_text() {
        for c in ['h', 'j', 'k', 'l', 'q', 'i'] {
            assert_eq!(
                what(letter(c), Mode::Navigation, Surface::Editing, false),
                Intent::Text(c),
                "{c} was taken as a command inside a field"
            );
        }
    }

    // ---- what no mode claims -------------------------------------------

    /// A modified key belongs to the keybind table and to nothing here. `ctrl+j`
    /// is not `j` and must not be read as `j`.
    #[test]
    fn a_modified_key_is_nobody_s_mistake() {
        for mode in [Mode::Navigation, Mode::Input] {
            for surface in [Surface::Typing, Surface::Editing, Surface::Browsing] {
                for c in ['j', 'k', 'q', 'i'] {
                    let key = Key::with(KeyCode::Char(c), true, false, false);
                    assert_eq!(
                        what(key, mode, surface, false),
                        Intent::Nothing,
                        "ctrl+{c} in {mode:?} on {surface:?}"
                    );
                }
            }
        }
    }

    /// A terminal that reports releases would otherwise fire every command twice
    /// and type every letter twice.
    #[test]
    fn a_key_release_is_not_a_command_and_not_text() {
        let mut key = letter('a');
        key.press = false;
        assert_eq!(
            what(key, Mode::Navigation, Surface::Typing, false),
            Intent::Nothing
        );
        assert_eq!(
            what(key, Mode::Navigation, Surface::Browsing, false),
            Intent::Nothing
        );
    }

    /// The arrows *are* claimed here, and that is the difference: a screen can be
    /// given the arrows back by changing this function, but it cannot be given
    /// back a key this function answers first. Which is why escape is not here.
    #[test]
    fn the_arrows_belong_to_the_window_when_it_is_open() {
        assert_eq!(
            what(plain(KeyCode::Down), Mode::Input, Surface::Browsing, true),
            Intent::Nothing,
            "the arrow was claimed by the screen as well as the window"
        );
        assert_eq!(
            what(
                plain(KeyCode::Down),
                Mode::Navigation,
                Surface::Browsing,
                false
            ),
            Intent::Move(Direction::Down)
        );
    }

    /// Shift+Enter switches mode, in every mode, on every surface, with no window
    /// and with one. It is the one key with one meaning.
    ///
    /// It used to be the way out of a zen test, which meant the key the typing
    /// screen *tells you to press to start typing* was also a key that could end a
    /// test. A key that means one thing in one place and another in the next is a
    /// key nobody can press on purpose, and this is the key the user is asked for
    /// first. Zen ends on `esc` now.
    #[test]
    fn shift_enter_switches_mode_everywhere() {
        let key = Key::with(KeyCode::Enter, false, false, true);
        for mode in [Mode::Navigation, Mode::Input] {
            for surface in [Surface::Typing, Surface::Editing, Surface::Browsing] {
                for window in [false, true] {
                    assert_eq!(
                        what(key, mode, surface, window),
                        Intent::Switch,
                        "shift+enter on {surface:?} in {mode:?} (window: {window})"
                    );
                }
            }
        }
    }

    /// And the hint names the same key the rules match on, so the overlay cannot
    /// tell a user to press something inert.
    #[test]
    fn the_switch_hint_names_the_key_the_rules_match() {
        assert_eq!(SWITCH_KEYS, "shift+enter");
        let key = Key::with(KeyCode::Enter, false, false, true);
        assert_eq!(
            what(key, Mode::Navigation, Surface::Typing, false),
            Intent::Switch
        );
    }

    /// Every mode, every surface, every key: nothing panics and nothing falls out
    /// of the rules. A `match` with a hole is a hole.
    #[test]
    fn the_rules_are_total_over_everything() {
        let codes = [
            KeyCode::Char('a'),
            KeyCode::Char('j'),
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Backspace,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::F(2),
        ];
        for mode in [Mode::Navigation, Mode::Input] {
            for surface in [Surface::Typing, Surface::Editing, Surface::Browsing] {
                for window in [false, true] {
                    for code in codes {
                        let _ = what(Key::bare(code), mode, surface, window);
                        let _ = what(Key::with(code, true, true, true), mode, surface, window);
                    }
                }
            }
        }
    }
}
