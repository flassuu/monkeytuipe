//! Application state and event loop.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::Action;
use crate::api;
use crate::config::bar::{self, on_off, step, Field, LengthUnit};
use crate::config::theme::Theme;
use crate::config::Config;
use crate::engine::{Mode, Test};
use crate::screens::input::{self};
use crate::screens::modes::{Key as PressedKey, Mode as Focus, Surface};
use crate::screens::topbar::{Bar, BarState};
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

/// A finished account request, delivered from the background.
///
/// The profile and the records are two requests and the profile is only worth
/// showing when the records arrived, so they arrive together.
#[derive(Debug, Clone)]
pub struct Account {
    pub profile: Result<api::Profile, String>,
    pub bests: Result<api::PersonalBests, String>,
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
    /// A note for the status line, shown once and then cleared.
    message: Option<String>,

    /// The one input window, when it is open.
    ///
    /// On the app rather than on a screen because any screen can want it — the
    /// bar's wrench, the command list on escape, the settings screen's text rows —
    /// and a window that belongs to whichever screen happened to open it has to
    /// be closed by that screen too.
    input: Option<input::Window>,

    /// Which mode the keyboard is in. See [`crate::screens::modes`].
    ///
    /// Not derivable from whether the window is open, and the difference matters:
    /// the window being open is what makes the mode *input*, but a screen can
    /// collect text with no window on screen, and then letters still have to be
    /// text.
    mode: Focus,

    /// The ApeKey client, built once the key is known.
    ///
    /// Built lazily rather than in `new` because the key can be edited while the
    /// app is running, and a client holding the old key would keep fetching the
    /// previous account's records.
    api: Option<api::ApiClient>,
    /// The signed-in user's profile, once fetched.
    profile: Option<api::Profile>,
    /// The personal bests, per mode, for the language in use.
    bests: api::PersonalBests,
    /// What went wrong with the account, for the settings screen.
    account_note: Option<String>,
    /// Where a finished account fetch is delivered.
    account_inbox: Receiver<Account>,
    account_sender: Sender<Account>,

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
        let (account_tx, account_inbox) = channel::<Account>();
        // The bar is built before the struct exists, because the struct literal
        // moves the config in and there is nothing left to read it from.
        let bar = crate::screens::topbar::Bar::build(BarState::from(&config));
        let mut app = Self {
            inbox_sender: tx,
            quote_sender: quote_tx,
            quote_inbox,
            account_sender: account_tx,
            account_inbox,
            language: language::embedded(FALLBACK_LANGUAGE).expect("english is embedded"),
            requested: FALLBACK_LANGUAGE.to_owned(),
            pending: None,
            terminal: crate::config::Terminal::from_env(),
            bar,
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
            message: None,
            input: None,
            mode: Focus::Navigation,
            api: None,
            profile: None,
            bests: Default::default(),
            account_note: None,
        };
        app.start_language(app.config.test.language.clone());
        app.start_quotes();
        app.refresh_account();
        app
    }

    // ---- word lists ----------------------------------------------------

    // ---- the account ---------------------------------------------------

    /// True when an ApeKey is configured, i.e. the account endpoints are usable.
    pub fn is_signed_in(&self) -> bool {
        self.api
            .as_ref()
            .is_some_and(api::ApiClient::is_authenticated)
    }

    /// The signed-in user's profile, once it has been fetched.
    pub fn profile(&self) -> Option<&api::Profile> {
        self.profile.as_ref()
    }

    /// The personal bests fetched for the mode and language in use.
    pub fn bests(&self) -> &api::PersonalBests {
        &self.bests
    }

    /// The record the current test is being measured against, if there is one.
    ///
    /// Matched on mode, language *and* length: showing a 60-second record next to
    /// a 15-second test would be a number that is true and useless.
    pub fn personal_best_for(&self) -> Option<&api::PersonalBest> {
        self.bests.best(
            self.config.test.mode,
            variants::base_of(&self.config.test.language),
            self.config
                .test
                .mode
                .length_unit()
                .map(|_| match self.config.test.mode {
                    crate::config::Mode::Words => self.config.test.words,
                    _ => self.config.test.time,
                }),
        )
    }

    /// Why the account could not be read, for the settings screen.
    pub fn account_note(&self) -> Option<&str> {
        self.account_note.as_deref()
    }

    /// Builds the client for the configured key, dropping anything fetched for a
    /// previous one.
    fn rebuild_client(&mut self) {
        self.clear_account();
        let Some(key) = self.config.resolved_ape_key() else {
            self.api = None;
            return;
        };
        self.api = Some(api::ApiClient::new(self.config.api_url.clone(), key));
    }

    /// Forgets the account, because the key behind it changed or went away.
    fn clear_account(&mut self) {
        self.profile = None;
        self.bests = Default::default();
        self.account_note = None;
    }

    /// Starts a fetch of the profile and the records.
    ///
    /// Both go out at once and neither blocks the first frame: a user with no
    /// network should get a typing test, not a spinner.
    pub fn refresh_account(&mut self) {
        self.rebuild_client();
        let Some(client) = self.api.clone() else {
            self.account_note = Some("no ApeKey set — reads need one".to_owned());
            return;
        };
        let mode = self.config.test.mode;
        let language = variants::base_of(&self.config.test.language).to_owned();
        let tx = self.account_sender.clone();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No runtime: a headless caller has no network, and saying so is
            // better than a spinner that never resolves.
            self.account_note = Some("no network runtime available".to_owned());
            return;
        };
        handle.spawn(async move {
            let profile = client.profile().await.map_err(|e| e.to_string());
            let bests = match &profile {
                Ok(_) => match client.personal_bests(mode, &language).await {
                    Ok(records) => {
                        let mut out = api::PersonalBests::default();
                        match mode {
                            crate::config::Mode::Time => out.time = records,
                            crate::config::Mode::Words => out.words = records,
                            _ => out.quote = records,
                        }
                        Ok(out)
                    }
                    Err(e) => Err(e.to_string()),
                },
                // No profile, so no point asking for records: the key is the
                // problem and the second request would fail the same way.
                Err(e) => Err(e.clone()),
            };
            let _ = tx.send(Account { profile, bests });
        });
    }

    /// Picks up a finished account request.
    fn drain_account(&mut self) {
        while let Ok(account) = self.account_inbox.try_recv() {
            self.account_note = match (account.profile, account.bests) {
                (Ok(profile), Ok(bests)) => {
                    self.profile = Some(profile);
                    self.bests = bests;
                    None
                }
                (Err(error), _) => Some(error),
                (_, Err(error)) => Some(error),
            };
        }
    }

    /// The text the settings editor for a row should start from.
    fn row_text(&self, row: Row) -> String {
        match row {
            Row::CustomText => self.config.test.custom_text.join("\n"),
            Row::ApeKey => self.config.ape_key.clone(),
            _ => String::new(),
        }
    }

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
    fn test_mode_inner(&self) -> Mode {
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

    /// The bar as it should be drawn right now, rebuilt from the config.
    ///
    /// Rebuilt on demand rather than kept in step: the bar is a pure function of
    /// the settings, and a cached copy of a pure function is one more thing to go
    /// stale — which would show one mode in the bar and run another.
    pub fn bar(&self) -> Bar {
        let mut bar = Bar::build(BarState::from(&self.config));
        bar.selected = self.bar.selected;
        bar.interactive = self.bar.interactive;
        bar
    }

    /// The field the bar has selected, if the selection is on a live button.
    pub fn bar_selection(&self) -> Option<Field> {
        self.bar.selected_field()
    }

    /// The field the bar has selected, or `None` when the bar is empty.
    pub fn bar_field(&self) -> Option<Field> {
        self.bar_selection()
    }

    /// Moves the bar's selection, skipping anything the current mode disabled.
    pub fn move_bar_selection(&mut self, by: isize) {
        let mut bar = self.bar();
        bar.move_selection(by);
        self.bar.selected = bar.selected;
    }

    /// Whether the bar should react to keys.
    ///
    /// Off while a test is running, which is what the site does: the whole bar
    /// goes to `opacity-0 pointer-events-none` once the words have focus, because
    /// changing a setting restarts the test and doing that under someone's hands
    /// is worse than making them press escape first.
    pub fn set_bar_interactive(&mut self, interactive: bool) {
        self.bar.interactive = interactive;
    }

    /// Whether the bar is currently reacting to keys.
    pub fn bar_is_interactive(&self) -> bool {
        self.bar.interactive
    }

    /// Opens the one input window, empty, and puts the keyboard in it.
    ///
    /// Opening and switching are one operation on purpose. A window that is open
    /// while the mode still says "navigation" is a state where a letter is a
    /// command and a character at the same time, and every bug that produces
    /// comes from that state existing.
    pub fn open_input(&mut self, reason: input::Reason) {
        self.input = Some(input::Window::new(reason));
        self.mode = Focus::Input;
    }

    /// Opens the one input window with something already in it, which is what
    /// editing an existing value does — the window is opened *on* the value, not
    /// on an empty field the user has to retype.
    pub fn open_input_with(&mut self, reason: input::Reason, text: impl Into<String>) {
        self.input = Some(input::Window::prefilled(reason, text));
        self.mode = Focus::Input;
    }

    /// Opens the window for a length, prefilled with the bare number.
    ///
    /// The bar says `30s` and the results say `30 seconds`, but a text field the
    /// user is about to edit should hold `30`: the unit is something the field
    /// *produces*, and prefilling it means the first keystroke lands in the middle
    /// of `30s` and produces nonsense.
    fn open_length(&mut self, field: Field) {
        let value = match field {
            Field::Time | Field::TimeCustom => self.config.test.time,
            _ => self.config.test.words,
        };
        self.open_input_with(input::Reason::Length(field), value.to_string());
    }

    /// Stores a custom passage, splitting it into lines.
    pub fn set_custom_text(&mut self, text: String) {
        self.apply(crate::screens::Effect::SetCustomText(text));
    }

    /// Runs a command by index into the command list.
    ///
    /// Returns `true` when the app should quit, so a `Quit` command does not
    /// have to be special-cased by whoever ran it.
    pub fn run_command(&mut self, index: usize) -> bool {
        use crate::screens::commands::Action as Cmd;
        let Some(action) = crate::screens::commands::action(index) else {
            return false;
        };
        match action {
            Cmd::Restart => {
                self.regenerate();
            }
            Cmd::Mode(mode) => {
                self.config.test.mode = mode;
                self.dirty = true;
                if mode == crate::config::Mode::Quote {
                    self.start_quotes();
                } else {
                    self.regenerate();
                }
            }
            Cmd::Toggle(field) => {
                self.change_bar_field(field, 1);
            }
            Cmd::SetLength(field) => {
                // The field opens on the value it already has, so a length is
                // edited rather than retyped.
                self.open_length(field);
            }
            Cmd::Blind => {
                self.config.test.blind = !self.config.test.blind;
                self.dirty = true;
            }
            Cmd::Difficulty(by) => {
                self.adjust_by(Row::Difficulty, by);
            }
            Cmd::Languages => {
                self.show_screen(ScreenKind::Settings);
            }
            Cmd::Settings => {
                self.show_screen(ScreenKind::Settings);
            }
            Cmd::CustomText => {
                self.open_input_with(input::Reason::Text, self.config.test.custom_text.join("\n"));
            }
            Cmd::NextTheme => {
                self.config.theme = self.config.theme.next();
                self.dirty = true;
            }
            Cmd::Theme(name) => {
                if let Some(theme) = crate::config::theme::ThemeName::ALL
                    .iter()
                    .copied()
                    .find(|t| t.label() == name.replace('_', " "))
                {
                    self.config.theme = theme;
                    self.dirty = true;
                }
            }
            Cmd::CopyResult => {
                self.message =
                    Some("no clipboard in a terminal — use the bar's copy instead".to_owned());
            }
            Cmd::Quit => return true,
        }
        false
    }

    /// Closes the input window, returning what came out of it.
    ///
    /// An `Invalid` outcome leaves the window **open**: closing it would throw
    /// away what the user typed, which is the one thing they would not want to
    /// retype.
    pub fn close_input(&mut self, outcome: input::Outcome) {
        if matches!(outcome, input::Outcome::Invalid(_)) {
            // The window stays open, so the mode stays on input: a window the user
            // is still typing into must keep the keyboard.
            return;
        }
        self.input = None;
        self.mode = Focus::Navigation;
        match outcome {
            input::Outcome::Cancelled => {}
            input::Outcome::Length(field, value) => {
                self.set_length(field, value);
            }
            input::Outcome::Text(text) => {
                self.set_custom_text(text);
            }
            input::Outcome::ApeKey(key) => {
                self.apply(crate::screens::Effect::SetApeKey(key));
            }
            input::Outcome::Command(index) => {
                self.run_command(index);
            }
            input::Outcome::Invalid(_) => unreachable!("handled above"),
        }
    }

    /// Which mode the keyboard is in.
    pub fn mode(&self) -> Focus {
        self.mode
    }

    /// Switches mode.
    ///
    /// Going to input mode without a window is the app's problem to fix rather
    /// than the caller's: a mode that says "keys are text" with no field to put
    /// them in is a mode that eats letters, which is the one thing input mode must
    /// never do.
    pub fn set_mode(&mut self, mode: Focus) {
        self.mode = mode;
        if mode == Focus::Input && self.input.is_none() {
            self.input = Some(input::Window::new(input::Reason::Command));
        }
        if mode == Focus::Navigation {
            self.input = None;
        }
    }

    /// The input window, if it is open.
    pub fn input_window(&self) -> Option<&input::Window> {
        self.input.as_ref()
    }

    /// Why the input window is open, if it is.
    pub fn input_reason(&self) -> Option<&input::Reason> {
        self.input.as_ref().map(input::Window::reason)
    }

    /// Whether the input window is open.
    pub fn input_is_open(&self) -> bool {
        self.input.is_some()
    }

    /// What a field currently says, for the settings screen and the status line.
    pub fn bar_value(&self, field: Field) -> String {
        let test = &self.config.test;
        match field {
            Field::Punctuation => on_off(test.punctuation),
            Field::Numbers => on_off(test.numbers),
            Field::Mode => test.mode.bar_label().to_owned(),
            Field::Time | Field::TimeCustom => LengthUnit::Seconds.render(test.time),
            Field::Words | Field::WordsCustom => LengthUnit::Words.render(test.words),
            Field::QuoteLength => test.quote_length.as_str().to_owned(),
            Field::CustomText => match test.custom_text.first() {
                Some(text) => format!(
                    "{} words",
                    text.split(' ').filter(|w| !w.is_empty()).count()
                ),
                None => "not set".to_owned(),
            },
        }
    }

    /// Changes a field by `by` steps, and rebuilds the test so the change can be
    /// seen before it is typed.
    ///
    /// Returns the field it acted on, or `None` when it did nothing. A field that
    /// needs the input window returns `None` here and the app opens the window
    /// instead — cycling "custom" would be cycling nothing.
    pub fn change_bar_field(&mut self, field: Field, by: i8) -> Option<Field> {
        if field.needs_input() {
            return None;
        }
        let by = isize::from(by);
        let test = &mut self.config.test;
        let mut needs_words = true;
        let mut needs_quotes = false;

        match field {
            Field::Mode => {
                let current = bar::MODES.iter().position(|m| *m == test.mode);
                test.mode = bar::MODES[step(current, by, bar::MODES.len())];
                needs_quotes = test.mode == crate::config::Mode::Quote;
                // Zen makes its own words, so there is nothing to build.
                if test.mode == crate::config::Mode::Zen {
                    needs_words = false;
                }
                // The site forces both false when switching into quote, zen or
                // custom: a passage is already punctuated and is not made of the
                // generator's words.
                if matches!(
                    test.mode,
                    crate::config::Mode::Quote
                        | crate::config::Mode::Zen
                        | crate::config::Mode::Custom
                ) {
                    test.punctuation = false;
                    test.numbers = false;
                }
            }
            Field::Time => {
                let current = bar::TIMES.iter().position(|t| *t == test.time);
                test.time = bar::TIMES[step(current, by, bar::TIMES.len())];
            }
            Field::Words => {
                let current = bar::WORD_COUNTS.iter().position(|w| *w == test.words);
                test.words = bar::WORD_COUNTS[step(current, by, bar::WORD_COUNTS.len())];
            }
            Field::QuoteLength => {
                let current = bar::QUOTE_LENGTHS
                    .iter()
                    .position(|q| *q == test.quote_length);
                test.quote_length = bar::QUOTE_LENGTHS[step(current, by, bar::QUOTE_LENGTHS.len())];
            }
            Field::Punctuation => test.punctuation = !test.punctuation,
            Field::Numbers => test.numbers = !test.numbers,
            Field::TimeCustom | Field::WordsCustom | Field::CustomText => return None,
        }

        self.dirty = true;
        if needs_quotes {
            self.start_quotes();
            return Some(field);
        }
        if needs_words {
            self.regenerate();
        }
        Some(field)
    }

    /// Sets a length from the input window.
    ///
    /// A preset length is a choice; anything else is a custom one, and the wrench
    /// stays lit so the bar says the value is not one of the presets — which is
    /// exactly how the site shows it.
    pub fn set_length(&mut self, field: Field, value: u32) -> bool {
        match field {
            Field::Time | Field::TimeCustom if value > 0 => self.config.test.time = value,
            Field::Words | Field::WordsCustom if value > 0 => self.config.test.words = value,
            _ => return false,
        }
        self.dirty = true;
        self.regenerate();
        true
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

    /// Whether the word pane hides the words that are not being typed.
    ///
    /// The website's "blind mode": only the current word is shown, so the test is
    /// on reading ahead rather than on recall. It is a setting with no engine
    /// effect — the words are all there, they are just not drawn — so it belongs
    /// to the screen rather than to the test.
    pub fn is_blind(&self) -> bool {
        self.config.test.blind
    }

    /// The mode the current test is running in.
    ///
    /// The mode the *test* is running in, which is not the same thing as
    /// [`Self::mode`]: that one is about the keyboard.
    ///
    /// Public because a screen has to render differently for zen — there is no
    /// target there, so the words pane draws what was typed rather than what was
    /// to be typed — and two things called "mode" is the price of an app with a
    /// keyboard mode and a test mode in it. The names say which is which.
    pub fn test_mode(&self) -> Mode {
        self.test_mode_inner()
    }

    /// Sets the theme, for a test or a command.
    ///
    /// Not a setter on the config, because the settings screen has its own arrow
    /// keys for it and a second route to the same value is a second thing to keep
    /// in step.
    pub fn set_theme(&mut self, theme: crate::config::theme::ThemeName) {
        self.config.theme = theme;
        self.dirty = true;
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
    /// What the settings screen is currently showing, for the tests and for
    /// anything that has to know whether an editor has the keyboard.
    pub fn view_kind(&self) -> Option<&crate::screens::settings::View> {
        self.screen.settings_view()
    }

    pub fn selected_row(&self) -> Option<Row> {
        self.screen.selected_row()
    }

    /// Draws the current screen.
    ///
    /// Public so the app can be rendered headlessly, e.g. against
    /// `ratatui::backend::TestBackend` in tests.
    pub fn render(&self, frame: &mut ratatui::Frame) {
        self.screen.render(self, frame);
        // The window is drawn last and over everything: it belongs to the app
        // rather than to a screen, so any screen can open it and it looks the
        // same wherever it was opened from.
        if let Some(window) = &self.input {
            input::render(window, frame.area(), self.theme(), frame);
        }
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
        self.drain_account();
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
            Effect::Adjust(row, by) => {
                self.adjust_by(row, by);
                false
            }
            Effect::SetLanguage(id) => {
                self.config.test.language = id.clone();
                self.start_language(id);
                false
            }
            Effect::SetApeKey(key) => {
                self.config.ape_key = key;
                self.dirty = true;
                // A new key means a different account, so whatever was fetched
                // for the old one is no longer about this user.
                self.clear_account();
                false
            }
            Effect::SetCustomText(text) => {
                // A passage is a list of lines and the first is the test, so
                // saving is splitting on newlines and dropping the blanks.
                self.config.test.custom_text = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect();
                self.dirty = true;
                if self.config.test.mode == crate::config::Mode::Custom {
                    self.regenerate();
                }
                false
            }
            Effect::OpenEditor(row) => {
                let text = self.row_text(row);
                self.screen.open_editor(row, text);
                false
            }
            Effect::ShowMessage(message) => {
                self.message = Some(message);
                false
            }
            Effect::MoveBar(by) => {
                self.move_bar_selection(isize::from(by));
                false
            }
            Effect::PressBar => {
                if let Some(field) = self.bar().activate() {
                    if self.change_bar_field(field, 1).is_none() && field.needs_input() {
                        self.open_length(field);
                    }
                }
                false
            }
            Effect::OpenCommands => {
                self.open_input(input::Reason::Command);
                false
            }
            Effect::FinishTest => {
                // The only way out of a zen test: it has no length, so there is
                // nothing to run out of.
                if self.test.mode() == Mode::Zen {
                    self.test.finish();
                    self.settle();
                }
                false
            }
            Effect::ChangeBar(by) => {
                let Some(field) = self.bar_field() else {
                    return false;
                };
                if self.change_bar_field(field, by).is_none() && field.needs_input() {
                    // The wrench opens the one input window, which is also where
                    // the commands live.
                    self.open_length(field);
                }
                false
            }
            Effect::Toggle(row) => {
                self.adjust_by(row, 1);
                false
            }
        }
    }

    /// Applies a settings row, in the given direction.
    ///
    /// Every variant is listed, including the ones that do nothing, so a new row
    /// cannot silently become a no-op: adding it here and forgetting it is a
    /// compile error rather than a key that quietly stops working.
    ///
    /// A toggle ignores the direction — there is no such thing as turning a toggle
    /// off *further* — and a cycle obeys it, which is why the direction is a
    /// parameter rather than being fixed at `1` here.
    fn adjust_by(&mut self, row: Row, by: i8) {
        match row {
            Row::Theme => self.config.theme = self.config.theme.step(isize::from(by)),
            Row::Language => {
                let id = next_language(&self.config.test.language);
                self.config.test.language = id.clone();
                self.start_language(id);
            }
            Row::Punctuation => self.config.test.punctuation = !self.config.test.punctuation,
            Row::Numbers => self.config.test.numbers = !self.config.test.numbers,
            // The switch is still honoured so an existing config is not ignored —
            // but it cannot make submission work, and the screen says so.
            Row::SubmitResults => self.config.submit_results = !self.config.submit_results,
            // Difficulty is not on the website's test bar — it is in the settings
            // panel, which is what this row is — but the command list steps it
            // directly, because a setting that can only be reached by opening a
            // screen and walking to a row is a setting most people never change.
            Row::Difficulty => {
                let current = bar::DIFFICULTIES
                    .iter()
                    .position(|d| *d == self.config.test.difficulty);
                self.config.test.difficulty =
                    bar::DIFFICULTIES[bar::step(current, isize::from(by), bar::DIFFICULTIES.len())];
            }
            // Free text and the bar's own fields. A row that has a view is
            // opened by the screen, not stepped here, and the bar's fields are
            // changed through the bar.
            Row::CustomText
            | Row::ApeKey
            | Row::Mode
            | Row::QuoteLength
            | Row::Blind
            | Row::Back => return,
        }
        self.dirty = true;
        // Punctuation, numbers and the language all change the words, so the test
        // is rebuilt to show the effect before it is typed.
        if matches!(
            row,
            Row::Punctuation | Row::Numbers | Row::Theme | Row::Language | Row::Difficulty
        ) {
            self.regenerate();
        }
        // A new key means a different account, so the records for the old one
        // have to go.
        if row == Row::Theme {
            // The theme is not a word-list setting, so nothing else to do; this
            // branch exists to say so rather than leave it implied.
        }
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
        use crate::screens::modes::{self as modes, Direction, Intent};

        let key = PressedKey::from_event(key);
        if !key.press {
            // Terminals that report releases would otherwise fire every binding
            // twice and type every letter twice.
            return Ok(false);
        }

        // The mode decides what a key means, and it is asked before anything
        // else: the two modes disagree about whether `q` is a word or a command,
        // and nothing further down can be right if this is wrong.
        let surface = self.surface();
        let focus = self.mode();
        match modes::intent(key, focus, surface, self.input.is_some()) {
            Intent::OpenInput => {
                // Opening and switching are one thing, so they cannot disagree: a
                // window that is open with the mode still on navigation is a state
                // where `q` is a command and a letter at the same time.
                self.open_input(input::Reason::Command);
                self.set_mode(Focus::Input);
                return Ok(false);
            }
            Intent::CloseInput => {
                // Closing without acting. A palette that is dismissed must not run
                // whatever it happened to have highlighted.
                self.input = None;
                self.set_mode(Focus::Navigation);
                return Ok(false);
            }
            Intent::Switch => {
                self.set_mode(focus.other());
                return Ok(false);
            }
            _ => {}
        }

        // In input mode the window has the keyboard, whole. It is the field's
        // mode: every bare key is the field's, including the ones the rules above
        // did not claim, so that a key the window does not use is still a key the
        // window owns rather than a key that types an `f` into the test.
        //
        // A *modified* key is the exception, and it is the important one: `ctrl+c`
        // has to quit with the window open, or there is a modal with no way out.
        if self.input.is_some() && focus == Focus::Input && !key.ctrl && !key.alt {
            let Some(outcome) = self.input.as_mut().and_then(|w| w.key(key.code)) else {
                return Ok(false);
            };
            self.close_input(outcome);
            return Ok(false);
        }

        match modes::intent(key, focus, surface, self.input.is_some()) {
            Intent::Skip => return self.on_action(Action::Skip),
            Intent::Move(direction) => {
                let action = match direction {
                    Direction::Up => Action::Up,
                    Direction::Down => Action::Down,
                    Direction::Left => Action::Left,
                    Direction::Right => Action::Right,
                };
                return self.on_action(action);
            }
            Intent::Text(c) => return self.on_action(Action::Char(c)),
            Intent::Command(letter) => {
                // A letter that is a command. `q` quits, the way it does in vim and
                // in every other program that has letters.
                if let Some(action) = self.action_for_letter(letter) {
                    return self.on_action(action);
                }
                // A letter that is not a command does nothing. It is not turned
                // into something nobody asked for, and it is not passed on as text
                // either: on a browsing screen there is nothing to type into.
            }
            Intent::Nothing | Intent::OpenInput | Intent::CloseInput | Intent::Switch => {}
        }

        // A key nothing claimed: the configured bindings, then the screen.
        self.on_keybind(key)
    }

    /// Which surface the keyboard rules apply to.
    ///
    /// A screen that wants text gets text, whatever it is. That is the same rule
    /// the typing screen uses, and the reason is the same: a key that means
    /// something and is also a letter has to be a letter, or a word cannot be
    /// typed.
    fn surface(&self) -> Surface {
        if self.screen.wants_text() {
            // The typing screen is the one surface where a letter is *always*
            // text, including the navigation-mode letters. An editor is a field
            // but it is not a test, and the difference is that the vim keys are
            // useful on the row list around it.
            if matches!(self.screen, crate::screens::ScreenState::Typing(_)) {
                Surface::Typing
            } else {
                Surface::Editing
            }
        } else {
            Surface::Browsing
        }
    }

    /// What a letter means in navigation mode, on a screen where letters are
    /// commands.
    ///
    /// Only the ones that are the same everywhere. Anything screen-specific stays
    /// with the screen: a settings screen is the only thing that can know that
    /// `q` means "back" there, and the app is the only thing that can know that
    /// `q` means "quit" everywhere.
    fn action_for_letter(&self, letter: char) -> Option<Action> {
        match letter {
            'q' => Some(Action::Quit),
            'j' | 'k' | 'h' | 'l' => None,
            '?' => Some(Action::Settings),
            _ => None,
        }
    }

    /// Runs the configured bindings and then the screen, for a key the mode rules
    /// did not claim.
    fn on_keybind(&mut self, key: PressedKey) -> anyhow::Result<bool> {
        let event = KeyEvent {
            code: key.code,
            modifiers: key.modifiers(),
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let Some(action) = self.resolve(event) else {
            return Ok(false);
        };
        self.on_action(action)
    }

    /// Hands an action to the screen and applies what comes back.
    fn on_action(&mut self, action: Action) -> anyhow::Result<bool> {
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
        // A printable key is text whenever the screen has a field to put it in.
        // `h` is bound to `left`, so without this a word containing an `h` cannot
        // be typed in the settings editor at all — the same rule the typing screen
        // follows, for the same reason.
        if self.screen.wants_text() {
            if let Some(c) = printable {
                return Some(Action::Char(c));
            }
        }

        // Escape opens the command list, which is what the site binds it to when
        // `quickRestart` is "off" — the default, and in that state escape has no
        // other job.
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            return Some(Action::Command);
        }
        // Shift+Enter ends a zen test, which is the only way out of one: it has
        // no length and therefore no way to run out.
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
            return Some(Action::Finish);
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

    /// The settings screen's own way out is the `back to typing` row, which is
    /// what the site does — there is no escape key to press there either.
    #[test]
    fn the_back_row_leaves_the_settings_screen() {
        let mut app = app();
        app.on_key(press(KeyCode::F(2))).expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Settings);
        // The row list wraps, so one short of a full lap lands on the last row.
        for _ in 0..settings_rows().saturating_sub(1) {
            app.on_key(press(KeyCode::Down)).expect("no io");
        }
        assert_eq!(app.selected_row(), Some(Row::Back));
        app.on_key(press(KeyCode::Enter)).expect("no io");
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
            // `auto` is the default and the first step is the next thing after it,
            // which is the terminal's own colours.
            crate::config::theme::ThemeName::Terminal,
            "the first step off the default is the next theme in the list"
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

    /// Escape is the command list on the site, not a back key — with
    /// `quickRestart` off it has no other job — so a new test is a command.
    #[test]
    fn a_new_test_from_the_result_is_a_command_not_a_key() {
        let mut app = finished(12);
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(app.input_reason(), Some(&input::Reason::Command));

        let index = crate::screens::commands::filter("next test")
            .first()
            .map(|m| m.command)
            .expect("a next-test command");
        app.run_command(index);
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

    /// How many rows the settings screen has, so a test can walk to the last one
    /// without hard-coding a number that changes when a row is added.
    fn settings_rows() -> usize {
        crate::screens::settings::ROWS.len()
    }

    fn app_with_test(mode: crate::config::Mode) -> App {
        let mut config = Config::default();
        config.test.mode = mode;
        App::new(config, PathBuf::from("/nonexistent/config.toml"))
    }

    /// The bar as the screen draws it, for a mode.
    fn bar_in(mode: crate::config::Mode) -> Bar {
        let mut config = Config::default();
        config.test.mode = mode;
        App::new(config, PathBuf::from("/nonexistent/config.toml")).bar()
    }

    #[test]
    fn the_bar_shows_the_mode_first_in_every_mode() {
        for mode in bar::MODES {
            let bar = bar_in(mode);
            assert_eq!(bar.selected_field(), Some(Field::Mode), "{mode:?}");
        }
    }

    /// Changing mode can empty a card under the selection, so the index has to be
    /// re-read rather than trusted.
    #[test]
    fn the_selection_survives_a_change_of_mode() {
        let mut app = app_with_test(crate::config::Mode::Words);
        for _ in 0..30 {
            app.move_bar_selection(1);
        }
        for mode in bar::MODES {
            app.change_bar_field(Field::Mode, 0);
            let total = app.bar().buttons().len();
            assert!(
                app.bar().selected < total,
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
        assert_eq!(app.bar().selected, 0, "it went off the left edge");
        for _ in 0..40 {
            app.move_bar_selection(1);
        }
        assert_eq!(app.bar().selected + 1, app.bar().buttons().len());
    }

    /// The wrench is a dialog, not a step, so changing it does nothing here.
    #[test]
    fn the_wrench_does_not_cycle() {
        let mut app = app_with_test(crate::config::Mode::Time);
        let before = app.config.test.time;
        assert_eq!(app.change_bar_field(Field::TimeCustom, 1), None);
        assert_eq!(app.config.test.time, before, "the wrench cycled a value");
        assert!(Field::TimeCustom.needs_input());
        assert!(!Field::Time.needs_input());
    }

    #[test]
    fn a_duration_outside_the_presets_lights_the_wrench() {
        let mut app = app_with_test(crate::config::Mode::Time);
        assert!(app.set_length(Field::TimeCustom, 42));
        let bar = app.bar();
        let custom = bar
            .right
            .buttons
            .iter()
            .find(|b| b.label == "custom")
            .expect("a wrench");
        assert!(custom.active, "42 seconds did not light the wrench");
    }

    #[test]
    fn a_value_has_to_be_come_back_round() {
        let mut app = app_with_test(crate::config::Mode::Time);
        for field in [Field::Mode, Field::Time, Field::QuoteLength] {
            let before = app.bar_value(field);
            let rounds = match field {
                Field::Mode => bar::MODES.len(),
                Field::Time => bar::TIMES.len(),
                _ => bar::QUOTE_LENGTHS.len(),
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
        for field in [Field::Punctuation, Field::Numbers] {
            let before = app.bar_value(field);
            app.change_bar_field(field, 1);
            assert_ne!(app.bar_value(field), before, "{field:?} did not flip");
            app.change_bar_field(field, 1);
            assert_eq!(app.bar_value(field), before, "{field:?} did not flip back");
        }
    }

    /// The site forces both false when switching into quote, zen or custom: a
    /// passage is already punctuated and is not made of the generator's words.
    #[test]
    fn switching_to_a_passage_mode_clears_the_toggles() {
        for mode in [
            crate::config::Mode::Quote,
            crate::config::Mode::Zen,
            crate::config::Mode::Custom,
        ] {
            let mut app = app_with_test(crate::config::Mode::Time);
            assert!(app.config.test.punctuation, "the premise changed");
            while app.config.test.mode != mode {
                app.change_bar_field(Field::Mode, 1);
            }
            assert!(!app.config.test.punctuation, "{mode:?} left punctuation on");
            assert!(!app.config.test.numbers, "{mode:?} left numbers on");
        }
    }

    /// Every mode must build a usable test, or a mode in the bar is a dead button.
    #[test]
    fn every_mode_builds_a_usable_test() {
        for mode in bar::MODES {
            let app = app_with_test(mode);
            assert!(!app.test().words().is_empty(), "{mode:?} produced no words");
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
        let mut app = app_with_test(crate::config::Mode::Quote);
        app.quotes = None;
        app.regenerate();
        assert_eq!(app.test().words().len(), 1);
        assert!(app.test().words()[0].text().contains("download"));
    }

    /// And with a quote file the passage is typed, not the placeholder.
    #[test]
    fn a_quote_test_types_the_quote_when_there_is_one() {
        let mut app = app_with_test(crate::config::Mode::Quote);
        app.quotes = Some(crate::words::quotes::QuoteList {
            language: "english".to_owned(),
            groups: Vec::new(),
            quotes: vec![crate::words::quotes::Quote {
                id: 1,
                text: "one two three".to_owned(),
                source: Some("s".to_owned()),
                length: 13,
                british_text: None,
                words: Vec::new(),
            }],
        });
        app.regenerate();
        let typed: Vec<&str> = app.test().words().iter().map(|w| w.text()).collect();
        assert_eq!(typed, ["one", "two", "three"]);
    }

    /// The four buckets, and a language with none in the one asked for.
    #[test]
    fn a_quote_length_that_nothing_falls_in_says_which_ones_do() {
        let mut app = app_with_test(crate::config::Mode::Quote);
        app.quotes = Some(crate::words::quotes::QuoteList {
            language: "klingon".to_owned(),
            groups: Vec::new(),
            quotes: vec![crate::words::quotes::Quote {
                id: 1,
                text: "short".to_owned(),
                source: Some("s".to_owned()),
                length: 5,
                british_text: None,
                words: Vec::new(),
            }],
        });
        app.config.test.quote_length = crate::config::QuoteLength::Thicc;
        app.regenerate();
        let word = &app.test().words()[0].text();
        assert!(word.contains("no thicc quotes"), "{word}");
        // And it names the buckets that do have some, because "nothing at all" and
        // "nothing that long" are different problems.
        assert!(word.contains("short"), "{word}");
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

    /// Shift+Enter is the only way out of a zen test.
    #[test]
    fn shift_enter_finishes_a_zen_test() {
        let mut app = app_with_test(crate::config::Mode::Zen);
        for c in "one two ".chars() {
            app.type_char(c);
        }
        assert!(!app.test().is_finished());
        app.on_key(press_mod(KeyCode::Enter, KeyModifiers::SHIFT))
            .expect("no io");
        assert!(app.test().is_finished(), "shift+enter did not finish zen");
        assert_eq!(app.screen_kind(), ScreenKind::Results);
    }

    /// And it does nothing in a test that ends on its own, where a stray
    /// shift+enter would end a test the user was still in the middle of.
    #[test]
    fn shift_enter_does_nothing_outside_zen() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.set_elapsed(Duration::from_millis(500));
        for c in "hello ".chars() {
            app.type_char(c);
        }
        app.on_key(press_mod(KeyCode::Enter, KeyModifiers::SHIFT))
            .expect("no io");
        assert!(!app.test().is_finished(), "shift+enter ended a timed test");
    }

    #[test]
    fn changing_a_setting_rebuilds_the_test_so_the_choice_can_be_seen() {
        let mut app = app_with_test(crate::config::Mode::Words);
        app.change_bar_field(Field::Words, 1); // 25 -> 50
        assert_eq!(app.test().words().len(), 50, "the new length took effect");
    }

    #[test]
    fn a_setting_change_marks_the_config_dirty() {
        let mut app = app_with_test(crate::config::Mode::Time);
        assert!(!app.is_dirty());
        app.change_bar_field(Field::Punctuation, 1);
        assert!(app.is_dirty(), "the change would not be saved");
    }

    // ---- the input window ------------------------------------------------

    /// Escape opens the command list, which is the site's default binding.
    #[test]
    fn escape_opens_the_command_list() {
        let mut app = app();
        assert!(app.input_window().is_none());
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(
            app.input_reason(),
            Some(&input::Reason::Command),
            "escape did not open the command list"
        );
    }

    /// The command list is reachable from a finished test too, which is where a
    /// re-run usually starts from.
    #[test]
    fn escape_opens_the_command_list_from_the_result_screen() {
        let mut app = finished(12);
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(app.input_reason(), Some(&input::Reason::Command));
    }

    /// A duration reads the way the site reads one, so `1h30m` works.
    #[test]
    fn the_window_reads_a_duration_the_way_the_site_does() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.open_input_with(input::Reason::Length(Field::TimeCustom), "30");
        app.close_input(input::Outcome::Length(Field::TimeCustom, 5400));
        assert_eq!(app.config.test.time, 5400);
        assert!(app.input_window().is_none(), "the window stayed open");
    }

    /// A value that cannot be read leaves the window open with the text still in
    /// it, rather than closing and losing what was typed.
    #[test]
    fn a_value_that_cannot_be_read_keeps_the_window_open() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.open_input_with(input::Reason::Length(Field::TimeCustom), "30");
        app.close_input(input::Outcome::Invalid("abc".to_owned()));
        assert!(app.input_window().is_some());
        assert_eq!(app.config.test.time, 30, "the value changed anyway");
    }

    /// Running a command is one call, so every command is reachable the same way.
    #[test]
    fn a_command_from_the_list_reaches_the_config() {
        let mut app = app();
        let index = crate::screens::commands::filter("punctuation")
            .first()
            .map(|m| m.command)
            .expect("the command");
        let before = app.config.test.punctuation;
        app.run_command(index);
        assert_ne!(app.config.test.punctuation, before);
    }

    /// Every command in the list has to reach something, or a line in the list is
    /// a line that does nothing when pressed.
    #[test]
    fn no_command_is_a_no_op() {
        use crate::screens::commands::Action as Cmd;
        for (index, command) in crate::screens::commands::COMMANDS.iter().enumerate() {
            // These three leave the bar and are checked by other tests; running
            // them here would only show a side effect.
            if matches!(
                command.action,
                Cmd::CopyResult | Cmd::Quit | Cmd::Languages | Cmd::Settings | Cmd::SetLength(_)
            ) {
                continue;
            }
            let mut app = app();
            let had_window = app.input_window().is_some();
            let before: Vec<String> = app
                .test()
                .words()
                .iter()
                .map(|w| w.text().to_owned())
                .collect();
            app.run_command(index);
            let after: Vec<String> = app
                .test()
                .words()
                .iter()
                .map(|w| w.text().to_owned())
                .collect();
            assert!(
                app.is_dirty() || before != after || !had_window,
                "command {index} ({}) changed nothing",
                command.display
            );
        }
    }

    /// The window is a modal: a key it does not use is a key it swallows, because
    /// letting `f` through would type an `f` into the test while the user is
    /// looking at a search field.
    #[test]
    fn an_open_window_swallows_keys_the_screen_would_have_used() {
        let mut app = app();
        app.on_key(press(KeyCode::Esc)).expect("no io");
        app.on_key(press(KeyCode::F(2))).expect("no io");
        assert_eq!(
            app.screen_kind(),
            ScreenKind::Typing,
            "f2 reached the screen through an open window"
        );
    }

    /// And a character goes into the field rather than into the test.
    #[test]
    fn typing_with_the_window_open_does_not_start_a_test() {
        let mut app = app();
        app.on_key(press(KeyCode::Esc)).expect("no io");
        for c in "zen".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        assert!(
            !app.test().is_started(),
            "the words were typed into the test"
        );
        assert_eq!(app.input_window().map(|w| w.text()), Some("zen"));
    }

    /// The whole path a user actually takes: escape, type a command, enter.
    #[test]
    fn a_command_can_be_run_from_the_keyboard_alone() {
        let mut app = app();
        assert!(app.config.test.punctuation, "the premise changed");
        app.on_key(press(KeyCode::Esc)).expect("no io");
        for c in "punc".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        app.on_key(press(KeyCode::Enter)).expect("no io");
        assert!(!app.config.test.punctuation, "the command did not run");
        assert!(
            !app.input_is_open(),
            "the window stayed open after running one"
        );
    }

    /// The second escape closes the list, which is how a palette is expected to
    /// behave: the first opens it, the second dismisses it.
    #[test]
    fn escape_closes_the_window_it_opened() {
        let mut app = app();
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert!(app.input_is_open());
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert!(!app.input_is_open(), "the window could not be closed");
    }

    /// A duration typed into the window becomes the length, the way the site's
    /// modal does. Reached the way a user reaches it: arrows to the wrench,
    /// enter, type, enter.
    #[test]
    fn a_duration_typed_into_the_window_becomes_the_length() {
        let mut app = app_with_test(crate::config::Mode::Time);
        // The wrench is the last button in the bar.
        for _ in 0..40 {
            app.move_bar_selection(1);
        }
        assert_eq!(app.bar_selection(), Some(Field::TimeCustom));
        app.on_key(press(KeyCode::Enter)).expect("no io");
        assert_eq!(
            app.input_reason(),
            Some(&input::Reason::Length(Field::TimeCustom))
        );
        // The field opens on the value it already had, so it is edited, not
        // retyped from nothing.
        assert_eq!(app.input_window().map(|w| w.text()), Some("30"));
        for _ in 0..2 {
            app.on_key(press(KeyCode::Backspace)).expect("no io");
        }
        for c in "2m".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        app.on_key(press(KeyCode::Enter)).expect("no io");
        assert_eq!(app.config.test.time, 120, "2m did not become 120 seconds");
    }

    /// The bar says so afterwards: a value that is not a preset lights the wrench.
    #[test]
    fn a_custom_duration_lights_the_wrench() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.open_input_with(input::Reason::Length(Field::Time), "");
        for c in "45".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        app.on_key(press(KeyCode::Enter)).expect("no io");
        assert_eq!(app.config.test.time, 45);
        assert!(
            app.bar()
                .right
                .buttons
                .iter()
                .any(|b| b.label == "custom" && b.active),
            "the wrench is not lit for 45 seconds"
        );
    }

    /// Nonsense keeps the window open rather than closing and losing the typing.
    #[test]
    fn nonsense_in_the_window_keeps_it_open() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.open_input_with(input::Reason::Length(Field::Time), "");
        for c in "soon".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        app.on_key(press(KeyCode::Enter)).expect("no io");
        assert!(app.input_is_open(), "the window closed on nonsense");
        assert_eq!(app.input_window().map(|w| w.text()), Some("soon"));
        assert_eq!(app.config.test.time, 30, "the value changed anyway");
    }

    /// `ctrl+c` quits even with the window open. A modal you cannot leave is a
    /// bug, not a feature.
    #[test]
    fn ctrl_c_quits_with_the_window_open() {
        let mut app = app();
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert!(app
            .on_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .expect("no io"));
    }

    /// The arrows move the highlight rather than the bar, because the window is
    /// in front of the bar.
    #[test]
    fn the_arrows_move_the_highlight_and_not_the_bar() {
        let mut app = app();
        app.on_key(press(KeyCode::Esc)).expect("no io");
        let before = app.bar().selected;
        app.on_key(press(KeyCode::Down)).expect("no io");
        assert_eq!(app.bar().selected, before, "the bar moved under the window");
        assert_eq!(
            app.input_window()
                .and_then(|w| w.selected())
                .map(|m| m.command),
            Some(1)
        );
    }

    /// Difficulty is not on the website's test bar, so a setting that lived only
    /// there would have become unreachable when the bar was rebuilt. It is on the
    /// settings screen and in the command list, and both have to work.
    #[test]
    fn difficulty_is_reachable_and_cycles_in_both_directions() {
        use crate::config::Difficulty;
        let before = app().config.test.difficulty;

        let index = crate::screens::commands::filter("harder")
            .first()
            .map(|m| m.command)
            .expect("a harder command");
        let mut app = app();
        app.run_command(index);
        assert_ne!(app.config.test.difficulty, before, "harder did nothing");
        let harder = app.config.test.difficulty;

        let index = crate::screens::commands::filter("easier")
            .first()
            .map(|m| m.command)
            .expect("an easier command");
        app.run_command(index);
        assert_eq!(app.config.test.difficulty, before, "easier did not undo it");
        assert_ne!(harder, Difficulty::Normal, "the premise is that it moved");
    }

    /// And it comes back round, like every other cycle in the app.
    #[test]
    fn difficulty_comes_back_round() {
        let mut app = app();
        let before = app.config.test.difficulty;
        for _ in 0..bar::DIFFICULTIES.len() {
            app.adjust_by(Row::Difficulty, 1);
        }
        assert_eq!(app.config.test.difficulty, before);
    }

    /// A difficulty change rebuilds the words, or the bar would say one difficulty
    /// and the test would be another.
    #[test]
    fn a_difficulty_change_rebuilds_the_test() {
        let mut app = app();
        let before: Vec<String> = app
            .test()
            .words()
            .iter()
            .map(|w| w.text().to_owned())
            .collect();
        app.adjust_by(Row::Difficulty, 1);
        let after: Vec<String> = app
            .test()
            .words()
            .iter()
            .map(|w| w.text().to_owned())
            .collect();
        assert_ne!(
            before, after,
            "the words did not change with the difficulty"
        );
    }

    // ---- the two modes --------------------------------------------------

    use crate::screens::modes::Mode as Focus;

    /// Escape opens the window and the keyboard with it. The two cannot be
    /// separate: a window that is open while the mode says "navigation" is a state
    /// where a letter is a command and a character at the same time.
    #[test]
    fn opening_the_window_and_taking_the_keyboard_are_the_same_thing() {
        let mut app = app();
        assert_eq!(app.mode(), Focus::Navigation);
        assert!(!app.input_is_open());
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(
            app.mode(),
            Focus::Input,
            "the window is open but the mode is not"
        );
        assert!(app.input_is_open());
    }

    /// And whatever opened the window, however it was opened, brought the mode with
    /// it. A bar button that opens a dialog and leaves the keyboard in navigation
    /// would be a dialog that types into the test.
    #[test]
    fn every_way_of_opening_the_window_takes_the_keyboard() {
        use crate::config::bar::Field;
        let ways: [fn(&mut App); 3] = [
            |app| app.open_input(input::Reason::Command),
            |app| app.open_input_with(input::Reason::Text, "x"),
            |app| app.open_input(input::Reason::Length(Field::Time)),
        ];
        for open in ways {
            let mut app = app();
            assert_eq!(app.mode(), Focus::Navigation, "the premise changed");
            open(&mut app);
            assert_eq!(app.mode(), Focus::Input);
            assert!(app.input_is_open());
        }
    }

    /// Closing without acting must not run whatever was highlighted. A dismissed
    /// palette that runs a command is a palette that does something on the way out.
    #[test]
    fn escape_closes_the_window_without_running_anything() {
        let mut app = app();
        let before = app.config.test.punctuation;
        app.on_key(press(KeyCode::Esc)).expect("no io");
        for c in "punc".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        // The first match is highlighted.
        let highlighted = app
            .input_window()
            .and_then(|w| w.selected())
            .map(|m| m.command);
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert!(!app.input_is_open());
        assert_eq!(app.mode(), Focus::Navigation);
        assert_eq!(
            app.config.test.punctuation, before,
            "escape ran the highlighted command"
        );
        assert!(highlighted.is_some(), "nothing was highlighted to dismiss");
    }

    /// An unreadable value leaves the window open, so it must also leave the
    /// keyboard in it. Closing the window here would throw away the typing and
    /// then send the next letter to the test.
    #[test]
    fn a_value_that_cannot_be_read_keeps_the_keyboard_too() {
        let mut app = app_with_test(crate::config::Mode::Time);
        app.open_input_with(input::Reason::Length(Field::Time), "");
        app.close_input(input::Outcome::Invalid("soon".to_owned()));
        assert!(app.input_is_open());
        assert_eq!(
            app.mode(),
            Focus::Input,
            "the window is open but the keyboard is not"
        );
    }

    /// `tab` is *skip* on the typing screen and a mode switch everywhere else. The
    /// typing screen is the one place where taking it away would break the command
    /// every monkeytype user knows.
    #[test]
    fn tab_is_skip_on_the_typing_screen_and_a_mode_switch_elsewhere() {
        let mut app = app();
        app.on_key(press(KeyCode::Tab)).expect("no io");
        assert_eq!(
            app.mode(),
            Focus::Navigation,
            "tab took the keyboard on the typing screen"
        );
        assert!(!app.input_is_open());

        app.on_key(press(KeyCode::F(2))).expect("no io");
        assert_eq!(app.screen_kind(), ScreenKind::Settings);
        app.on_key(press(KeyCode::Tab)).expect("no io");
        assert_eq!(
            app.mode(),
            Focus::Input,
            "tab did not switch on the settings screen"
        );
        assert!(app.input_is_open());
    }

    /// `i` enters input mode, the way it does in vim — and only where a letter is
    /// not text. On the typing screen it is a letter.
    #[test]
    fn i_is_a_mode_key_away_from_the_test_and_a_letter_in_it() {
        let mut app = app();
        app.on_key(press(KeyCode::F(2))).expect("no io");
        app.on_key(press(KeyCode::Char('i'))).expect("no io");
        assert_eq!(
            app.mode(),
            Focus::Input,
            "i did not open the window on the settings screen"
        );

        let mut app = self::app();
        for c in "i".chars() {
            app.type_char(c);
        }
        assert_eq!(
            app.test().words()[0].input(),
            "i",
            "i did not reach the words on the typing screen"
        );
        assert_eq!(app.mode(), Focus::Navigation);
    }

    /// `q` quits from a browsing screen. It has to: a mode where letters are
    /// commands and `q` is not one of them is a mode where `q` does nothing, and
    /// that is what every other program with letters does.
    #[test]
    fn q_quits_from_a_browsing_screen() {
        let mut app = self::app();
        app.on_key(press(KeyCode::F(2))).expect("no io");
        assert!(
            app.on_key(press(KeyCode::Char('q'))).expect("no io"),
            "q did not quit from the settings screen"
        );
    }

    /// And it is a letter on the typing screen, which is the whole reason the mode
    /// has two settings and not one.
    #[test]
    fn q_is_a_letter_on_the_typing_screen() {
        let mut app = self::app();
        app.type_char('q');
        assert_eq!(app.test().words()[0].input(), "q");
        assert!(!app.test().is_finished());
    }

    /// The vim keys move the settings selection, and each one only in its own
    /// direction — which is the bug this whole change started from.
    #[test]
    fn the_vim_keys_move_the_settings_selection() {
        use crate::screens::modes::Direction;
        let mut app = app();
        app.on_key(press(KeyCode::F(2))).expect("no io");
        let start = app.selected_row();
        app.on_key(press(KeyCode::Char('j'))).expect("no io");
        assert_ne!(app.selected_row(), start, "j did not move down");
        app.on_key(press(KeyCode::Char('k'))).expect("no io");
        assert_eq!(app.selected_row(), start, "k did not move back up");
        let _ = Direction::Down;
    }

    /// And they are letters in the window, because the field is where they belong.
    #[test]
    fn the_vim_keys_are_letters_in_the_window() {
        let mut app = app();
        app.on_key(press(KeyCode::Esc)).expect("no io");
        for c in "hjkl".chars() {
            app.on_key(press(KeyCode::Char(c))).expect("no io");
        }
        assert_eq!(app.input_window().map(|w| w.text()), Some("hjkl"));
    }

    /// The status line says which mode the keyboard is in. A mode that cannot be
    /// seen is a mode nobody can learn.
    #[test]
    fn the_status_line_says_which_mode_the_keyboard_is_in() {
        let mut app = app();
        assert!(app.mode() == Focus::Navigation);
        assert_eq!(Focus::Navigation.label(), "NAV");
        assert_eq!(Focus::Input.label(), "INS");
        app.on_key(press(KeyCode::Esc)).expect("no io");
        assert_eq!(app.mode().label(), "INS");
    }

    /// Every mode, every surface, both window states, and no key panics or falls
    /// out of the rules. The rules are asked on *every* key, so a gap in them is
    /// not a missing feature — it is a keypress that does something undefined.
    #[test]
    fn no_key_does_anything_undefined_in_any_mode() {
        let keys = [
            KeyCode::Char('a'),
            KeyCode::Char('j'),
            KeyCode::Char('q'),
            KeyCode::Char('i'),
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Backspace,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::F(2),
        ];
        for screen in [
            ScreenKind::Typing,
            ScreenKind::Settings,
            ScreenKind::Results,
        ] {
            for open in [false, true] {
                for code in keys {
                    // A fresh app per key: `q` quits and `enter` can leave, and a
                    // loop that carried one app across all of them would only be
                    // testing the first key.
                    let mut one = app();
                    if screen != ScreenKind::Typing {
                        one.show_screen(screen);
                    }
                    if open {
                        one.open_input(input::Reason::Command);
                    }
                    let _ = one.on_key(press(code));
                }
            }
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
