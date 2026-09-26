//! Application state and event loop.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::Action;
use crate::config::theme::Theme;
use crate::config::Config;
use crate::engine::{Mode, Test};
use crate::screens::{Effect, Row, Screen, ScreenKind, ScreenState};
use crate::stats::{calculate_wpm, CharCounts};
use crate::terminal::Tui;

/// How often the clock redraws. 100ms keeps the countdown readable without
/// burning CPU on a redraw loop that has nothing to animate.
pub const TICK: Duration = Duration::from_millis(100);

/// Word count in a placeholder test, until the real generator lands.
const PLACEHOLDER_WORDS: usize = 40;

pub struct App {
    pub config: Config,
    screen: ScreenState,
    test: Test,
    /// Wall-clock reading of when the test began, if it has.
    anchor: Option<Instant>,
    /// How long the test has been running. Cached so it can be set directly by
    /// tests and by any future replay.
    elapsed: Duration,
    config_path: PathBuf,
    /// Set when the config no longer matches what is on disk.
    dirty: bool,
}

impl App {
    /// Builds an app from a loaded config, seeding a test so the UI is
    /// populated before the first keystroke.
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        let test = Test::new(placeholder_words(), Mode::Time, config.test.time);
        Self {
            config,
            screen: ScreenState::default(),
            test,
            anchor: None,
            elapsed: Duration::ZERO,
            config_path,
            dirty: false,
        }
    }

    // ---- accessors used by screens -------------------------------------

    pub fn theme(&self) -> Theme {
        self.config.theme.resolve()
    }

    /// The test in progress.
    pub fn test(&self) -> &Test {
        &self.test
    }

    /// Index of the word the caret is on.
    pub fn cursor_word(&self) -> usize {
        self.test.active_index()
    }

    /// The first word index to draw for a caret on `cursor`, chosen so the
    /// active word stays on screen.
    ///
    /// The active word is kept one third of the way down the window rather than
    /// pinned to the top, which leaves finished words visible above it.
    ///
    /// The caret is a parameter rather than read from the engine so that the
    /// geometry can be exercised at any position without having to fake a run of
    /// keystrokes to get there.
    pub fn scroll_offset_from(&self, cursor: usize, visible: usize) -> usize {
        if visible == 0 {
            return 0;
        }
        let context = (visible - 1) / 3;
        cursor.saturating_sub(context)
    }

    /// [`Self::scroll_offset_from`] for the live caret position.
    pub fn scroll_offset(&self, visible: usize) -> usize {
        self.scroll_offset_from(self.cursor_word(), visible)
    }

    /// Column of the caret within the drawn word line, for a caret on `cursor`.
    ///
    /// The caret sits at the start of the active word, so this is the display
    /// width of everything drawn before it: the words from the scroll offset up
    /// to (but excluding) the active word, plus one space between each pair.
    ///
    /// `visible` must be the same value passed to
    /// [`scroll_offset_from`](Self::scroll_offset_from), otherwise the caret
    /// lands on the wrong cell.
    pub fn caret_column_from(&self, cursor: usize, visible: usize) -> usize {
        use unicode_width::UnicodeWidthStr;

        let first = self.scroll_offset_from(cursor, visible);
        let mut column = 0usize;
        for word in self.test.words().iter().take(cursor).skip(first) {
            column += word.text().width() + 1;
        }
        column
    }

    /// [`Self::caret_column_from`] for the live caret position.
    pub fn caret_column(&self, visible: usize) -> usize {
        self.caret_column_from(self.cursor_word(), visible)
    }

    /// The character under the caret, if the test has not run out of words.
    pub fn current_char(&self) -> Option<char> {
        let word = self.test.active_word();
        word.char_at(word.input_len_utf16())
    }

    // ---- live counters -------------------------------------------------

    /// Seconds the test has been running.
    pub fn elapsed_secs(&self) -> f64 {
        self.elapsed.as_secs_f64()
    }

    /// Characters produced so far, including the word being typed.
    ///
    /// The word in progress is scored on the spot rather than accumulated, so
    /// the live figure moves as you type instead of jumping a word at a time.
    /// That costs one pass over a single word, which is not the same as
    /// rescanning the whole test on every keystroke.
    pub fn live_chars(&self) -> CharCounts {
        let mut counts = self.test.chars();
        if self.test.is_running() {
            counts += self.test.active_word().count(true, true);
        }
        counts
    }

    /// Live words per minute, from the characters produced so far.
    pub fn wpm(&self) -> f64 {
        let seconds = self.elapsed_secs();
        if seconds <= 0.0 {
            return 0.0;
        }
        calculate_wpm(f64::from(self.live_chars().all_correct), seconds)
    }

    /// Live accuracy, as a percentage of keystrokes that were right.
    pub fn accuracy(&self) -> f64 {
        self.test.accuracy()
    }

    /// The countdown for the status line.
    ///
    /// Seconds for a timed test, words left for a word-count one.
    pub fn countdown(&self) -> String {
        match self.test.mode() {
            Mode::Time => {
                let left = f64::from(self.test.mode2()) - self.elapsed_secs();
                format!("{:>3}s", left.ceil().max(0.0) as u32)
            }
            Mode::Words | Mode::Quote => {
                let total = self.test.words().len() as u32;
                format!(
                    "{}/{}",
                    self.cursor_word().min(total as usize) as u32,
                    total
                )
            }
        }
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

    /// Installs a word list and puts the caret at its start, resetting the clock.
    ///
    /// The engine calls this with a freshly generated list; tests use it to
    /// control the words being rendered.
    pub fn set_words(&mut self, words: Vec<String>) {
        self.test = Test::new(words, self.test.mode(), self.test.mode2());
        self.reset_clock();
    }

    /// Moves the caret to a word index, clamped to the list.
    ///
    /// Words between here and where the caret was are marked skipped, which is
    /// the only way to reach a later word without typing to it.
    pub fn set_cursor_word(&mut self, word: usize) {
        let clamped = word.min(self.test.words().len().saturating_sub(1));
        for _ in self.cursor_word()..clamped {
            self.test.skip();
        }
    }
    /// Sets how long the test has been running.
    ///
    /// This detaches the clock from the wall clock, so the next [`Self::tick`]
    /// will not overwrite the value. The event loop never calls it; it exists for
    /// tests and for a future replay, both of which have to drive time by hand.
    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.anchor = None;
        self.elapsed = elapsed;
    }

    /// Resets the clock and clears the anchor, so time runs again from zero.
    fn reset_clock(&mut self) {
        self.elapsed = Duration::ZERO;
        self.anchor = None;
    }

    /// Advances the clock, ending the test if its time is up.
    ///
    /// The ticker is the only source of time, so the countdown and the timeout
    /// never depend on when a keystroke happened to arrive.
    pub fn tick(&mut self) {
        if self.test.is_finished() {
            self.freeze_clock();
            return;
        }
        if let Some(anchor) = self.anchor {
            self.elapsed = anchor.elapsed();
        }
        self.check_timeout();
    }

    /// Stops the clock where the test ended.
    ///
    /// The engine is the only thing that knows a test is over, and the clock is
    /// the only thing that can keep it honest. Left running, `elapsed` would go
    /// on growing after the last keystroke and the final wpm would quietly decay
    /// for as long as the screen stayed up — a result that changes while you look
    /// at it. Dropping the anchor is enough: `set_elapsed` stays free to drive
    /// the clock by hand.
    fn freeze_clock(&mut self) {
        self.anchor = None;
    }

    fn check_timeout(&mut self) {
        if self.test.mode() != Mode::Time || self.test.is_finished() {
            return;
        }
        if self.elapsed_secs() >= f64::from(self.test.mode2()) {
            self.test.finish();
            self.freeze_clock();
        }
    }

    /// Feeds a typed character to the engine, starting the clock on the first.
    pub fn type_char(&mut self, c: char) {
        if self.test.is_started() {
            if self.test.is_finished() {
                // Input after the end is dropped by the engine; refreshing the
                // anchor here would restart the clock behind a frozen result.
                return;
            }
        } else {
            self.anchor = Some(Instant::now());
        }
        self.test.input(c);
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
                self.test = Test::new(placeholder_words(), self.test.mode(), self.test.mode2());
                self.reset_clock();
                false
            }
            Effect::Type(c) => {
                self.type_char(c);
                false
            }
            Effect::Backspace => {
                self.test.backspace();
                false
            }
            Effect::SkipWord => {
                if !self.test.is_started() {
                    self.anchor = Some(Instant::now());
                }
                self.test.skip();
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
                self.tick();
                continue;
            }

            match event::read().context("reading terminal event")? {
                Event::Key(key) if self.on_key(key)? => return Ok(()),
                Event::Key(_)
                | Event::Resize(_, _)
                | Event::FocusGained
                | Event::FocusLost
                | Event::Mouse(_)
                | Event::Paste(_) => self.tick(),
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
    ///
    /// On the typing screen a printable key is *always* text, even when the
    /// keybind table claims it: a typist has to be able to type `t`, `,` or `q`,
    /// all of which are words as much as they are commands. The defaults avoid
    /// bare letters for exactly this reason, but a user who binds one anyway
    /// should not find it silently swallowed mid-test.
    fn resolve(&self, key: KeyEvent) -> Option<Action> {
        // Terminals that report key releases (Windows) would otherwise fire
        // every binding twice.
        if key.kind != KeyEventKind::Press {
            return None;
        }

        // Shift is ignored: the character already carries the case, so `A`
        // arrives as `KeyCode::Char('A')` and must still be text. Any other
        // modifier means the key is a shortcut rather than a letter.
        let shortcut = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let printable = match key.code {
            KeyCode::Char(c) => shortcut.then_some(c),
            _ => None,
        };
        if self.screen.kind() == ScreenKind::Typing {
            if let Some(c) = printable {
                return Some(Action::Char(c));
            }
        }

        self.bound_action(&key).or(match key.code {
            KeyCode::Char(c) => Some(Action::Char(c)),
            KeyCode::Enter => Some(Action::Char('\n')),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Tab => Some(Action::Skip),
            _ => None,
        })
    }

    /// The action a key is bound to in the user's keybind table.
    fn bound_action(&self, key: &KeyEvent) -> Option<Action> {
        let name = describe(key)?;
        self.config
            .keybinds
            .all()
            .into_iter()
            .find(|(_, keys)| keys.iter().any(|k| k == &name))
            .map(|(action, _)| action)
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
        KeyCode::F(n) => format!("f{n}"),
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

    /// Sends a string of keystrokes through the real key path.
    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = app();
        assert!(app
            .on_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .expect("no io"));
    }

    #[test]
    fn ctrl_c_quits_even_mid_test() {
        let mut app = app();
        type_text(&mut app, "the quick ");
        assert!(app.test().is_running());
        assert!(app
            .on_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .expect("no io"));
    }

    #[test]
    fn a_bare_letter_is_text_on_the_typing_screen_even_if_it_is_bound() {
        let mut app = app();
        // Bind `q` to quit in the user's own config, as older versions did.
        app.config.keybinds.quit = vec!["q".into()];
        app.on_key(press(KeyCode::Char('q'))).expect("no io");
        assert!(!app.test().is_finished(), "`q` typed instead of quitting");
        assert_eq!(app.test().active_word().input(), "q");
    }

    #[test]
    fn f2_toggles_the_settings_screen() {
        let mut app = app();
        assert!(!app.on_key(press(KeyCode::F(2))).expect("no io"));
        assert_eq!(app.screen_kind(), ScreenKind::Settings);
        assert!(!app.on_key(press(KeyCode::F(2))).expect("no io"));
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
    }

    #[test]
    fn a_comma_is_typed_not_treated_as_a_command() {
        // Punctuation tests need it, so it can never open a menu.
        let mut app = app();
        app.on_key(press(KeyCode::Char(','))).expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
        assert_eq!(app.test().active_word().input(), ",");
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
    fn a_bound_control_key_resolves_to_its_action() {
        let app = app();
        assert_eq!(
            app.resolve(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::Quit)
        );
        assert_eq!(app.resolve(press(KeyCode::F(2))), Some(Action::Settings));
    }

    #[test]
    fn settings_navigation_wraps() {
        let mut app = app();
        app.on_key(press(KeyCode::F(2))).expect("no io");
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
        app.on_key(press(KeyCode::F(2))).expect("no io");
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
    }

    #[test]
    fn toggling_a_setting_marks_the_config_dirty() {
        let mut app = app();
        assert!(!app.is_dirty());
        app.on_key(press(KeyCode::F(2))).expect("no io");
        // Row 0 is the theme; right cycles it.
        app.on_key(press(KeyCode::Right)).expect("no io");
        assert!(app.is_dirty());
        assert_eq!(app.config.theme, crate::config::theme::ThemeName::Gruvbox);
    }

    #[test]
    fn restarting_resets_the_word_cursor_and_the_clock() {
        let mut app = app();
        type_text(&mut app, "the ");
        app.set_elapsed(Duration::from_secs(7));
        assert!(app.cursor_word() > 0);

        app.on_key(press_mod(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("no io");
        assert_eq!(app.cursor_word(), 0);
        assert_eq!(app.elapsed_secs(), 0.0, "the clock restarts too");
        assert_eq!(app.wpm(), 0.0);
    }

    #[test]
    fn scroll_offset_keeps_the_active_word_visible() {
        let app = app();
        for visible in 1..=8usize {
            for cursor in 0..app.test().words().len() {
                let first = app.scroll_offset_from(cursor, visible);
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
        let app = app();
        assert_eq!(
            app.scroll_offset_from(7, 1),
            7,
            "one row can only hold the active word"
        );
        assert_eq!(app.caret_column_from(7, 1), 0);
    }

    #[test]
    fn caret_column_equals_the_width_of_what_is_drawn_before_it() {
        use unicode_width::UnicodeWidthStr;

        let app = app();
        for visible in 1..=8usize {
            for cursor in 0..app.test().words().len() {
                let first = app.scroll_offset_from(cursor, visible);
                let expected: usize = app
                    .test()
                    .words()
                    .iter()
                    .take(cursor)
                    .skip(first)
                    .map(|word| word.text().width() + 1)
                    .sum();
                assert_eq!(
                    app.caret_column_from(cursor, visible),
                    expected,
                    "visible={visible} cursor={cursor}"
                );
            }
        }
    }

    #[test]
    fn caret_column_counts_words_and_gaps() {
        let app = app();
        // The placeholder list starts: the(3) quick(5) brown(5) ...
        // A four-row window keeps one word of context above the active word, so
        // the window starts at `cursor - 1` once the cursor passes the first row.
        assert_eq!(app.scroll_offset_from(0, 4), 0);
        assert_eq!(app.caret_column_from(0, 4), 0);

        assert_eq!(app.scroll_offset_from(1, 4), 0);
        assert_eq!(app.caret_column_from(1, 4), 4, "'the' plus one space");

        assert_eq!(
            app.scroll_offset_from(2, 4),
            1,
            "'the' has scrolled off the top"
        );
        assert_eq!(app.caret_column_from(2, 4), 6, "'quick' plus one space");
    }

    #[test]
    fn caret_column_never_lands_past_the_window() {
        use unicode_width::UnicodeWidthStr;

        let app = app();
        for cursor in 0..app.test().words().len() {
            let column = app.caret_column_from(cursor, 3);
            let drawn: usize = app
                .test()
                .words()
                .iter()
                .skip(app.scroll_offset_from(cursor, 3))
                .take(3)
                .map(|word| word.text().width())
                .sum();
            assert!(
                column <= drawn,
                "caret at column {column} but the window is only {drawn} wide"
            );
        }
    }

    // ---- live engine wiring --------------------------------------------

    #[test]
    fn typing_moves_the_caret_and_updates_the_counters() {
        let mut app = app();
        type_text(&mut app, "the ");
        assert_eq!(app.cursor_word(), 1);
        assert!(
            (app.accuracy() - 100.0).abs() < 1e-9,
            "nothing was mistyped"
        );
        app.set_elapsed(Duration::from_secs(10));
        // Four characters in ten seconds: 4 / 5 / (10 / 60) = 4.8.
        assert!((app.wpm() - 4.8).abs() < 1e-9, "got {}", app.wpm());
    }

    #[test]
    fn the_clock_does_not_run_before_the_first_keystroke() {
        let mut app = app();
        app.tick();
        assert!(!app.test().is_started());
        assert_eq!(app.elapsed_secs(), 0.0);
        assert_eq!(app.wpm(), 0.0);
    }

    #[test]
    fn a_mistyped_character_lowers_the_accuracy_but_not_the_word_count() {
        let mut app = app();
        // "tge" against the target "the": one wrong letter, right space.
        type_text(&mut app, "tge ");
        assert!(
            (app.accuracy() - 75.0).abs() < 1e-9,
            "got {}",
            app.accuracy()
        );
        let chars = app.test().chars();
        assert_eq!(chars.correct_word, 0, "the word was not exact");
        assert_eq!(chars.incorrect, 1);
        assert_eq!(chars.extra, 1, "the space of a wrong word counts as extra");
    }

    #[test]
    fn a_transposed_word_costs_two_keystrokes() {
        // "teh" against "the" mistypes two characters, because the caret does not
        // skip ahead when a letter goes wrong.
        let mut app = app();
        type_text(&mut app, "teh ");
        assert!(
            (app.accuracy() - 50.0).abs() < 1e-9,
            "got {}",
            app.accuracy()
        );
    }

    #[test]
    fn a_bound_key_is_typed_while_a_test_runs() {
        let mut app = app();
        app.on_key(press(KeyCode::Char('q'))).expect("no io");
        assert!(app.test().is_started());
        assert_eq!(app.test().active_word().input(), "q");
    }

    #[test]
    fn a_capital_letter_is_still_text_even_though_it_arrives_as_shift() {
        let mut app = app();
        app.config.keybinds.left = vec!["a".into()];
        app.on_key(press_mod(KeyCode::Char('A'), KeyModifiers::SHIFT))
            .expect("no io");
        assert_eq!(app.test().active_word().input(), "A");
    }

    #[test]
    fn a_capital_letter_reaches_the_engine_without_a_modifier_too() {
        // Some terminals report the upper-case character with no modifier at all.
        let mut app = app();
        app.config.keybinds.left = vec!["a".into()];
        app.on_key(press(KeyCode::Char('A'))).expect("no io");
        assert_eq!(app.test().active_word().input(), "A");
    }

    #[test]
    fn an_arrow_with_a_modifier_is_still_a_shortcut() {
        // Ctrl+Left is a word-jump, not a letter, and must not be typed.
        let app = app();
        assert_eq!(
            app.resolve(press_mod(KeyCode::Left, KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn backspace_undoes_within_the_word() {
        let mut app = app();
        type_text(&mut app, "tx");
        app.on_key(press(KeyCode::Backspace)).expect("no io");
        assert_eq!(app.test().active_word().input(), "t");
    }

    #[test]
    fn tab_skips_a_word_without_scoring_it() {
        let mut app = app();
        app.on_key(press(KeyCode::Tab)).expect("no io");
        assert_eq!(app.cursor_word(), 1);
        assert_eq!(app.accuracy(), 0.0, "a skip is not a character");
    }

    #[test]
    fn the_test_ends_when_the_clock_runs_out() {
        let mut app = app();
        type_text(&mut app, "the qui");
        app.set_elapsed(Duration::from_secs(app.config.test.time as u64));
        app.tick();
        assert!(app.test().is_finished());
        assert_eq!(app.countdown(), "  0s");
        // A timed test credits the half-typed trailing word: "the " scores 4 and
        // the prefix "qui" of "quick " scores 3.
        assert_eq!(app.test().chars().correct_word, 7);
        assert_eq!(app.test().chars().missed, 0, "a prefix owes nothing");
    }

    #[test]
    fn the_clock_does_not_end_the_test_early() {
        let mut app = app();
        type_text(&mut app, "the");
        app.set_elapsed(Duration::from_secs(u64::from(app.config.test.time) - 1));
        app.tick();
        assert!(!app.test().is_finished());
        assert_eq!(app.countdown(), "  1s");
    }

    #[test]
    fn typing_after_the_test_ends_changes_nothing() {
        let mut app = app();
        type_text(&mut app, "the ");
        app.set_elapsed(Duration::from_secs(u64::from(app.config.test.time)));
        app.tick();
        let chars = app.test().chars();

        type_text(&mut app, "quick brown");
        assert_eq!(app.test().chars(), chars, "a finished test is frozen");
    }

    #[test]
    fn a_restart_clears_the_characters_too() {
        let mut app = app();
        type_text(&mut app, "the quick brown fox ");
        assert!(app.test().chars().correct_word > 0);
        app.on_key(press_mod(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("no io");
        assert_eq!(app.test().chars(), CharCounts::default());
        assert_eq!(app.accuracy(), 0.0);
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

    /// A result you are looking at must not change under you.
    ///
    /// Before the clock was frozen, the wall clock kept feeding `elapsed` after
    /// `finish()`, so the wpm on a finished test decayed for as long as the
    /// screen stayed up.
    #[test]
    fn the_clock_stops_when_the_test_ends() {
        let mut app = app();
        let test = Test::new(vec!["a".into(); 200], Mode::Time, 5);
        app.test = test;

        app.type_char('a');
        app.set_elapsed(Duration::from_secs_f64(5.0));
        app.tick();
        assert!(app.test().is_finished(), "the time was up");
        let wpm_at_the_end = app.wpm();
        let elapsed_at_the_end = app.elapsed_secs();

        // A wall-clock advance and a burst of input must both change nothing.
        std::thread::sleep(Duration::from_millis(30));
        for _ in 0..5 {
            app.tick();
        }
        app.type_char('a');
        app.type_char('b');
        app.test.skip();

        assert_eq!(app.elapsed_secs(), elapsed_at_the_end, "the clock moved on");
        assert_eq!(app.wpm(), wpm_at_the_end, "the result moved on");
        assert!(app.wpm() > 0.0, "a finished test has a result to show");
    }

    #[test]
    fn input_after_the_end_does_not_restart_the_clock() {
        let mut app = app();
        app.test = Test::new(vec!["a".into(); 200], Mode::Words, 200);
        for _ in 0..200 {
            app.type_char('a');
            app.type_char(' ');
        }
        assert!(app.test().is_finished());
        let frozen = app.elapsed_secs();

        std::thread::sleep(Duration::from_millis(20));
        app.type_char('a');
        app.tick();
        assert_eq!(app.elapsed_secs(), frozen);
    }
}
