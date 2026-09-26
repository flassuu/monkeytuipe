//! Application state and event loop.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::Action;
use crate::config::theme::Theme;
use crate::config::Config;
use crate::screens::{Effect, Row, Screen, ScreenKind, ScreenState};
use crate::terminal::Tui;

/// How often the clock redraws. 100ms keeps the countdown readable without
/// burning CPU on a redraw loop that has nothing to animate.
pub const TICK: Duration = Duration::from_millis(100);

/// Word count in a placeholder test, until the real generator lands.
const PLACEHOLDER_WORDS: usize = 40;

pub struct App {
    pub config: Config,
    screen: ScreenState,
    words: Vec<String>,
    cursor_word: usize,
    config_path: PathBuf,
    /// Set when the config no longer matches what is on disk.
    dirty: bool,
}

impl App {
    /// Builds an app from a loaded config, seeding a word list so the UI is
    /// populated before the first keystroke.
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        Self {
            config,
            screen: ScreenState::default(),
            words: placeholder_words(),
            cursor_word: 0,
            config_path,
            dirty: false,
        }
    }

    // ---- accessors used by screens -------------------------------------

    pub fn theme(&self) -> Theme {
        self.config.theme.resolve()
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    pub fn cursor_word(&self) -> usize {
        self.cursor_word
    }

    /// The first word index to draw, chosen so the active word stays on screen.
    ///
    /// The active word is kept one third of the way down the window rather than
    /// pinned to the top, which leaves finished words visible above it.
    pub fn scroll_offset(&self, visible: usize) -> usize {
        if visible == 0 {
            return 0;
        }
        let context = (visible - 1) / 3;
        self.cursor_word.saturating_sub(context)
    }

    /// Column of the caret within the drawn word line.
    ///
    /// The caret sits at the start of the active word, so this is the display
    /// width of everything drawn before it: the words from the scroll offset up
    /// to (but excluding) the active word, plus one space between each pair.
    ///
    /// `visible` must be the same value passed to [`scroll_offset`](Self::scroll_offset),
    /// otherwise the caret lands on the wrong cell.
    pub fn caret_column(&self, visible: usize) -> usize {
        use unicode_width::UnicodeWidthStr;

        let first = self.scroll_offset(visible);
        let mut column = 0usize;
        for word in self.words.iter().take(self.cursor_word).skip(first) {
            column += word.width() + 1;
        }
        column
    }

    /// The character under the caret, if the test has not run out of words.
    pub fn current_char(&self) -> Option<char> {
        self.words.get(self.cursor_word)?.chars().next()
    }

    // The live counters are placeholders until the engine lands in Phase 1.
    pub fn wpm(&self) -> u32 {
        0
    }

    pub fn accuracy(&self) -> f64 {
        100.0
    }

    pub fn remaining_secs(&self) -> u32 {
        self.config.test.time
    }

    /// The screen currently on display.
    pub fn screen_kind(&self) -> ScreenKind {
        self.screen.kind()
    }

    /// The settings row currently highlighted, for tests and later UI.
    pub fn selected_row(&self) -> Option<Row> {
        self.screen.selected_row()
    }

    /// Draws the current screen.
    ///
    /// Public so the app can be rendered headlessly, e.g. against
    /// `ratatui::backend::TestBackend` in tests.
    pub fn render(&self, frame: &mut ratatui::Frame) {
        self.screen.render(self, frame);
    }

    /// Switches screens. Used by effects today, and by a command palette later.
    pub fn show_screen(&mut self, kind: ScreenKind) {
        self.apply(Effect::Switch(kind));
    }

    /// Installs a word list and puts the caret at its start.
    ///
    /// The engine calls this with a freshly generated list; tests use it to
    /// control the words being rendered.
    pub fn set_words(&mut self, words: Vec<String>) {
        self.words = words;
        self.cursor_word = 0;
    }

    /// Moves the caret to a word index, clamped to the list.
    pub fn set_cursor_word(&mut self, word: usize) {
        self.cursor_word = word.min(self.words.len().saturating_sub(1));
    }

    // ---- effects -------------------------------------------------------

    /// Applies one effect. Returns `true` when the app should quit.
    fn apply(&mut self, effect: Effect) -> bool {
        match effect {
            Effect::Quit => true,
            Effect::Switch(kind) => {
                self.screen = match kind {
                    ScreenKind::Typing => ScreenState::Typing(Default::default()),
                    ScreenKind::Settings => ScreenState::Settings(Default::default()),
                };
                false
            }
            Effect::RestartTest => {
                self.words = placeholder_words();
                self.cursor_word = 0;
                false
            }
            Effect::Adjust(row) => {
                self.adjust(row);
                false
            }
            Effect::Toggle(row) => {
                self.adjust(row);
                false
            }
        }
    }

    /// Applies a `left`/`right` press, or a toggle, to a settings row.
    fn adjust(&mut self, row: Row) {
        match row {
            Row::Theme => self.config.theme = self.config.theme.next(),
            Row::Language => {
                self.config.test.language = next_language(&self.config.test.language);
            }
            Row::Punctuation => self.config.test.punctuation = !self.config.test.punctuation,
            Row::Numbers => self.config.test.numbers = !self.config.test.numbers,
            Row::SubmitResults => self.config.submit_results = !self.config.submit_results,
            // The ApeKey is text input and `Back` is navigation; neither is a toggle.
            Row::ApeKey | Row::Back => return,
        }
        self.dirty = true;
    }

    /// True when the config has changes that are not on disk.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Persists the config. Called on quit so a session's settings survive.
    pub fn save_config(&mut self) -> anyhow::Result<()> {
        self.config.save(&self.config_path)?;
        self.dirty = false;
        Ok(())
    }

    // ---- loop ----------------------------------------------------------

    /// Runs until the user quits or the terminal errors.
    pub fn run(&mut self, terminal: &mut Tui) -> anyhow::Result<()> {
        let mut last_tick = Instant::now();
        loop {
            terminal.draw(|frame| self.render(frame))?;
            let timeout = TICK.saturating_sub(last_tick.elapsed());
            if !event::poll(timeout).context("polling terminal events")? {
                last_tick = Instant::now();
                continue;
            }

            match event::read().context("reading terminal event")? {
                Event::Key(key) if self.on_key(key)? => return Ok(()),
                Event::Key(_)
                | Event::Resize(_, _)
                | Event::FocusGained
                | Event::FocusLost
                | Event::Mouse(_)
                | Event::Paste(_) => {}
            }
        }
    }

    /// Applies one raw key event. Returns `true` when the app should quit.
    fn on_key(&mut self, key: KeyEvent) -> anyhow::Result<bool> {
        let Some(action) = self.resolve(key) else {
            return Ok(false);
        };

        let effects = match &mut self.screen {
            ScreenState::Typing(screen) => screen.handle(action),
            ScreenState::Settings(screen) => screen.handle(action),
        };

        let mut quit = false;
        for effect in effects {
            quit |= self.apply(effect);
        }
        if quit {
            // Save on the way out, but never let a bad path stop the app from
            // restoring the terminal.
            let _ = self.save_config().context("saving the config on exit");
        }
        Ok(quit)
    }

    /// Maps a key to an action using the configured keybinds.
    fn resolve(&self, key: KeyEvent) -> Option<Action> {
        // Terminals that report key releases (Windows) would otherwise fire
        // every binding twice.
        if key.kind != KeyEventKind::Press {
            return None;
        }
        // Anything unbound is a literal character, which the engine will want.
        let literal = match key.code {
            KeyCode::Char(c) => Some(Action::Char(c)),
            _ => None,
        };
        let name = describe(&key)?;
        self.config
            .keybinds
            .all()
            .into_iter()
            .find(|(_, keys)| keys.iter().any(|k| k == &name))
            .map(|(action, _)| action)
            .or(literal)
    }
}

/// A placeholder word list until the real generator lands in a later phase.
fn placeholder_words() -> Vec<String> {
    const WORDS: &[&str] = &[
        "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "and", "then",
    ];
    (0..PLACEHOLDER_WORDS)
        .map(|i| WORDS[i % WORDS.len()].to_owned())
        .collect()
}

/// Cycles through a short language list until the real language set is wired in.
fn next_language(current: &str) -> String {
    const LANGUAGES: &[&str] = &["english", "russian", "german"];
    LANGUAGES
        .iter()
        .position(|l| *l == current)
        .map(|i| LANGUAGES[(i + 1) % LANGUAGES.len()].to_owned())
        .unwrap_or_else(|| LANGUAGES[0].to_owned())
}

/// Normalises a key event to the spelling used in `config.toml`.
fn describe(key: &KeyEvent) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_owned());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_owned());
    }
    let base = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".to_owned(),
        KeyCode::Esc => "esc".to_owned(),
        KeyCode::Tab => "tab".to_owned(),
        KeyCode::Backspace => "backspace".to_owned(),
        KeyCode::BackTab => "backtab".to_owned(),
        KeyCode::Delete => "delete".to_owned(),
        KeyCode::Insert => "insert".to_owned(),
        KeyCode::Home => "home".to_owned(),
        KeyCode::End => "end".to_owned(),
        KeyCode::PageUp => "pageup".to_owned(),
        KeyCode::PageDown => "pagedown".to_owned(),
        KeyCode::Up => "up".to_owned(),
        KeyCode::Down => "down".to_owned(),
        KeyCode::Left => "left".to_owned(),
        KeyCode::Right => "right".to_owned(),
        // Media and other keys are not representable as a config string.
        _ => return None,
    };
    parts.push(base);
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keybinds::Binding;
    use crossterm::event::KeyEventState;

    fn app() -> App {
        let dir = std::env::temp_dir().join(format!("monkeytuipe-app-{}", std::process::id()));
        App::new(Config::default(), dir.join("config.toml"))
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn press_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_quits_from_the_typing_screen() {
        let mut app = app();
        assert!(app.on_key(press(KeyCode::Char('q'))).expect("no io"));
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = app();
        assert!(app
            .on_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .expect("no io"));
    }

    #[test]
    fn comma_toggles_the_settings_screen() {
        let mut app = app();
        assert!(!app.on_key(press(KeyCode::Char(','))).expect("no io"));
        assert_eq!(app.screen_kind(), ScreenKind::Settings);
        assert!(!app.on_key(press(KeyCode::Char(','))).expect("no io"));
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = app();
        let key = KeyEvent {
            kind: KeyEventKind::Release,
            ..press(KeyCode::Char('q'))
        };
        assert!(!app.on_key(key).expect("no io"), "a release must not quit");
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
    }

    #[test]
    fn unbound_characters_fall_through_as_char_actions() {
        let app = app();
        assert_eq!(
            app.resolve(press(KeyCode::Char('z'))),
            Some(Action::Char('z'))
        );
    }

    #[test]
    fn bound_characters_do_not_also_fall_through() {
        let app = app();
        assert_eq!(app.resolve(press(KeyCode::Char('q'))), Some(Action::Quit));
    }

    #[test]
    fn settings_navigation_wraps() {
        let mut app = app();
        app.on_key(press(KeyCode::Char(','))).expect("no io");
        assert_eq!(app.selected_row(), Some(Row::Theme));
        for _ in 0..crate::screens::settings::ROWS.len() {
            app.on_key(press(KeyCode::Down)).expect("no io");
        }
        assert_eq!(
            app.selected_row(),
            Some(Row::Theme),
            "down should wrap back to the top"
        );
        app.on_key(press(KeyCode::Up)).expect("no io");
        assert_eq!(
            app.selected_row(),
            Some(Row::Back),
            "up should wrap to the bottom"
        );
    }

    #[test]
    fn esc_leaves_the_settings_screen() {
        let mut app = app();
        app.on_key(press(KeyCode::Char(','))).expect("no io");
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
    }

    #[test]
    fn toggling_a_setting_marks_the_config_dirty() {
        let mut app = app();
        assert!(!app.is_dirty());
        app.on_key(press(KeyCode::Char(','))).expect("no io");
        // Row 0 is the theme; right cycles it.
        app.on_key(press(KeyCode::Right)).expect("no io");
        assert!(app.is_dirty());
        assert_eq!(app.config.theme, crate::config::theme::ThemeName::Gruvbox);
    }

    #[test]
    fn restarting_resets_the_word_cursor() {
        let mut app = app();
        app.cursor_word = 12;
        app.on_key(press(KeyCode::Char('r'))).expect("no io");
        assert_eq!(app.cursor_word(), 0);
    }

    #[test]
    fn scroll_offset_keeps_the_active_word_visible() {
        let mut app = app();
        for visible in 1..=8usize {
            for cursor in 0..app.words().len() {
                app.cursor_word = cursor;
                let first = app.scroll_offset(visible);
                assert!(cursor >= first, "word {cursor} is above the window");
                assert!(
                    cursor < first + visible,
                    "word {cursor} is past the window (first={first}, visible={visible})"
                );
            }
        }
    }

    #[test]
    fn a_one_line_window_shows_only_the_active_word() {
        let mut app = app();
        app.cursor_word = 7;
        assert_eq!(
            app.scroll_offset(1),
            7,
            "one row can only hold the active word"
        );
        assert_eq!(app.caret_column(1), 0);
    }

    #[test]
    fn caret_column_equals_the_width_of_what_is_drawn_before_it() {
        use unicode_width::UnicodeWidthStr;

        let mut app = app();
        for visible in 1..=8usize {
            for cursor in 0..app.words().len() {
                app.cursor_word = cursor;
                let first = app.scroll_offset(visible);
                let expected: usize = app
                    .words()
                    .iter()
                    .take(cursor)
                    .skip(first)
                    .map(|word| word.as_str().width() + 1)
                    .sum();
                assert_eq!(
                    app.caret_column(visible),
                    expected,
                    "visible={visible} cursor={cursor}"
                );
            }
        }
    }

    #[test]
    fn caret_column_counts_words_and_gaps() {
        let mut app = app();
        // The placeholder list starts: the(3) quick(5) brown(5) ...
        // A four-row window keeps one word of context above the active word, so
        // the window starts at `cursor - 1` once the cursor passes the first row.
        app.cursor_word = 0;
        assert_eq!(app.scroll_offset(4), 0);
        assert_eq!(app.caret_column(4), 0);

        app.cursor_word = 1;
        assert_eq!(app.scroll_offset(4), 0);
        assert_eq!(app.caret_column(4), 4, "'the' plus one space");

        app.cursor_word = 2;
        assert_eq!(app.scroll_offset(4), 1, "'the' has scrolled off the top");
        assert_eq!(app.caret_column(4), 6, "'quick' plus one space");
    }

    #[test]
    fn caret_column_never_lands_past_the_window() {
        use unicode_width::UnicodeWidthStr;

        let mut app = app();
        for cursor in 0..app.words().len() {
            app.cursor_word = cursor;
            let column = app.caret_column(3);
            let drawn: usize = app
                .words()
                .iter()
                .skip(app.scroll_offset(3))
                .take(3)
                .map(|word| word.as_str().width())
                .sum();
            assert!(
                column <= drawn,
                "caret at column {column} but the window is only {drawn} wide"
            );
        }
    }

    #[test]
    fn describe_normalises_key_spellings() {
        assert_eq!(describe(&press(KeyCode::Up)).as_deref(), Some("up"));
        assert_eq!(describe(&press(KeyCode::Char(' '))).as_deref(), Some(" "));
        assert_eq!(
            describe(&press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)).as_deref(),
            Some("ctrl+c")
        );
    }

    #[test]
    fn default_keybinds_cover_every_binding_variant() {
        let keybinds = Config::default().keybinds;
        let expected = [
            Binding::Quit,
            Binding::Up,
            Binding::Down,
            Binding::Left,
            Binding::Right,
            Binding::Select,
            Binding::Back,
            Binding::Settings,
            Binding::StartTest,
            Binding::Restart,
        ];
        assert_eq!(keybinds.all().len(), expected.len());
        for (_, keys) in keybinds.all() {
            assert!(!keys.is_empty(), "a binding must have at least one key");
        }
    }
}
