//! Application state and event loop.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::Action;
use crate::api;
use crate::config::bar::{self, on_off, step, Bar, Field, LengthUnit};
use crate::config::theme::Theme;
use crate::config::Config;
use crate::engine::{Mode, Test};
use crate::screens::{Effect, Row, Screen, ScreenKind, ScreenState};
use crate::stats::result::{score as score_test, Scoring};
use crate::stats::{
    build_chart, calculate_wpm, CharCounts, Chart, ChartContext, Event as LogEvent, EventLog,
    TestResult,
};
use crate::terminal::Tui;
use crate::words::language::{self, Language};
use crate::words::quotes::{self, QuoteList};
use crate::words::{self, variants, Options as WordOptions};

/// How often the clock redraws. 100ms keeps the countdown readable without
/// burning CPU on a redraw loop that has nothing to animate.
pub const TICK: Duration = Duration::from_millis(100);

/// The language used until the configured one has been fetched.
///
/// An offline start has to render something, and English is the one list that
/// is always in the binary. The status line says so rather than pretending the
/// test is in the requested language.
const FALLBACK_LANGUAGE: &str = "english";

/// Milliseconds since the Unix epoch.
///
/// Used for the result's `timestamp`, which the server rounds to the second and
/// buckets by day, so it has to be wall time rather than the test's own clock.
fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

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
    /// Where a finished quote download is delivered.
    ///
    /// A second channel rather than a shared one because the two payloads are
    /// different types: a word list and a quote list are not variants of a
    /// download, and a shared enum would make every reader handle the case it
    /// does not care about.
    quote_inbox: Receiver<QuoteList>,
    quote_sender: Sender<QuoteList>,
    /// What the terminal behind the app is capable of, decided once at startup.
    ///
    /// Read from the environment rather than queried: see
    /// [`crate::config::terminal`] for why, and for what is given up by not
    /// asking the terminal over OSC 11.
    terminal: crate::config::Terminal,
    /// The settings bar: which field is selected.
    ///
    /// Separate from the settings screen, which is where the values that are not
    /// a short list of choices live — the ApeKey, the custom text.
    bar: Bar,
    /// The quote list for a quote test, once it has been fetched.
    quotes: Option<QuoteList>,
    /// The language whose quotes are being fetched right now, if any.
    quote_pending: Option<String>,
    /// Why there is no quote, for the status line.
    quote_note: Option<String>,
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
    /// Whether the results screen has already been shown for this test.
    results_shown: bool,
    /// When the test began, in milliseconds since the epoch, for the result's
    /// `timestamp`. Taken from the system clock rather than from the test clock:
    /// the server uses it to bucket results by day, so it has to be wall time.
    started_at_ms: Option<i64>,
}

impl App {
    /// Builds an app from a loaded config, seeding a test so the UI is
    /// populated before the first keystroke.
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        let (tx, inbox) = channel::<Language>();
        let (quote_tx, quote_inbox) = channel::<QuoteList>();
        let mut app = Self {
            inbox_sender: tx,
            quote_sender: quote_tx,
            quote_inbox,
            language: language::embedded(FALLBACK_LANGUAGE).expect("english is embedded"),
            requested: FALLBACK_LANGUAGE.to_owned(),
            pending: None,
            terminal: crate::config::Terminal::from_env(),
            bar: Bar::idle(),
            quotes: None,
            quote_pending: None,
            quote_note: None,
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
            results_shown: false,
            started_at_ms: None,
        };
        app.start_language(app.config.test.language.clone());
        app.start_quotes();
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

    /// The language being downloaded right now, if any.
    pub fn pending_language(&self) -> Option<&str> {
        self.pending.as_ref().map(|pending| pending.id.as_str())
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

    /// A clone of the quote receiver's sender half, for a fetch to report back on.
    fn quote_tx(&self) -> Sender<QuoteList> {
        self.quote_sender.clone()
    }

    fn install(&mut self, language: Language) {
        self.language = language;
        self.pending = None;
        self.regenerate();
    }

    /// Builds a fresh word list from the current language and config, and starts
    /// a new test with it.
    ///
    /// Three of the five modes do not go through the generator at all: a custom
    /// test types the user's own passage, a quote test a published one, and zen
    /// has no target. Running those through the generator would apply punctuation
    /// and difficulty rules to text that already has its own.
    pub fn regenerate(&mut self) {
        let options = WordOptions {
            count: self.word_count(),
            punctuation: self.config.test.punctuation,
            numbers: self.config.test.numbers,
            difficulty: self.config.test.difficulty,
            zipf: self.config.test.zipf,
        };
        let words = match self.config.test.mode {
            crate::config::Mode::Custom => self.custom_words(),
            crate::config::Mode::Quote => self.quote_words(),
            // Zen is typed against nothing, so one blank word is all it needs to
            // start; the engine makes the rest as they are committed.
            crate::config::Mode::Zen => vec![String::new()],
            crate::config::Mode::Time | crate::config::Mode::Words => {
                words::Generator::new(&self.language, options).generate()
            }
        };
        self.test = Test::new(words, self.test_mode(), self.test_length());
        self.reset_clock();
        self.log.clear();
        self.results_shown = false;
    }

    /// How long the test is, read from the config rather than from the test that
    /// happens to exist.
    ///
    /// The engine's own `mode` would be circular — the test is rebuilt from this
    /// very function — and keeping a second copy of the settings in the engine
    /// is how `mode = "words"` in a config file ends up ignored.
    fn test_mode(&self) -> Mode {
        use crate::config::Mode as ConfigMode;
        match self.config.test.mode {
            ConfigMode::Time => Mode::Time,
            ConfigMode::Words => Mode::Words,
            ConfigMode::Quote => Mode::Quote,
            ConfigMode::Zen => Mode::Zen,
            ConfigMode::Custom => Mode::Custom,
        }
    }

    /// Seconds for a timed test, words for a word-count one, and nothing for a
    /// quote, whose length is the passage's own.
    fn test_length(&self) -> u32 {
        use crate::config::Mode as ConfigMode;
        match self.config.test.mode {
            ConfigMode::Time => self.config.test.time,
            ConfigMode::Words => self.config.test.words,
            // A passage's length is the passage's own, and zen has none.
            ConfigMode::Quote | ConfigMode::Custom | ConfigMode::Zen => 0,
        }
    }

    /// The passage a custom test types.
    ///
    /// The **first** line, always. The website picks one of the pasted lines at
    /// random, which is fine when the test is a one-off but makes a passage
    /// impossible to come back to — and a passage you cannot come back to is not
    /// something you can practise. So this takes the first line and leaves the
    /// choice to the order they are written in the file.
    fn custom_words(&self) -> Vec<String> {
        let Some(passage) = self.config.test.custom_text.first() else {
            // No text at all. An empty test would finish instantly and report a
            // result of nothing, so the fallback is a word the typist has to
            // delete, which is visible.
            return vec!["custom text is empty".to_owned()];
        };
        let words: Vec<String> = passage
            .split(' ')
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect();
        if words.is_empty() {
            return vec!["custom text is empty".to_owned()];
        }
        words
    }

    /// The quote a quote test types.
    fn quote_words(&self) -> Vec<String> {
        let Some(list) = &self.quotes else {
            return vec!["quotes are still downloading".to_owned()];
        };
        let length = self.config.test.quote_length;
        match list.pick(length) {
            Some(quote) => quote
                .text_for(false)
                .split(' ')
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect(),
            None => {
                // Say which buckets are empty rather than "no quotes found": a
                // language with no long quotes and a language with no quotes at
                // all are different problems.
                let missing = list
                    .counts()
                    .into_iter()
                    .filter(|(bucket, count)| *bucket != length && *count > 0)
                    .map(|(bucket, _)| bucket.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if missing.is_empty() {
                    vec![format!("no quotes for {} in this language", list.language)]
                } else {
                    vec![format!(
                        "no {} quotes; this language has {}",
                        length.as_str(),
                        missing
                    )]
                }
            }
        }
    }

    /// Starts a quote download for a quote test, if one is needed.
    ///
    /// Same shape as [`Self::start_language`]: the download runs in the
    /// background and the test is built with a placeholder until it lands, so the
    /// first frame never waits on the network. A 2.3 MB file is a long wait and
    /// blocking on it would look like a hang.
    pub fn start_quotes(&mut self) {
        if self.config.test.mode != crate::config::Mode::Quote {
            return;
        }
        let base = variants::base_of(&self.config.test.language).to_owned();
        if let Some(list) = &self.quotes {
            if list.language == base {
                self.quote_note = None;
                self.regenerate();
                return;
            }
        }
        // Already cached: use it now rather than going near the network.
        if quotes::is_available_offline(&base) {
            match quotes::load_cached(&base) {
                Ok(list) if !list.is_empty() => {
                    self.quotes = Some(list);
                    self.quote_pending = None;
                    self.quote_note = None;
                    self.regenerate();
                    return;
                }
                Ok(_) => {
                    self.quotes = None;
                    self.quote_pending = None;
                    self.quote_note = Some("the quote file was empty".to_owned());
                    return;
                }
                Err(err) => {
                    self.quote_note = Some(err.to_string());
                    return;
                }
            }
        }
        self.quotes = None;
        self.quote_note = None;
        if self.quote_pending.as_deref() == Some(base.as_str()) {
            return;
        }
        self.quote_pending = Some(base.clone());
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let tx = self.quote_tx();
        handle.spawn(async move {
            if let Ok(list) = quotes::fetch(&base).await {
                let _ = tx.send(list);
            }
        });
    }

    /// A short note about the quote list, for the status line.
    pub fn quote_status(&self) -> Option<String> {
        if let Some(language) = &self.quote_pending {
            return Some(format!("{language} quotes ↓"));
        }
        self.quote_note.clone()
    }

    /// Picks up a quote list a background download has finished with.
    fn drain_quotes(&mut self) {
        while let Ok(list) = self.quote_inbox.try_recv() {
            self.quote_pending = None;
            self.quote_note = if list.is_empty() {
                Some("the quote file was empty".to_owned())
            } else {
                None
            };
            self.quotes = (!list.is_empty()).then_some(list);
            self.regenerate();
        }
    }

    /// How many words the test needs.
    fn word_count(&self) -> usize {
        use crate::config::Mode as ConfigMode;
        match self.config.test.mode {
            ConfigMode::Time => WordOptions::for_duration(self.config.test.time).count,
            ConfigMode::Words => self.config.test.words as usize,
            // A quote is whatever the quote turns out to be, and zen needs only
            // enough words to get going: it makes its own as you go.
            ConfigMode::Quote | ConfigMode::Custom => 100,
            ConfigMode::Zen => 25,
        }
    }

    // ---- the settings bar -----------------------------------------------

    /// The fields the bar shows, in order, for the current mode.
    pub fn bar_fields(&self) -> Vec<Field> {
        Bar::fields(self.config.test.mode)
    }

    /// The field the bar has selected.
    ///
    /// Always a field the bar actually shows: changing mode can shorten the bar
    /// under a selection, so the index is clamped rather than trusted.
    pub fn bar_field(&self) -> Field {
        let fields = self.bar_fields();
        fields
            .get(self.bar.selected())
            .copied()
            // A bar with no fields cannot be navigated, and this keeps the
            // accessor total rather than an Option every screen has to unwrap.
            .unwrap_or(Field::Mode)
    }

    /// Which field is selected, as an index into [`Self::bar_fields`].
    pub fn bar_selection(&self) -> usize {
        self.bar
            .selected()
            .min(self.bar_fields().len().saturating_sub(1))
    }

    /// Moves the bar's selection, clamped to the fields that exist.
    pub fn move_bar_selection(&mut self, by: isize) {
        self.bar.move_selection(by, self.bar_fields().len());
    }

    /// What a field currently says.
    ///
    /// The mode and the length are shown as the value alone, everything else as
    /// `label value`, which is the arrangement the website's buttons use.
    pub fn bar_value(&self, field: Field) -> String {
        let test = &self.config.test;
        match field {
            Field::Mode => test.mode.bar_label().to_owned(),
            Field::Length => match test.mode.length_unit() {
                Some(unit) => unit.render(self.bar_length()),
                None => "∞".to_owned(),
            },
            Field::QuoteLength => test.quote_length.as_str().to_owned(),
            Field::Punctuation => on_off(test.punctuation),
            Field::Numbers => on_off(test.numbers),
            Field::Difficulty => test.difficulty.as_str().to_owned(),
            Field::CustomText => match test.custom_text.first() {
                Some(text) => {
                    let words = text.split(' ').filter(|w| !w.is_empty()).count();
                    format!("{} words", words)
                }
                None => "not set".to_owned(),
            },
            Field::Language => {
                let label = variants::Language {
                    id: test.language.clone(),
                    base: variants::base_of(&test.language).to_owned(),
                    words: self.language.words.len() as u32,
                    embedded: language::embedded(&test.language).is_some(),
                };
                label.label()
            }
            Field::Blind => on_off(test.blind),
        }
    }

    /// The bar's length, as the mode counts it.
    fn bar_length(&self) -> u32 {
        match self.config.test.mode {
            crate::config::Mode::Time => self.config.test.time,
            crate::config::Mode::Words => self.config.test.words,
            _ => 0,
        }
    }

    /// Changes a field by `by` steps, and rebuilds the test so the change can be
    /// seen before it is typed.
    ///
    /// Fields with no short list of choices — the custom text — are left alone
    /// rather than cycled: they are typed on the settings screen, and a field
    /// that silently does nothing when you press the key is worse than one that
    /// says so.
    pub fn change_bar_field(&mut self, field: Field, by: i8) {
        let by = by as isize;
        // Read before the borrow: switching language needs `self` immutably to
        // work out which variants exist, and the config mutably to store the new
        // one, so the two cannot be held at once.
        let variants_now = (field == Field::Language).then(|| self.language_variants());
        let test = &mut self.config.test;
        let mut needs_words = true;
        let mut needs_quotes = false;
        let mut needs_language = false;

        match field {
            Field::Mode => {
                let current = bar::MODES.iter().position(|m| *m == test.mode);
                test.mode = bar::MODES[step(current, by, bar::MODES.len())];
                needs_quotes = test.mode == crate::config::Mode::Quote;
            }
            Field::Length => match test.mode.length_unit() {
                Some(LengthUnit::Seconds) => {
                    let current = bar::TIMES.iter().position(|t| *t == test.time);
                    test.time = bar::TIMES[step(current, by, bar::TIMES.len())];
                }
                Some(LengthUnit::Words) => {
                    let current = bar::WORD_COUNTS.iter().position(|w| *w == test.words);
                    test.words = bar::WORD_COUNTS[step(current, by, bar::WORD_COUNTS.len())];
                }
                // Zen and quote have no length to choose.
                None => return,
            },
            Field::QuoteLength => {
                let current = bar::QUOTE_LENGTHS
                    .iter()
                    .position(|q| *q == test.quote_length);
                test.quote_length = bar::QUOTE_LENGTHS[step(current, by, bar::QUOTE_LENGTHS.len())];
            }
            Field::Punctuation => test.punctuation = !test.punctuation,
            Field::Numbers => test.numbers = !test.numbers,
            Field::Blind => test.blind = !test.blind,
            Field::Difficulty => {
                let current = bar::DIFFICULTIES.iter().position(|d| *d == test.difficulty);
                test.difficulty = bar::DIFFICULTIES[step(current, by, bar::DIFFICULTIES.len())];
            }
            Field::CustomText => return,
            Field::Language => {
                needs_language = true;
                let Some(ids) = variants_now else {
                    return;
                };
                let Some(current) = ids.iter().position(|id| *id == test.language) else {
                    return;
                };
                let next = step(Some(current), by, ids.len());
                test.language = ids[next].clone();
            }
        }

        self.dirty = true;
        // Zen makes its own words, so there is nothing to build.
        if test.mode == crate::config::Mode::Zen {
            needs_words = false;
        }
        if needs_language {
            let id = self.config.test.language.clone();
            self.start_language(id);
            return;
        }
        if needs_quotes {
            self.start_quotes();
            return;
        }
        if needs_words {
            self.regenerate();
        }
    }

    /// The word-list ids the bar cycles through for the current base language.
    ///
    /// The base list plus every sized variant that is embedded or already cached.
    /// A variant that is neither is not offered, because a picker full of entries
    /// that fail to download is worse than one that grows as the cache does.
    pub fn language_variants(&self) -> Vec<String> {
        let base = variants::base_of(&self.config.test.language).to_owned();
        let cached = |id: &str| words::fetch::is_available_offline(id);
        variants::available(&base, &cached)
    }

    // ---- accessors used by screens -------------------------------------

    pub fn theme(&self) -> Theme {
        self.config.theme.for_terminal(&self.terminal)
    }

    /// What the terminal says about itself, for the status line.
    pub fn terminal(&self) -> &crate::config::Terminal {
        &self.terminal
    }

    /// The test in progress.
    pub fn test(&self) -> &Test {
        &self.test
    }

    /// Index of the word the caret is on.
    pub fn cursor_word(&self) -> usize {
        self.test.active_index()
    }

    /// The character under the caret, if the test has not run out of words.
    pub fn current_char(&self) -> Option<char> {
        let word = self.test.active_word();
        word.char_at(word.input_len_utf16())
    }

    /// The display width of every word, for laying the word pane out.
    ///
    /// The pane wraps the words itself rather than letting ratatui do it, because
    /// it needs to know which line the active word landed on in order to put it
    /// in the middle — and a wrapping widget will not say.
    pub fn word_widths(&self) -> Vec<usize> {
        use unicode_width::UnicodeWidthStr;
        self.test
            .words()
            .iter()
            .map(|word| word.text().width())
            .collect()
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

    /// Live words per minute.
    ///
    /// `correct_word`, not `all_correct`: the website's live counter divides
    /// `getChars(...).correctWord` by the elapsed time, and so does its chart.
    /// Counting every matching character instead makes the number disagree with
    /// the site's and with the chart drawn right above it, which is worse than
    /// either being a different measure.
    pub fn wpm(&self) -> f64 {
        let seconds = self.elapsed_secs();
        if seconds <= 0.0 {
            return 0.0;
        }
        calculate_wpm(f64::from(self.live_chars().correct_word), seconds)
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
        let targets = self.targets();
        build_chart(
            &self.log,
            &ChartContext {
                targets: &targets,
                is_timed: self.test.mode().is_timed(self.test.mode2()),
                end_ms: self.now_ms(),
            },
        )
    }

    /// The finished test, prepared for submission.
    ///
    /// Computed and kept even though it cannot be sent, because it is what a
    /// submission needs and because a hash that is quietly wrong is worse than
    /// one that is verifiably right. [`Self::submit`] is what actually tries, and
    /// it reports that there was nowhere to send it.
    pub fn submission_body(&self) -> Option<api::submission::ResultBody> {
        let result = self.result()?;
        let test = &self.config.test;
        Some(api::submission::body_for(
            &result,
            api::submission::TestSettings {
                mode: test.mode.as_str(),
                mode2: self.mode2_string(),
                language: api::language_for_submission(&test.language),
                difficulty: test.difficulty.as_str(),
                punctuation: test.punctuation,
                numbers: test.numbers,
                blind: test.blind,
                timestamp_ms: self.timestamp_ms(),
            },
        ))
    }

    /// What happened when a finished test was offered to monkeytype.
    pub fn submit(&self) -> api::submission::SubmitOutcome {
        api::submission::SubmitOutcome::Nowhere {
            destination: api::submission::Destination::Monkeytype,
            reason: api::submission::Destination::Monkeytype.describe(),
        }
    }

    /// `mode2` as the server wants it: a string, because that is what the site's
    /// own payload carries even where the schema also accepts a number.
    fn mode2_string(&self) -> String {
        match self.config.test.mode {
            crate::config::Mode::Quote | crate::config::Mode::Custom => {
                // A passage's length is its own; the site sends the quote id here,
                // which a client that picks a quote locally does have.
                self.test.words().len().to_string()
            }
            _ => self.test_length().to_string(),
        }
    }

    /// When the test ended, in milliseconds since the epoch.
    fn timestamp_ms(&self) -> i64 {
        (self.started_at_ms.unwrap_or_else(epoch_ms) / 1000) * 1000
    }

    /// The finished test, scored. `None` until one has been run.
    ///
    /// A pure function of the log, so it is recomputed on demand: there is no
    /// snapshot to keep in step with the engine, and a test that is still
    /// running scores to whatever it has done so far rather than not at all.
    pub fn result(&self) -> Option<TestResult> {
        if !self.test.is_started() {
            return None;
        }
        Some(score_test(Scoring {
            log: &self.log,
            targets: &self.targets(),
            is_timed: self.test.mode().is_timed(self.test.mode2()),
            duration_secs: self.elapsed_secs(),
            chars: self.test.chars(),
            inputs: self.test.inputs(),
        }))
    }

    /// Every target word, each including the character that ends it.
    fn targets(&self) -> Vec<String> {
        self.test.words().iter().map(|word| word.target()).collect()
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
            Mode::Words | Mode::Quote | Mode::Custom => {
                let total = self.test.words().len() as u32;
                format!(
                    "{}/{}",
                    self.cursor_word().min(total as usize) as u32,
                    total
                )
            }
            // Zen has no total: it makes words as it goes. A running count of
            // words typed is the one number that is meaningful in a mode with no
            // target, so that is what this shows.
            Mode::Zen => format!("{}w", self.cursor_word()),
        }
    }

    /// The screen currently on display.
    pub fn screen_kind(&self) -> ScreenKind {
        self.screen.kind()
    }

    /// Feeds one key through the real key path, as the event loop would.
    ///
    /// The event loop is the only caller in production; this exists so a test can
    /// check that a screen really drops a key rather than only that its `handle`
    /// returns nothing.
    pub fn press(&mut self, code: KeyCode) -> bool {
        self.on_key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
        .unwrap_or(false)
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
        self.results_shown = false;
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
        self.started_at_ms = None;
    }

    /// Advances the clock, ending the test if its time is up.
    ///
    /// The ticker is the only source of time, so the countdown and the timeout
    /// never depend on when a keystroke happened to arrive.
    pub fn tick(&mut self) {
        self.drain_languages();
        self.drain_quotes();
        if !self.test.is_finished() {
            if let Some(anchor) = self.anchor {
                self.elapsed = anchor.elapsed();
            }
            self.check_timeout();
        }
        // Both in one place, so the tick that *ends* the test shows the result
        // as well as the ones after it.
        self.settle();
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
            self.started_at_ms = Some(epoch_ms());
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
        self.settle();
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

    /// Everything that has to happen once the test may have just ended.
    ///
    /// Called from the ticker *and* after every keystroke, because a word-count
    /// test ends on a keystroke: leaving it to the next tick means a test that
    /// runs out of words sits on the typing screen showing a finished test, and
    /// in a headless caller there may not be a next tick at all.
    fn settle(&mut self) {
        if !self.test.is_finished() {
            return;
        }
        self.freeze_clock();
        // The log closes before the result is shown, or the chart stops at the
        // last keystroke and the test's final second goes missing.
        self.close_log();
        self.show_results();
    }

    /// Moves to the results screen the first time a test ends.
    ///
    /// Only from the typing screen, and only once: a finished test stays on its
    /// result while the user reads it, instead of yanking them back the moment
    /// the ticker notices the clock ran out again. This is the bug the old
    /// client had in reverse — a test that ended and left you staring at a
    /// frozen word list with no score anywhere.
    fn show_results(&mut self) {
        if !self.test.is_finished() || self.results_shown {
            return;
        }
        if self.screen.kind() != ScreenKind::Typing {
            return;
        }
        self.results_shown = true;
        self.apply(Effect::Switch(ScreenKind::Results));
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
                    ScreenKind::Results => ScreenState::Results(Default::default()),
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
                self.settle();
                false
            }
            Effect::Adjust(row) => {
                self.adjust(row);
                false
            }
            Effect::MoveBar(by) => {
                self.move_bar_selection(isize::from(by));
                false
            }
            Effect::ChangeBar(by) => {
                let field = self.bar_field();
                self.change_bar_field(field, by);
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
            ScreenState::Results(screen) => screen.handle(action),
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

    /// The default theme is `auto`, so "toggling the theme" moves away from it
    /// rather than along the list from the top.
    #[test]
    fn toggling_a_setting_marks_the_config_dirty() {
        let mut app = app();
        assert!(!app.is_dirty());
        app.on_key(press(KeyCode::F(2))).expect("no io");
        // Row 0 is the theme; right cycles it.
        app.on_key(press(KeyCode::Right)).expect("no io");
        assert!(app.is_dirty());
        assert_eq!(
            app.config.theme,
            crate::config::theme::ThemeName::Monkeytype,
            "auto is the default, so the first step is the first real theme"
        );
    }

    /// `auto` is the default, and it is the only setting that can be right on a
    /// machine nobody has configured: the colour depth and the background are
    /// both things the terminal usually knows and this client did not.
    #[test]
    fn the_default_theme_is_auto() {
        assert_eq!(
            Config::default().theme,
            crate::config::theme::ThemeName::Auto
        );
        let app = app();
        assert_eq!(
            app.theme().background,
            crate::config::theme::ThemeName::Monkeytype
                .resolve()
                .background,
            "an unknown terminal gets a dark theme"
        );
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

    /// The counter and the chart are the same number, drawn twice.
    ///
    /// The website divides `correctWord` by the elapsed time for its live
    /// counter *and* for the last point of the chart. Anything else — counting
    /// every matching character, say — puts two different figures on one screen.
    #[test]
    fn the_live_wpm_is_the_last_point_of_the_chart() {
        let mut app = app_with(&["the", "quick", "brown", "fox"]);
        type_text(&mut app, "the quick ");
        // On a whole second the two are directly comparable, and must match.
        for seconds in [2u64, 3, 5] {
            app.set_elapsed(Duration::from_secs(seconds));
            let chart = app.chart();
            let last = *chart
                .wpm
                .last()
                .unwrap_or_else(|| panic!("no bucket at {seconds}s"));
            assert_eq!(
                app.wpm(),
                last,
                "at {seconds}s the counter and the chart disagree"
            );
        }
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

    // ---- results --------------------------------------------------------

    /// An app whose 10-second test has just run out.
    fn finished(seconds: u64) -> App {
        let mut config = Config::default();
        config.test.time = 10;
        let mut app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        app.set_words(
            "the quick brown fox jumps over the lazy dog"
                .split(' ')
                .map(str::to_owned)
                .collect(),
        );
        for (ms, text) in [(0u64, "the quick "), (1200, "brown fox "), (2400, "jumps ")] {
            app.set_elapsed(Duration::from_millis(ms));
            for c in text.chars() {
                app.type_char(c);
            }
        }
        app.set_elapsed(Duration::from_secs(seconds));
        app.tick();
        app
    }

    #[test]
    fn a_test_that_runs_out_shows_its_result() {
        let app = finished(12);
        assert!(app.test().is_finished());
        assert_eq!(app.screen_kind(), ScreenKind::Results);
        assert!(app.result().is_some());
    }

    #[test]
    fn the_result_screen_stays_up_instead_of_being_re_shown() {
        let mut app = finished(12);
        for _ in 0..5 {
            app.tick();
        }
        assert_eq!(app.screen_kind(), ScreenKind::Results);
    }

    /// The old client's worst bug, in its other form: a test that ended and left
    /// you staring at a frozen word list with the score nowhere on screen.
    #[test]
    fn a_finished_test_reports_a_score_rather_than_a_hanging_clock() {
        let app = finished(12);
        let result = app.result().expect("a result");
        assert!(result.wpm > 0.0, "got {}", result.wpm);
        assert!(result.accuracy > 0.0);
        assert_eq!(
            result.duration_secs, 12.0,
            "the clock stopped where it ended"
        );
        assert!(!result.chart.is_empty());
    }

    #[test]
    fn an_unstarted_test_has_no_result_to_show() {
        let app = app();
        assert!(app.result().is_none());
    }

    #[test]
    fn a_result_is_scored_from_the_log_and_does_not_drift() {
        let mut app = finished(12);
        let first = app.result().expect("a result");
        std::thread::sleep(Duration::from_millis(20));
        for _ in 0..5 {
            app.tick();
        }
        assert_eq!(app.result().expect("a result"), first);
    }

    #[test]
    fn the_results_screen_ignores_typing() {
        let mut app = finished(12);
        let before = app.result().expect("a result");
        for c in "more typing".chars() {
            app.press(KeyCode::Char(c));
        }
        assert_eq!(app.result().expect("a result"), before);
        assert_eq!(app.screen_kind(), ScreenKind::Results);
    }

    #[test]
    fn esc_from_the_result_starts_a_new_test() {
        let mut app = finished(12);
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
        assert!(!app.test().is_started(), "a new test has not begun");
        assert_eq!(app.elapsed_secs(), 0.0);
    }

    #[test]
    fn ctrl_r_from_the_result_starts_a_new_test_too() {
        let mut app = finished(12);
        app.on_key(press_mod(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Typing);
        assert!(!app.test().is_started());
    }

    #[test]
    fn a_word_count_test_shows_a_result_when_the_words_run_out() {
        let mut config = Config::default();
        config.test.mode = crate::config::Mode::Words;
        config.test.words = 3;
        let mut app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        app.set_words(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        app.set_elapsed(Duration::from_millis(500));
        for _ in 0..3 {
            app.type_char('a');
            app.type_char(' ');
        }
        assert!(app.test().is_finished(), "the words ran out");
        assert_eq!(app.screen_kind(), ScreenKind::Results);
        // Two characters of `correct_word` over half a second. Only the first
        // word was right — the other two were typed as "a" — and a word-count
        // test credits nothing that is not exact.
        assert_eq!(app.result().expect("a result").wpm, 48.0);
    }

    // ---- the settings bar -----------------------------------------------

    fn app_with_test(mode: crate::config::Mode) -> App {
        let mut config = Config::default();
        config.test.mode = mode;
        App::new(config, PathBuf::from("/nonexistent/config.toml"))
    }

    /// Every mode the bar can be in, so a rule about the bar is checked against
    /// all of them rather than against whichever one was on screen.
    fn every_mode() -> Vec<App> {
        bar::MODES.iter().map(|m| app_with_test(*m)).collect()
    }

    #[test]
    fn the_bar_shows_the_mode_first_in_every_mode() {
        for app in every_mode() {
            assert_eq!(
                app.bar_field(),
                Field::Mode,
                "in {:?}",
                app.config.test.mode
            );
        }
    }

    /// The selection is an index into a list whose length changes with the mode.
    /// Leaving it past the end means a bar with nothing selected and arrows that
    /// go nowhere.
    #[test]
    fn the_selection_survives_a_change_of_mode() {
        let mut app = app_with_test(crate::config::Mode::Words);
        // Walk to the far end of the longest bar.
        for _ in 0..10 {
            app.move_bar_selection(1);
        }
        assert_eq!(app.bar_selection(), app.bar_fields().len() - 1);

        for mode in bar::MODES {
            app.change_bar_field(Field::Mode, 0);
            // Whatever the mode now is, the selection names a field that exists.
            let fields = app.bar_fields();
            assert!(
                app.bar_selection() < fields.len(),
                "{mode:?} left the selection past the end"
            );
        }
    }

    #[test]
    fn the_arrows_move_the_selection_and_never_escape_the_bar() {
        let mut app = app_with_test(crate::config::Mode::Time);
        for _ in 0..20 {
            app.move_bar_selection(-1);
        }
        assert_eq!(app.bar_selection(), 0, "it went off the left edge");
        for _ in 0..20 {
            app.move_bar_selection(1);
        }
        assert_eq!(app.bar_selection(), app.bar_fields().len() - 1);
    }

    #[test]
    fn up_and_down_change_the_selected_field() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.move_bar_selection(1); // the length
        assert_eq!(app.bar_field(), Field::Length);
        assert_eq!(app.bar_value(Field::Length), "30s");

        app.change_bar_field(app.bar_field(), 1);
        assert_eq!(app.config.test.time, 60);
        app.change_bar_field(app.bar_field(), -1);
        assert_eq!(
            app.config.test.time, 30,
            "a value is a ring, so up goes back"
        );
    }

    /// A value has to come back to where it started, or one direction is a dead
    /// end.
    #[test]
    fn every_ring_comes_back_round() {
        let mut app = app_with_test(crate::config::Mode::Time);
        for field in [Field::Mode, Field::Length, Field::Difficulty] {
            let before = app.bar_value(field);
            let rounds = match field {
                Field::Mode => bar::MODES.len(),
                Field::Length => bar::TIMES.len(),
                _ => bar::DIFFICULTIES.len(),
            };
            for _ in 0..rounds {
                app.change_bar_field(field, 1);
            }
            assert_eq!(app.bar_value(field), before, "{field:?} did not come round");
        }
    }

    #[test]
    fn a_toggle_flips_rather_than_cycling() {
        let mut app = app_with_test(crate::config::Mode::Time);
        for field in [Field::Punctuation, Field::Numbers, Field::Blind] {
            let before = app.bar_value(field);
            app.change_bar_field(field, 1);
            assert_ne!(app.bar_value(field), before, "{field:?} did not flip");
            app.change_bar_field(field, 1);
            assert_eq!(app.bar_value(field), before, "{field:?} did not flip back");
        }
    }

    /// Zen and quote have no length, so a keypress that would change one has to
    /// do nothing visible rather than spin a value that is not on the bar.
    #[test]
    fn a_mode_with_no_length_ignores_a_length_change() {
        for mode in [crate::config::Mode::Zen, crate::config::Mode::Quote] {
            let mut app = app_with_test(mode);
            let before = app.config.test.time;
            app.change_bar_field(Field::Length, 1);
            assert_eq!(app.config.test.time, before, "{mode:?}");
        }
    }

    /// The custom text is typed on the settings screen, not cycled here.
    #[test]
    fn the_custom_text_is_not_cycled_by_the_bar() {
        let mut app = app_with_test(crate::config::Mode::Custom);
        app.config.test.custom_text = vec!["a passage".to_owned()];
        let before = app.bar_value(Field::CustomText);
        app.change_bar_field(Field::CustomText, 1);
        assert_eq!(app.bar_value(Field::CustomText), before);
        assert_eq!(app.bar_value(Field::CustomText), "2 words");
    }

    /// Every mode the bar offers must actually build a test. A mode that produced
    /// an empty word list would leave the typing screen blank.
    #[test]
    fn every_mode_builds_a_usable_test() {
        for mode in bar::MODES {
            let app = app_with_test(mode);
            let words = app.test().words();
            assert!(!words.is_empty(), "{mode:?} produced no words");
            assert!(
                app.test().mode().is_scored() == (mode != crate::config::Mode::Zen),
                "{mode:?} disagrees about being scored"
            );
        }
    }

    /// A custom test types the user's own text, not a generated word list.
    #[test]
    fn a_custom_test_types_the_passage_it_was_given() {
        let mut app = app_with_test(crate::config::Mode::Custom);
        app.config.test.custom_text = vec!["one two three".to_owned()];
        app.regenerate();
        let typed: Vec<&str> = app.test().words().iter().map(|w| w.text()).collect();
        assert_eq!(typed, ["one", "two", "three"]);
    }

    /// Punctuation and numbers are generator settings, so they must not be
    /// applied to a passage that already has its own.
    #[test]
    fn a_custom_test_ignores_the_generator_settings() {
        let mut app = app_with_test(crate::config::Mode::Custom);
        app.config.test.punctuation = true;
        app.config.test.numbers = true;
        app.config.test.custom_text = vec!["Hello, world. 42!".to_owned()];
        app.regenerate();
        let typed: Vec<&str> = app.test().words().iter().map(|w| w.text()).collect();
        assert_eq!(typed, ["Hello,", "world.", "42!"]);
    }

    /// An empty passage would finish instantly and report nothing.
    #[test]
    fn a_custom_test_with_no_text_says_so_instead_of_being_empty() {
        let mut app = app_with_test(crate::config::Mode::Custom);
        app.regenerate();
        assert_eq!(app.test().words().len(), 1);
        assert!(app.test().words()[0].text().contains("empty"));
    }

    /// With no quote downloaded yet there is still something on screen, and it
    /// says what is wrong rather than being blank.
    #[test]
    fn a_quote_test_without_quotes_says_so() {
        let app = app_with_test(crate::config::Mode::Quote);
        assert_eq!(app.test().words().len(), 1);
        assert!(app.test().words()[0].text().contains("download"));
    }

    /// Zen is endless: committing a word must produce another one rather than
    /// ending the test.
    #[test]
    fn a_zen_test_never_runs_out_of_words() {
        let mut app = app_with_test(crate::config::Mode::Zen);
        for _ in 0..50 {
            app.set_elapsed(Duration::from_millis(100));
            for c in "hello ".chars() {
                app.type_char(c);
            }
        }
        assert!(!app.test().is_finished(), "zen ended");
        assert!(app.test().words().len() > 50, "zen ran out of words");
    }

    /// Zen has no target, so the countdown is a count of words rather than a
    /// count of seconds — there is no total to count down to.
    #[test]
    fn a_zen_test_shows_a_word_count_rather_than_a_clock() {
        let mut app = app_with_test(crate::config::Mode::Zen);
        assert_eq!(app.countdown(), "0w");
        for c in "one two ".chars() {
            app.type_char(c);
        }
        assert_eq!(app.countdown(), "2w");
    }

    #[test]
    fn changing_a_setting_rebuilds_the_test_so_the_choice_can_be_seen() {
        let mut app = app_with_test(crate::config::Mode::Words);
        let before: Vec<String> = app
            .test()
            .words()
            .iter()
            .map(|w| w.text().to_owned())
            .collect();
        app.change_bar_field(Field::Length, 1); // 25 -> 50
        let after: Vec<String> = app
            .test()
            .words()
            .iter()
            .map(|w| w.text().to_owned())
            .collect();
        assert_eq!(app.test().words().len(), 50, "the new length took effect");
        assert_ne!(before, after, "and it is a different set of words");
    }

    #[test]
    fn a_setting_change_marks_the_config_dirty() {
        let mut app = app_with_test(crate::config::Mode::Time);
        assert!(!app.is_dirty());
        app.change_bar_field(Field::Punctuation, 1);
        assert!(app.is_dirty(), "the change would not be saved");
    }

    /// The bar and the settings screen both change the config, so a change made
    /// in one has to be visible in the other.
    #[test]
    fn the_bar_and_the_settings_screen_agree() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.move_bar_selection(2); // punctuation
        assert_eq!(app.bar_field(), Field::Punctuation);
        let before = app.config.test.punctuation;
        app.change_bar_field(Field::Punctuation, 1);
        assert_ne!(app.config.test.punctuation, before);
    }

    // ---- submission -----------------------------------------------------

    /// A test that has been run, so there is a result to prepare.
    fn run_one() -> App {
        let mut config = Config::default();
        config.test.time = 10;
        let mut app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
        app.set_words("the quick brown".split(' ').map(str::to_owned).collect());
        app.set_elapsed(Duration::from_millis(1500));
        for c in "the quick ".chars() {
            app.type_char(c);
        }
        app
    }

    /// A finished test must produce a payload with a hash, because the hash is
    /// what the server checks first and a missing one is an unexplained 461.
    #[test]
    fn a_finished_test_produces_a_hashed_payload() {
        let app = run_one();
        let body = app.submission_body().expect("a payload");
        assert!(!body.result.hash.is_empty(), "the payload has no hash");
        assert_eq!(body.result.mode, "time");
        assert_eq!(
            body.result.mode2, "10",
            "the length is a string, as the site sends it"
        );
        assert_eq!(body.result.language, "english");
        assert_eq!(body.result.difficulty, "normal");
        assert!(body.result.wpm > 0.0);
    }

    /// There is no writable endpoint on the public API for an ApeKey, and the
    /// app must say so rather than report a save that did not happen.
    #[test]
    fn submitting_reports_that_there_was_nowhere_to_send_it() {
        let app = run_one();
        let outcome = app.submit();
        assert!(!outcome.was_saved());
        let message = outcome.message();
        assert!(message.contains("not submitted"), "{message}");
        assert!(message.contains("ApeKey"), "{message}");
    }

    /// An unstarted test has nothing to submit, and must not invent a payload.
    #[test]
    fn an_unrun_test_has_no_payload() {
        let app = app();
        assert!(app.submission_body().is_none());
    }

    /// The timestamp is wall time rounded to the second, because the server
    /// buckets results by day using it.
    #[test]
    fn the_timestamp_is_the_second_the_test_started() {
        let app = run_one();
        let body = app.submission_body().expect("a payload");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_millis() as i64;
        assert!(body.result.timestamp > 0, "the timestamp was not set");
        assert!(
            body.result.timestamp <= now_ms,
            "{} is in the future",
            body.result.timestamp
        );
        assert_eq!(
            body.result.timestamp % 1000,
            0,
            "the server rounds this to the second"
        );
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
