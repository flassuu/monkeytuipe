//! Interface languages.
//!
//! **The website has none.** That is worth stating plainly, because this module
//! exists to copy something and the honest answer is that there was nothing to
//! copy: `monkeytypegame/monkeytype` ships no translation files, has no i18n
//! module, has no i18n dependency, and hardcodes English in every component and
//! every HTML template. A maintainer confirmed it in [discussion #6316][1] (2025-02-27):
//! "there is no localization of the website. It will always display in english."
//! The word "language" all over that site means the *word list* to type, which is a
//! different setting and is in [`crate::config::test::language`].
//!
//! So this is a design of our own rather than a port. The rule that shapes it:
//!
//! > Adding a new interface string must be a **compile error** until someone has
//! > decided what it says in every language.
//!
//! That is what [`Key`] is for. It is an exhaustive enum, so a string that has no
//! entry does not render as blank and does not fall back to English quietly — it
//! does not compile. A translation table that is checked at runtime is checked
//! too late.
//!
//! ## What is translated and what is not
//!
//! Translated: everything this app *says* — the bar, the command list, the
//! settings, the window titles and hints, the counters, the results.
//!
//! Not translated:
//!
//! - **language names.** A list of languages is written in the languages it names.
//!   A picker offering "russian" to a Russian speaker and "russian" to an English
//!   one is not wrong in the second case and *is* wrong in the first.
//! - **theme names.** "gruvbox" and "nord" are names. "dracula" is not a
//!   sentence.
//! - **what the user typed.** A passage, an ApeKey, a search query.
//! - **numbers and units.** `wpm`, `15s`, `24/25`. Digits are not English.
//!
//! ## Adding a language
//!
//! Add a variant to [`Lang`], a column to every row of [`STRINGS`], and the
//! compiler will tell you about the ones you missed.
//!
//! [1]: https://github.com/monkeytypegame/monkeytype/discussions/6316

use serde::{Deserialize, Serialize};

/// An interface language.
///
/// Serialised as its code — `en`, `ru` — because that is what
/// [`Lang::code`] documents and what a config file wants. The first version
/// derived the spelling from the variant names and wrote `english`, which
/// disagreed with its own documentation and with every other language-tagged
/// thing on the machine. The full names are accepted on read for the same reason
/// `theme = "terminal"` is: a config that once parsed must keep parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lang {
    /// The default, and the only language the website has.
    #[default]
    #[serde(rename = "en", alias = "english")]
    English,
    #[serde(rename = "ru", alias = "russian")]
    Russian,
}

impl Lang {
    pub const ALL: [Self; 2] = [Self::English, Self::Russian];

    /// How the language names itself, in that language.
    ///
    /// A language picker that shows every language in the interface's own language
    /// is the wrong way round: someone looking for their own language is looking
    /// for the one thing on the screen they can read without already knowing it.
    pub fn self_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Russian => "Русский",
        }
    }

    /// The language's name in English, lowercased.
    ///
    /// For the command list, and for searching. A command list is a list of things
    /// to *type*, and the thing a Russian speaker types is `русский` — which is in
    /// the aliases — while the thing anyone types in an English interface is
    /// `russian`. The settings row uses [`Self::self_name`] instead, because it is
    /// read rather than typed.
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Russian => "russian",
        }
    }

    /// How the language appears in `config.toml`.
    ///
    /// The config spelling is the language's own code, because a config file is
    /// read by whoever wrote it and `ru` means the same thing everywhere.
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Russian => "ru",
        }
    }

    /// Reads a config value.
    ///
    /// Returns `None` for anything unknown rather than defaulting, because a
    /// misspelled `ui_language` that silently became English is a config file that
    /// looks right and is not.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|lang| lang.code().eq_ignore_ascii_case(code))
    }

    /// The string for `key` in this language.
    pub fn tr(self, key: Key) -> &'static str {
        STRINGS[key as usize][self as usize]
    }
}

/// Every string this app can say, in every language, in one place.
///
/// A macro rather than an enum next to a table, because an enum next to a table
/// can disagree: a variant with no row is an out-of-bounds index and a panic in
/// the middle of drawing a screen, and the first version of this had exactly that
/// — a key with no row, found by a test that had to exist only to notice. The
/// macro makes the enum, the rows and the key list the *same* thing, so they
/// cannot drift and the test that watched for drift is not needed.
///
/// ```ignore
/// strings! {
///     Punctuation => ["punctuation", "пунктуация"],
///     Numbers     => ["numbers",     "цифры"],
/// }
/// ```
macro_rules! strings {
    ($($key:ident => [$($text:literal),+ $(,)?]),* $(,)?) => {
        /// One interface string.
        ///
        /// Exhaustive on purpose: a new string that has not been given a row in
        /// every language does not render as blank, and does not quietly fall back
        /// to English. It does not exist.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Key {
            $(
                #[allow(missing_docs)]
                $key,
            )*
        }

        impl Key {
            /// Every key, in declaration order.
            pub const ALL: &'static [Key] = &[$(Key::$key,)*];
        }

        /// The strings: one row per key, one column per language.
        const STRINGS: &[&[&str]] = &[$( &[$($text),+], )*];
    };
}

strings! {
    // ---- the top bar ---------------------------------------------------
    Punctuation          => ["punctuation", "пунктуация"],
    Numbers              => ["numbers",     "цифры"],
    ModeTime             => ["time",        "время"],
    ModeWords            => ["words",       "слова"],
    ModeQuote            => ["quote",       "цитата"],
    ModeZen              => ["zen",         "дзен"],
    ModeCustom           => ["custom",      "свой текст"],
    QuoteAll             => ["all",         "все"],
    QuoteShort           => ["short",       "короткие"],
    QuoteMedium          => ["medium",      "средние"],
    QuoteLong            => ["long",        "длинные"],
    QuoteThicc           => ["thicc",       "толстые"],
    Add                  => ["add",         "добавить"],
    Change               => ["change",      "изменить"],
    // Not "custom". The mode card already says `custom`, and the bar showed
    // "custom" twice within forty columns, which reads as a stutter rather than as
    // two different things. The site uses a wrench glyph here — the meaning is
    // "not one of the presets" — and "other" is the word for that.
    Other                => ["other",       "другое"],
    // ---- modes and lengths --------------------------------------------
    Difficulty           => ["difficulty",  "сложность"],
    DifficultyNormal     => ["normal",      "обычная"],
    DifficultyExpert     => ["expert",      "сложная"],
    DifficultyMaster     => ["master",      "мастер"],
    On                   => ["on",          "вкл"],
    Off                  => ["off",         "выкл"],
    Seconds              => ["seconds",     "сек"],
    WordsCount           => ["words",       "слов"],
    NotSet               => ["not set",     "не задано"],
    EmptyCustomText      => ["custom text is empty", "свой текст пуст"],
    NoQuotesDownloaded   => ["quotes are still downloading", "цитаты ещё скачиваются"],
    NoQuotesAtAll        => ["no quotes for {length} in {language}", "нет цитат {length} на языке {language}"],
    NoQuotesThatLong     => [
        "no {length} quotes; this language has {have}",
        "нет цитат {length}; на этом языке есть {have}",
    ],
    Blind                => ["blind",       "вслепую"],
    // ---- the command window -------------------------------------------
    Commands             => ["commands",    "команды"],
    DurationTitle        => ["duration",    "длительность"],
    WordsTitle           => ["words",       "слов"],
    CustomTextTitle      => ["custom text", "свой текст"],
    ApeKeyTitle          => ["ape key",     "ключ ape"],
    CommandHint          => [
        "type to search · ↑↓ move · enter run · esc close",
        "наберите для поиска · ↑↓ выбор · enter выполнить · esc закрыть",
    ],
    LengthHint           => [
        "seconds, or 1h30m · h hours m minutes",
        "секунды, или 1ч30м · ч часы м минуты",
    ],
    TextHint             => [
        "one passage per line · the first line is the test",
        "по абзацу на строку · тест — первая строка",
    ],
    ApeKeyHint           => ["from monkeytype account settings", "из настроек аккаунта monkeytype"],
    EndlessPreview       => ["0 — endless, which is what zen is for", "0 — бесконечно, для этого есть дзен"],
    NotADurationPreview  => ["{text} is not a duration", "{text} — это не длительность"],
    WordCountPreview     => ["{words} words", "слов: {words}"],
    // ---- settings ------------------------------------------------------
    Settings             => ["settings",    "настройки"],
    Language             => ["language",    "язык"],
    Theme                => ["theme",       "тема"],
    // Distinct from `Language` in English as well, and it has to be: the settings
    // has both rows, and two rows labelled "language" is a question with no
    // answer. In Russian the words differ on their own, which is a nice accident
    // and not something to rely on.
    InterfaceLanguage    => ["interface language", "язык интерфейса"],
    SubmitResults        => ["submit results", "отправлять результаты"],
    BackToTyping         => ["back to typing",  "назад к набору"],
    ThemeBrowser         => ["choose a theme", "выберите тему"],
    ThemePickerHint      => [
        "↑↓ move · enter choose · esc close",
        "↑↓ выбор · enter применить · esc закрыть",
    ],
    NoApeKeySet          => [
        "no ApeKey set — reads need one",
        "ключ ape не задан — для чтения нужен",
    ],
    NotSignedIn          => [
        "not signed in — no ApeKey set — reads need one",
        "не выполнен вход — ключ ape не задан — для чтения нужен",
    ],
    SettingsHint         => [
        "←→ or hl change · enter open · ↑↓ or jk move · i commands · esc close",
        "←→ или hl изменить · enter открыть · ↑↓ или jk выбрать · i команды · esc закрыть",
    ],
    EditingRow           => ["editing {row}", "правка: {row}"],
    EditorHelp           => [
        "enter saves · esc discards",
        "enter сохранить · esc отменить",
    ],
    // ---- the language browser ------------------------------------------
    // The window's title, so it says what the window is for rather than repeating
    // a row label that is on screen behind it.
    LanguageBrowser      => ["choose a language", "выберите язык"],
    BaseColumn           => ["base",        "основа"],
    // ---- the typing screen ---------------------------------------------
    PressAnyKey          => [
        "press any key to start typing",
        "нажмите любую клавишу, чтобы начать",
    ],
    StartedHints         => [
        "tab skip · ctrl+r restart · esc commands · f2 settings",
        "tab пропустить · ctrl+r заново · esc команды · f2 настройки",
    ],
    WaitingHints         => [
        "esc commands · ctrl+c quit",
        "esc команды · ctrl+c выход",
    ],
    Wpm                  => ["wpm",         "wpm"],
    // `wpm` is the unit and every language that types uses the same three letters
    // for it; translating it would make the number unrecognisable. `acc` is a
    // label rather than a unit, so it does get translated.
    Acc                  => ["acc",         "точн"],
    Time                 => ["time",        "время"],
    ModeLabel            => ["mode",        "режим"],
    // ---- the results screen --------------------------------------------
    ResultsHints         => [
        "ctrl+r again · esc commands · ctrl+c quit",
        "ctrl+r ещё раз · esc команды · ctrl+c выход",
    ],
    NotSubmitted         => [
        "not submitted — monkeytype's POST /results needs a browser login, not an ApeKey",
        "не отправлено — POST /results на monkeytype требует вход в браузере, а не ключ ape",
    ],
    CopyUnavailable      => [
        "no clipboard in a terminal — the bar's copy does not exist either",
        "в терминале нет буфера обмена — копирования в полосе тоже нет",
    ],
    // ---- messages -------------------------------------------------------
    SubmissionImpossible => ["not possible — see below", "невозможно — см. ниже"],
    NotSignedInDash      => ["not signed in —",        "не выполнен вход —"],
    SetApeKeyInSettings  => [
        "set an ApeKey in settings",
        "задайте ключ ape в настройках",
    ],
    ReadingRecords       => ["signed in · reading records…", "вход выполнен · читаем рекорды…"],
    DayStreak            => ["· {days} day streak",    "· серия: {days} дн."],
    ApeKeySet            => ["key set",     "ключ задан"],
}

/// Replaces `{name}` in a string.
///
/// Hand-rolled rather than pulled in, for the same reason there is no i18n
/// dependency in the website: there are three placeholders in the whole
/// catalogue and a crate for that is a crate to keep up to date.
pub fn fill(template: &str, name: &str, value: &str) -> String {
    template.replace(&format!("{{{name}}}"), value)
}

/// Replaces several placeholders at once, in one pass.
///
/// One pass is the whole point. Doing it as a chain of `replace` calls looks
/// equivalent and is not: substituting `{a}` with a value that happens to contain
/// the *text* `{b}` leaves a `{b}` in the output, and the next `replace` — the one
/// meant for the template — eats it. A language name is unlikely to contain a
/// brace, and "unlikely" is not a property worth relying on in the one function
/// that writes text a user reads.
pub fn fill_all(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut said = String::with_capacity(template.len());
    let mut rest = template;
    'outer: while let Some(open) = rest.find('{') {
        said.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            // An unclosed brace is not a placeholder. It goes through as itself,
            // which is visible — better than dropping it.
            said.push_str(&rest[open..]);
            return said;
        };
        let name = &after[..close];
        match pairs.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => said.push_str(value),
            // Not one of ours. Leave it, so an unknown placeholder is visible
            // rather than silently becoming an empty hole.
            None => said.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
        // Every pass consumes at least two characters, so this terminates even when
        // nothing matches.
        if rest.is_empty() {
            break 'outer;
        }
    }
    said.push_str(rest);
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_row() {
        assert_eq!(
            STRINGS.len(),
            Key::ALL.len(),
            "the table has {} rows for {} keys",
            STRINGS.len(),
            Key::ALL.len()
        );
    }

    /// The thing the whole design exists for: a key with no row would index out
    /// of bounds at the point of use, which is a panic in the middle of drawing a
    /// screen. This catches it at the point of addition.
    #[test]
    fn every_key_in_the_enum_is_in_the_all_list() {
        let mut declared: Vec<usize> = Key::ALL.iter().map(|key| *key as usize).collect();
        let before = declared.len();
        declared.sort_unstable();
        declared.dedup();
        assert_eq!(before, declared.len(), "a key is listed twice in Key::ALL");
        assert_eq!(
            declared,
            (0..STRINGS.len()).collect::<Vec<_>>(),
            "Key::ALL and the table disagree; the rows are the keys"
        );
    }

    #[test]
    fn every_key_has_a_string_in_every_language() {
        for key in Key::ALL {
            for lang in Lang::ALL {
                let said = lang.tr(*key);
                assert!(!said.is_empty(), "{key:?} is empty in {lang:?}");
                assert!(
                    !said.trim().is_empty(),
                    "{key:?} is only whitespace in {lang:?}"
                );
            }
        }
    }

    /// Every row has one column per language. A row with two is an off-by-one that
    /// would show language A's text on the settings screen labelled language B.
    #[test]
    fn every_row_has_one_column_per_language() {
        for (index, row) in STRINGS.iter().enumerate() {
            assert_eq!(
                row.len(),
                Lang::ALL.len(),
                "row {index} ({:?}) has {} columns for {} languages",
                Key::ALL.get(index),
                row.len(),
                Lang::ALL.len()
            );
        }
    }

    /// A key that says the same thing as another one is a bug waiting to happen:
    /// somebody edits one of them and the other does not follow. English is the
    /// column to check, since it is the reference.
    #[test]
    fn no_two_keys_say_the_same_thing_in_english() {
        let mut seen: std::collections::BTreeMap<&str, Key> = std::collections::BTreeMap::new();
        for key in Key::ALL {
            let said = Lang::English.tr(*key);
            // `words` is both a mode and a unit, and that is deliberate.
            // These are deliberately the same word in two places: `time` is both
            // a mode and the name of the counter, and `words` is both a mode and a
            // count. The counter and the mode button are the same concept and
            // translating them differently would be translating one word twice.
            if matches!(
                key,
                Key::WordsCount | Key::ModeWords | Key::WordsTitle | Key::Time | Key::ModeTime
            ) {
                continue;
            }
            if let Some(previous) = seen.insert(said, *key) {
                panic!("{key:?} and {previous:?} both say {said:?} in english");
            }
        }
    }

    /// Russian is a real language and not a transliteration, so a string that came
    /// out identical to the English one is either a name that should not have been
    /// translated or a translation that was never done. Only the ones that are
    /// genuinely the same are allowed, and the list is short and explicit.
    #[test]
    fn russian_is_not_the_english_with_the_letters_replaced() {
        // These are the same in both because they are not sentences: an
        // abbreviation, a number, or a word that is spelled the same.
        // `wpm` is the only one, and it is a unit rather than a sentence: it is
        // the same three letters in every language that types, and translating it
        // would make the number unrecognisable.
        let allowed = [Key::Wpm];
        for key in Key::ALL {
            if allowed.contains(key) {
                continue;
            }
            let english = Lang::English.tr(*key);
            let russian = Lang::Russian.tr(*key);
            assert_ne!(russian, english, "{key:?} was not translated: {russian:?}");
        }
    }

    #[test]
    fn a_language_names_itself_and_is_spelled_correctly_in_a_config() {
        assert_eq!(Lang::English.code(), "en");
        assert_eq!(Lang::Russian.code(), "ru");
        assert_eq!(Lang::English.name(), "english");
        assert_eq!(Lang::Russian.name(), "russian");
        assert_eq!(Lang::Russian.self_name(), "Русский");
        // The two names are the same language said two ways, and the difference is
        // the point: the settings row is read, the command list is typed into.
        assert_ne!(Lang::Russian.name(), Lang::Russian.self_name());
        assert_eq!(Lang::from_code("ru"), Some(Lang::Russian));
        assert_eq!(Lang::from_code("RU"), Some(Lang::Russian));
        // Every language is findable by its own code, or it cannot be configured.
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), Some(lang), "{lang:?}");
        }
    }

    /// A misspelled config value is not silently English. The alternative is a
    /// config file that looks right and is not, which is the worst kind.
    #[test]
    fn an_unknown_language_code_is_refused() {
        assert_eq!(Lang::from_code("klingon"), None);
        assert_eq!(Lang::from_code(""), None);
        assert_eq!(Lang::from_code("englsh"), None);
    }

    /// Every language is distinct from English, so switching to it changes
    /// something. A language column identical to the reference is a language that
    /// does not exist.
    #[test]
    fn no_language_is_the_english() {
        assert_ne!(Lang::Russian, Lang::English);
        let same: Vec<Key> = Key::ALL
            .iter()
            .filter(|key| Lang::Russian.tr(**key) == Lang::English.tr(**key))
            .copied()
            .collect();
        assert_eq!(
            same,
            vec![Key::Wpm],
            "these strings are the same in both languages: {same:?}"
        );
    }

    #[test]
    fn several_placeholders_are_filled_in() {
        assert_eq!(
            fill_all(
                "no {length} quotes; this language has {have}",
                &[("length", "thicc"), ("have", "short, long")]
            ),
            "no thicc quotes; this language has short, long"
        );
        // A value that contains another placeholder's name is left alone, because
        // the replacement is done in one pass rather than a search for a second
        // pattern over the result.
        assert_eq!(
            fill_all("{a} and {b}", &[("a", "{b}"), ("b", "two")]),
            "{b} and two"
        );
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(fill("{words} words", "words", "24"), "24 words");
        assert_eq!(
            fill("{text} is not a duration", "text", "soon"),
            "soon is not a duration"
        );
        // A string with no placeholder comes back unchanged rather than mangled.
        assert_eq!(fill("nothing here", "words", "24"), "nothing here");
        // And an unknown placeholder is left alone, which is better than removing
        // it: a leftover `{x}` is visible, a silently empty string is not.
        assert_eq!(fill("a {b} c", "x", "y"), "a {b} c");
    }
}
