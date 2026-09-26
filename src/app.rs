//! Application state and event loop.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::Action;
use crate::config::theme::Theme;
use crate::config::Config;
use crate::engine::{Mode, Test};
use crate::screens::{Effect, Row, Screen, ScreenKind, ScreenState};
use crate::stats::{
    build_chart, calculate_wpm, CharCounts, Chart, ChartContext, Event as LogEvent, EventLog,
};
use crate::terminal::Tui;
use crate::words::language::{self, Language};
use crate::words::{self, Options as WordOptions};

/// How often the clock redraws. 100ms keeps the countdown readable without
/// burning CPU on a redraw loop that has nothing to animate.
pub const TICK: Duration = Duration::from_millis(100);

/// The language used until the configured one has been fetched.
///
/// An offline start has to render something, and English is the one list that
/// is always in the binary. The status line says so rather than pretending the
/// test is in the requested language.
const FALLBACK_LANGUAGE: &str = "english";

/// A language being fetched in the background.
struct Pending {
    id: String,
}

pub struct App {
    pub config: Config,
    screen: ScreenState,
    test: Test,
    /// The word list in use, and the id it came from.
    language: Language,
    /// The language the config asks for.
    ///
    /// Kept apart from [`Self::language`] because the two legitimately differ:
    /// a list that is neither embedded nor cached leaves the app on the
    /// fallback, and the status line has to be able to say so.
    requested: String,
    /// A language being fetched, if one is.
    pending: Option<Pending>,
    /// Where a finished fetch is delivered.
    ///
    /// The event loop is synchronous, so a download is run as a task and the
    /// result handed back through this channel, drained by [`Self::tick`].
    inbox: Receiver<Language>,
    /// The other half of `inbox`, kept so a fetch can be started at any time.
    inbox_sender: Sender<Language>,
    /// Wall-clock reading of when the test began, if it has.
    anchor: Option<Instant>,
    /// Everything the typist did, with timings.
    ///
    /// Nothing reads this while a test is running; it exists so the chart and the
    /// per-key statistics can be rebuilt afterwards. The chart is the reason it
    /// is written from the first keystroke rather than at the end: per-second
    /// buckets need per-keystroke timings, and there is no other record of them.
    log: EventLog,
    /// How long the test has been running. Cached so it can be set directly by
    /// tests and by any future replay.
    elapsed: Duration,
    /// Whether the clock is being driven by hand rather than by the wall clock.
    ///
    /// Set by [`Self::set_elapsed`]. Without it a hand-set time would be thrown
    /// away by the next keystroke, which re-anchors to `Instant::now()` — so a
    /// test that set the clock to 1.2s and then typed would see every keystroke
    /// land at zero.
    manual_clock: bool,
    config_path: PathBuf,
    /// Set when the config no longer matches what is on disk.
    dirty: bool,
}

impl App {
    /// Builds an app from a loaded config, seeding a test so the UI is
    /// populated before the first keystroke.
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        let (tx, inbox) = channel();
        let mut app = Self {
            inbox_sender: tx,
            language: language::embedded(FALLBACK_LANGUAGE).expect("english is embedded"),
            requested: FALLBACK_LANGUAGE.to_owned(),
            pending: None,
            inbox,
            config,
            screen: ScreenState::default(),
            test: Test::new(Vec::new(), Mode::Time, 30),
            anchor: None,
            log: EventLog::new(),
            elapsed: Duration::ZERO,
            manual_clock: false,
            config_path,
            dirty: false,
        };
        app.start_language(app.config.test.language.clone());
        app
    }

    // ---- word lists ----------------------------------------------------

    /// The word list in use.
    pub fn language(&self) -> &Language {
        &self.language
    }

    /// A short note about the word list for the status line, e.g. `russian` or
    /// `russian (downloading)`.
    pub fn language_status(&self) -> String {
        match &self.pending {
            Some(pending) => format!("{} ↓", pending.id),
            None if self.language.id() != self.requested => {
                format!("{} (offline)", self.language.id())
            }
            None => self.language.id().to_owned(),
        }
    }

    /// Switches to a language, fetching it first if it is not in the binary.
    ///
    /// The UI keeps working throughout: an unembedded language is fetched in the
    /// background and swapped in when it lands, rather than blocking the first
    /// frame on the network.
    pub fn start_language(&mut self, id: String) {
        self.requested = id.clone();
        if let Some(language) = language::embedded(&id) {
            self.install(language);
            return;
        }
        if !words::fetch::is_available_offline(&id) {
            // Nothing to fetch and nothing cached, so there is no point starting
            // a download the user cannot wait for. Say which list is in use —
            // and still build a test, or the typing screen would come up blank.
            self.pending = None;
            self.regenerate();
            return;
        }
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        let tx: Sender<Language> = self.inbox_tx();
        self.pending = Some(Pending { id: id.clone() });
        // A test built in a synchronous context (a headless test, say) has no
        // runtime to spawn onto; the fallback list stands in until a fetch is
        // started deliberately.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(language) = words::fetch::fetch(&id).await {
                    let _ = tx.send(language);
                }
            });
        }
    }

    /// The channel half, cloned out of the receiver.
    ///
    /// `Receiver` has no clone, so the sender is kept alongside it for the life
    /// of the app instead.
    fn inbox_tx(&self) -> Sender<Language> {
        self.inbox_sender.clone()
    }

    fn install(&mut self, language: Language) {
        self.language = language;
        self.pending = None;
        self.regenerate();
    }

    /// Builds a fresh word list from the current language and config, and starts
    /// a new test with it.
    pub fn regenerate(&mut self) {
        let options = WordOptions {
            count: self.word_count(),
            punctuation: self.config.test.punctuation,
            numbers: self.config.test.numbers,
            difficulty: self.config.test.difficulty,
            zipf: self.config.test.zipf,
        };
        let words = words::Generator::new(&self.language, options).generate();
        self.test = Test::new(words, self.test_mode(), self.test_length());
        self.reset_clock();
        self.log.clear();
    }

    /// How long the test is, read from the config rather than from the test that
    /// happens to exist.
    ///
    /// The engine's own `mode` would be circular — the test is rebuilt from this
    /// very function — and keeping a second copy of the settings in the engine
    /// is how `mode = "words"` in a config file ends up ignored.
    fn test_mode(&self) -> Mode {
        match self.config.test.mode {
            crate::config::Mode::Time => Mode::Time,
            crate::config::Mode::Words => Mode::Words,
            crate::config::Mode::Quote => Mode::Quote,
        }
    }

    /// Seconds for a timed test, words for a word-count one, and nothing for a
    /// quote, whose length is the passage's own.
    fn test_length(&self) -> u32 {
        match self.config.test.mode {
            crate::config::Mode::Time => self.config.test.time,
            crate::config::Mode::Words => self.config.test.words,
            crate::config::Mode::Quote => 0,
        }
    }

    /// How many words the test needs.
    fn word_count(&self) -> usize {
        match self.config.test.mode {
            crate::config::Mode::Time => WordOptions::for_duration(self.config.test.time).count,
            crate::config::Mode::Words => self.config.test.words as usize,
            crate::config::Mode::Quote => 100,
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

    /// The wpm, burst and error series, as far as the test has got.
    ///
    /// Rebuilt from the event log on demand rather than kept up to date, so
    /// there is no way for the chart and the engine to disagree: they are
    /// computed from the same record, and the log is the only input.
    pub fn chart(&self) -> Chart {
        let targets: Vec<String> = self.test.words().iter().map(|w| w.target()).collect();
        build_chart(
            &self.log,
            &ChartContext {
                targets: &targets,
                is_timed: self.test.mode().is_timed(self.test.mode2()),
                end_ms: self.now_ms(),
            },
        )
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
        self.test = Test::new(words, self.test_mode(), self.test_length());
        self.reset_clock();
        self.log.clear();
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
    /// Sets how long the test has been running, and detaches the clock.
    ///
    /// Detaching means three things: the next [`Self::tick`] will not overwrite
    /// the value, the next keystroke will not start the wall clock, and event
    /// timestamps come from `elapsed` rather than from `Instant::now()`. The
    /// event loop never calls it; it exists for tests and for a future replay,
    /// both of which have to drive time by hand. [`Self::regenerate`] and
    /// [`Self::set_words`] put the wall clock back.
    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.anchor = None;
        self.elapsed = elapsed;
        self.manual_clock = true;
    }

    /// Resets the clock and clears the anchor, so time runs again from zero.
    fn reset_clock(&mut self) {
        self.elapsed = Duration::ZERO;
        self.anchor = None;
        self.manual_clock = false;
    }

    /// Advances the clock, ending the test if its time is up.
    ///
    /// The ticker is the only source of time, so the countdown and the timeout
    /// never depend on when a keystroke happened to arrive.
    pub fn tick(&mut self) {
        self.drain_languages();
        if self.test.is_finished() {
            self.freeze_clock();
            self.close_log();
            return;
        }
        if let Some(anchor) = self.anchor {
            self.elapsed = anchor.elapsed();
        }
        self.check_timeout();
        self.close_log();
    }

    /// Picks up a word list a background fetch has finished with.
    fn drain_languages(&mut self) {
        loop {
            match self.inbox.try_recv() {
                Ok(language) => self.install(language),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
            }
        }
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
        } else if !self.manual_clock {
            self.anchor = Some(Instant::now());
        }

        // Recorded before the engine sees the character, because afterwards the
        // position it landed at no longer exists: a space has already ended the
        // word and moved the caret on.
        let word = self.test.active_index();
        let index = self.test.active_word().input_len_utf16();
        let correct = self.test.active_word().accepts(c);
        self.test.input(c);
        // The commit character is a keystroke like any other and has to be in the
        // log: the chart's replay rebuilds each word's input from the log, and a
        // word whose space was never typed would read as unfinished.
        self.log.push(
            self.now_ms(),
            LogEvent::Insert {
                word,
                index,
                correct,
                ch: c,
            },
        );
        self.close_log();
    }

    /// Records a backspace against whichever word it actually lands in.
    ///
    /// The engine walks back into the previous word when the active one is
    /// empty, so the word has to be worked out before the call, not after.
    fn record_backspace(&mut self) {
        let active = self.test.active_index();
        let word = if !self.test.active_word().input().is_empty() {
            active
        } else if active > 0 {
            active - 1
        } else {
            return;
        };
        let index = self
            .test
            .words()
            .get(word)
            .map_or(0, crate::engine::Word::last_typed_index);
        self.log
            .push(self.now_ms(), LogEvent::Delete { word, index });
    }

    /// Closes the log once the test is over, so the chart stops at the right
    /// place rather than at the last keystroke.
    fn close_log(&mut self) {
        if self.test.is_finished() && !self.log.is_finished() {
            self.log.finish(self.now_ms());
        }
    }

    /// Milliseconds since the first keystroke, which is the origin every event
    /// timestamp and every chart boundary is measured from.
    ///
    /// Read from the wall clock while the test runs so that keystrokes inside
    /// one tick keep their real spacing; `tick` is still the only thing that
    /// decides how long the test has been running, so a test driven by
    /// [`Self::set_elapsed`] reports the time it was given.
    fn now_ms(&self) -> f64 {
        match self.anchor {
            Some(anchor) if !self.manual_clock => anchor.elapsed().as_secs_f64() * 1000.0,
            _ => self.elapsed.as_secs_f64() * 1000.0,
        }
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
                self.regenerate();
                false
            }
            Effect::Type(c) => {
                self.type_char(c);
                false
            }
            Effect::Backspace => {
                self.record_backspace();
                self.test.backspace();
                false
            }
            Effect::SkipWord => {
                if !self.test.is_started() && !self.manual_clock {
                    self.anchor = Some(Instant::now());
                }
                self.log.push(
                    self.now_ms(),
                    LogEvent::Skip {
                        word: self.test.active_index(),
                    },
                );
                self.test.skip();
                self.close_log();
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
                let id = self.config.test.language.clone();
                self.start_language(id);
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

/// Cycles through the languages that work with no network.
fn next_language(current: &str) -> String {
    let ids: Vec<&str> = language::embedded_ids().collect();
    let position = ids.iter().position(|id| *id == current);
    match position {
        Some(index) => ids[(index + 1) % ids.len()].to_owned(),
        None => ids.first().copied().unwrap_or(FALLBACK_LANGUAGE).to_owned(),
    }
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

    /// An app whose word list is pinned.
    ///
    /// The app generates real words now, so a test that types a specific string
    /// has to say what it is typing against — otherwise it is testing whatever
    /// the generator happened to produce.
    fn app_with(words: &[&str]) -> App {
        let mut app = app();
        app.set_words(words.iter().map(|w| (*w).to_owned()).collect());
        app
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
        let app = app_with(&["the", "quick", "brown", "fox"]);
        // the(3) quick(5) brown(5) ...
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
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
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
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
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
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
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
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
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
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
        type_text(&mut app, "the");
        app.set_elapsed(Duration::from_secs(u64::from(app.config.test.time) - 1));
        app.tick();
        assert!(!app.test().is_finished());
        assert_eq!(app.countdown(), "  1s");
    }

    #[test]
    fn typing_after_the_test_ends_changes_nothing() {
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
        type_text(&mut app, "the ");
        app.set_elapsed(Duration::from_secs(u64::from(app.config.test.time)));
        app.tick();
        let chars = app.test().chars();

        type_text(&mut app, "quick brown");
        assert_eq!(app.test().chars(), chars, "a finished test is frozen");
    }

    #[test]
    fn a_restart_clears_the_characters_too() {
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
        type_text(&mut app, "the quick brown fox ");
        assert!(app.test().chars().correct_word > 0);
        app.on_key(press_mod(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("no io");
        assert_eq!(app.test().chars(), CharCounts::default());
        assert_eq!(app.accuracy(), 0.0);
    }

    // ---- the event log ---------------------------------------------------

    #[test]
    fn the_log_records_every_keystroke_with_its_position() {
        let mut app = app_with(&["the", "quick"]);
        type_text(&mut app, "the ");
        let events: Vec<_> = app.log.events().to_vec();
        assert_eq!(events.len(), 4, "t, h, e and the commit space");

        let words: Vec<usize> = events.iter().map(|e| e.event.word()).collect();
        assert_eq!(words, [0, 0, 0, 0]);
        let indices: Vec<usize> = events
            .iter()
            .map(|e| match e.event {
                LogEvent::Insert { index, .. } => index,
                other => panic!("expected an insert, got {other:?}"),
            })
            .collect();
        assert_eq!(indices, [0, 1, 2, 3], "the commit lands after the text");
        assert!(events.iter().all(|e| e.ms <= 1000.0), "first second");
    }

    #[test]
    fn the_log_records_a_wrong_character_as_wrong() {
        let mut app = app_with(&["the"]);
        type_text(&mut app, "tge ");
        let wrong: Vec<bool> = app
            .log
            .inserts()
            .map(|e| matches!(e.event, LogEvent::Insert { correct: false, .. }))
            .collect();
        assert_eq!(wrong, [false, true, false, false], "only 'g' is wrong");
    }

    #[test]
    fn a_backspace_into_an_empty_word_is_recorded_against_the_word_left() {
        let mut app = app_with(&["the", "quick"]);
        type_text(&mut app, "the ");
        assert_eq!(app.cursor_word(), 1, "the caret moved on");

        app.on_key(press(KeyCode::Backspace)).expect("no io");
        let last = app.log.events().last().expect("an event");
        assert_eq!(app.cursor_word(), 0, "the caret walked back");
        assert!(
            matches!(last.event, LogEvent::Delete { word: 0, .. }),
            "the deletion belongs to the first word, got {:?}",
            last.event
        );
    }

    #[test]
    fn a_skip_is_recorded_against_the_word_it_skipped() {
        let mut app = app_with(&["the", "quick"]);
        app.on_key(press(KeyCode::Tab)).expect("no io");
        let last = app.log.events().last().expect("an event");
        assert!(matches!(last.event, LogEvent::Skip { word: 0 }));
    }

    #[test]
    fn a_restart_clears_the_log() {
        let mut app = app_with(&["the", "quick"]);
        type_text(&mut app, "the ");
        assert!(!app.log.is_empty());

        app.on_key(press_mod(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("no io");
        assert!(app.log.is_empty(), "a new test starts from nothing");
        assert!(app.chart().is_empty());
    }

    #[test]
    fn the_log_stops_when_the_test_does() {
        let mut app = app_with(&["the", "quick"]);
        type_text(&mut app, "the qui");
        app.set_elapsed(Duration::from_secs(u64::from(app.config.test.time)));
        app.tick();

        assert!(app.test().is_finished());
        assert!(app.log.is_finished());
        let frozen = app.log.recorded_end_ms().expect("the end was recorded");
        // More time and more input must not extend the chart.
        std::thread::sleep(Duration::from_millis(20));
        app.type_char('c');
        app.tick();
        assert_eq!(app.log.recorded_end_ms(), Some(frozen));
    }

    #[test]
    fn the_chart_grows_while_a_test_runs() {
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
        // `set_elapsed` collapses the clock to a single point, so a scripted test
        // can only place keystrokes at the start and at whatever time it sets.
        type_text(&mut app, "the ");
        app.set_elapsed(Duration::from_millis(1100));
        type_text(&mut app, "quick ");
        app.set_elapsed(Duration::from_millis(2400));

        let chart = app.chart();
        assert_eq!(chart.len(), 2, "two whole seconds have gone by");
        assert_eq!(chart.burst[0], 48.0, "four characters in the first second");
        assert_eq!(chart.burst[1], 72.0, "six in the second");
        assert!(chart.err.iter().all(|e| *e == 0), "nothing was mistyped");
        assert!(!chart.has_errors());
    }

    #[test]
    fn an_empty_second_shows_up_as_a_dip_rather_than_being_skipped() {
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
        type_text(&mut app, "the ");
        app.set_elapsed(Duration::from_millis(2100));
        type_text(&mut app, "q");
        app.set_elapsed(Duration::from_millis(4200));
        type_text(&mut app, "uick ");

        let chart = app.chart();
        assert_eq!(chart.len(), 4, "four whole seconds have gone by");
        assert_eq!(chart.burst[0], 48.0, "'the ', four characters");
        assert_eq!(chart.burst[1], 0.0, "nothing was typed in the second");
        assert_eq!(chart.burst[2], 12.0, "one character in the third");
        assert_eq!(chart.burst[3], 0.0, "and none in the fourth");
    }

    #[test]
    fn the_chart_of_a_fresh_app_is_empty() {
        let app = app();
        assert!(app.chart().is_empty());
        assert_eq!(app.chart().final_wpm(), 0.0);
    }

    #[test]
    fn a_word_count_test_is_not_treated_as_timed() {
        // The difference is visible: only on a clock does the half-typed word
        // count, so a words test and a timed test over the same keystrokes give
        // different curves.
        let mut config = Config::default();
        config.test.mode = crate::config::Mode::Words;
        config.test.words = 2;
        let mut app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        app.set_words(vec!["the".to_owned(), "quick".to_owned()]);
        assert_eq!(app.test().mode(), Mode::Words);

        app.set_elapsed(Duration::from_millis(1200));
        type_text(&mut app, "the q");
        app.set_elapsed(Duration::from_millis(2200));
        let chart = app.chart();

        // The first bucket is empty: a hand-set clock puts every keystroke at
        // 1.2s, and the first boundary is 1s.
        assert_eq!(chart.burst[0], 0.0);
        assert_eq!(chart.burst[1], 60.0, "five keystrokes in the second second");
        // 'the ' is four finished characters; the 'q' of "quick" is not a word
        // yet, so it earns nothing until the word is.
        assert_eq!(chart.wpm[1], 24.0, "4 characters over 2 seconds");
    }

    #[test]
    fn a_timed_test_credits_the_word_being_typed_on_the_chart() {
        let mut app = app_with(&["the", "quick"]);
        app.set_elapsed(Duration::from_millis(1200));
        type_text(&mut app, "the q");
        app.set_elapsed(Duration::from_millis(2200));

        // Same keystrokes as above, but on a clock the 'q' counts as a prefix of
        // "quick ", so the average is a character higher.
        assert_eq!(app.chart().wpm[1], 30.0, "5 characters over 2 seconds");
    }

    #[test]
    fn the_configured_mode_reaches_the_engine() {
        // The mode used to be read back off the test being rebuilt, so
        // `mode = "words"` in a config file was silently ignored.
        for (mode, words, expected) in [
            (crate::config::Mode::Time, 0u32, 10u32),
            (crate::config::Mode::Words, 7, 7),
        ] {
            let mut config = Config::default();
            config.test.mode = mode;
            config.test.time = 10;
            config.test.words = words;
            let app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
            assert_eq!(app.test().mode2(), expected, "{mode:?}");
        }
    }

    #[test]
    fn an_unavailable_language_still_produces_a_test_to_type() {
        // Nothing was fetched, so `start_language` returns early — and used to
        // return before building a test, leaving the typing screen blank.
        let mut config = Config::default();
        config.test.language = "klingon".to_owned();
        let app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        assert!(!app.test().words().is_empty());
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
    // ---- word generation ------------------------------------------------

    #[test]
    fn a_new_app_generates_a_test_in_the_configured_language() {
        let mut config = Config::default();
        config.test.language = "russian".to_owned();
        config.test.time = 15;
        let app = App::new(config, PathBuf::from("/nonexistent/config.toml"));

        assert_eq!(app.language().id(), "russian");
        assert_eq!(app.language_status(), "russian");
        let words = app.test().words();
        assert!(!words.is_empty(), "the UI would be empty");
        // A 15s test gets the same generous batch a 30s one does, so a slow
        // typist cannot run off the end of the list mid-test.
        assert_eq!(words.len(), 100);
        for word in words {
            let text = word.text();
            assert!(text.ends_with(' '), "{text:?} lost its commit character");
            assert!(
                !text.trim_end_matches(' ').is_empty(),
                "a word cannot be blank"
            );
        }
    }

    #[test]
    fn a_restart_makes_a_different_test() {
        let mut app = app();
        let before: Vec<String> = app
            .test()
            .words()
            .iter()
            .map(|w| w.text().to_owned())
            .collect();
        app.regenerate();
        let after: Vec<String> = app
            .test()
            .words()
            .iter()
            .map(|w| w.text().to_owned())
            .collect();
        assert_eq!(before.len(), after.len());
        assert_ne!(before, after, "a restart should shuffle the word list");
    }

    #[test]
    fn switching_language_swaps_the_word_list() {
        let mut app = app();
        assert_eq!(app.language().id(), "english");
        app.start_language("german".to_owned());
        assert_eq!(app.language().id(), "german");
        assert_eq!(app.language_status(), "german");
    }

    /// A language that is neither embedded nor cached cannot be fetched from a
    /// synchronous test, and the app has to say which list it is actually using
    /// rather than quietly typing German in an English test.
    #[test]
    fn an_unavailable_language_says_so_instead_of_lying() {
        let mut config = Config::default();
        config.test.language = "klingon".to_owned();
        let app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        assert_eq!(
            app.language().id(),
            FALLBACK_LANGUAGE,
            "it should fall back rather than render nothing"
        );
        assert!(
            app.language_status().contains("offline"),
            "got {:?}",
            app.language_status()
        );
    }

    #[test]
    fn word_count_follows_the_mode() {
        let mut config = Config::default();
        config.test.mode = crate::config::Mode::Words;
        config.test.words = 7;
        let app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        assert_eq!(app.test().words().len(), 7);
    }

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
