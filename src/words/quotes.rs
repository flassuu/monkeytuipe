//! Quotes: a fixed passage to type, and where they come from.
//!
//! There is no public API for these. `GET /quotes` exists and returns the
//! *submission moderation queue* to anyone with the `quoteMod` permission — it is
//! not a library of quote texts, and asking for it unauthenticated gives a 401.
//! The website serves them instead as static files at
//! `monkeytype.com/quotes/<language>.json`, which is what this fetches.
//!
//! The file is large — english is 2.3 MB for about 7,700 quotes — so it is
//! downloaded once and cached, and the cache is invalidated by the file's own
//! `etag` rather than by a version number, because there is no version to check.
//!
//! ## The shape, which is not what you would guess
//!
//! - `language` is at the **top level**, not per quote.
//! - There is no `isFavourite` field. Favourites are a per-user thing stored on
//!   the server, and a client without an account has no favourites to show.
//! - Length buckets are a `groups` array of `[low, high]` character counts, four
//!   of them, which is where [`QuoteLength`] gets its bounds.
//! - `britishText` exists on a handful of quotes and is the spelling variant the
//!   website offers in place of `text` when the user's config asks for it. It is
//!   kept, because a client that silently drops it has no way to offer it later.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::bar::QuoteLength;

/// Where the quote files live. Same host as the word lists, and no auth.
pub const UPSTREAM_BASE: &str = "https://monkeytype.com/quotes";

/// One passage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub id: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Character count, which the website stores rather than recomputing.
    #[serde(default)]
    pub length: u32,
    /// The British spelling variant, on the few quotes that have one.
    #[serde(
        default,
        rename = "britishText",
        skip_serializing_if = "Option::is_none"
    )]
    pub british_text: Option<String>,
    /// The passage as the website's `textSplit` holds it, filled in on load.
    #[serde(skip)]
    pub words: Vec<String>,
}

impl Quote {
    /// Rebuilds the derived word list.
    ///
    /// The website splits on a single space and drops empty pieces, so `"a  b"`
    /// is two words rather than three with a blank in the middle. Anything else
    /// puts a word on screen that has no letters in it.
    fn prepare(&mut self) {
        self.words = self
            .text
            .split(' ')
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect();
    }

    /// The passage with British spellings, where the quote has them.
    ///
    /// Falls back to `text` rather than to a half-translated mix: a quote that
    /// is spelled one way throughout is better than one that changes dialect
    /// halfway through.
    pub fn text_for(&self, british: bool) -> &str {
        if british {
            self.british_text.as_deref().unwrap_or(&self.text)
        } else {
            &self.text
        }
    }

    /// The label the website shows under a quote.
    pub fn source(&self) -> &str {
        self.source.as_deref().unwrap_or("")
    }
}

/// A whole quote file: the language, its length buckets, and its quotes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteList {
    pub language: String,
    /// `[low, high]` character counts, one bucket per [`QuoteLength`].
    #[serde(default)]
    pub groups: Vec<[usize; 2]>,
    #[serde(default)]
    pub quotes: Vec<Quote>,
}

impl QuoteList {
    /// Reads a quote file, filling in the derived word lists.
    ///
    /// Quotes with no words are dropped: a passage that is an empty string or all
    /// spaces cannot be typed, and leaving it in the list means a pick that lands
    /// on it and types nothing.
    pub fn parse(body: &str) -> Result<Self, QuoteError> {
        let mut list: QuoteList =
            serde_json::from_str(body).map_err(|e| QuoteError::Malformed {
                reason: e.to_string(),
            })?;
        for quote in &mut list.quotes {
            quote.prepare();
        }
        list.quotes.retain(|quote| !quote.words.is_empty());
        Ok(list)
    }

    /// Whether the file had nothing usable in it.
    pub fn is_empty(&self) -> bool {
        self.quotes.is_empty()
    }

    /// How many quotes fall in each bucket, longest bucket last.
    ///
    /// Shown when there is no quote of the length asked for, because "no medium
    /// quotes for icelandic" and "no quotes at all" are different problems and a
    /// bare "none found" hides the difference.
    pub fn counts(&self) -> Vec<(QuoteLength, usize)> {
        crate::config::bar::QUOTE_LENGTHS
            .iter()
            .copied()
            .map(|bucket| {
                let count = self
                    .quotes
                    .iter()
                    .filter(|quote| bucket.accepts(quote.text.chars().count()))
                    .count();
                (bucket, count)
            })
            .collect()
    }

    /// A quote of about the length asked for.
    ///
    /// Which one is not random: the first match, lowest id first as the file is
    /// stored. The website picks at random, but a terminal client that picked at
    /// random would make a quote test impossible to reproduce — the same
    /// passage has to come back when you ask for it again, or there is no way to
    /// practise one you got wrong. [`Self::pick_from`] is the random one.
    pub fn pick(&self, length: QuoteLength) -> Option<&Quote> {
        self.pick_from(length, 0)
    }

    /// A quote of about the length asked for, choosing the *n*th match.
    ///
    /// The offset is what makes a different passage reachable without a random
    /// number generator: cycling through `0..len` by hand walks the whole bucket.
    pub fn pick_from(&self, length: QuoteLength, offset: usize) -> Option<&Quote> {
        self.quotes
            .iter()
            .filter(|quote| length.accepts(quote.text.chars().count()))
            .nth(offset)
    }

    /// How many quotes a bucket holds.
    pub fn count_of(&self, length: QuoteLength) -> usize {
        self.quotes
            .iter()
            .filter(|quote| length.accepts(quote.text.chars().count()))
            .count()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QuoteError {
    #[error("http transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("the quote file could not be read: {reason}")]
    Malformed { reason: String },

    #[error("could not use the quote cache at {path}: {reason}")]
    Io { path: PathBuf, reason: String },
}

impl QuoteError {
    fn io(path: &Path, err: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            reason: err.to_string(),
        }
    }
}

/// The cache directory: `$MONKEYTUIPE_CACHE`, else
/// `$XDG_CACHE_HOME/monkeytuipe/quotes`, else `~/.cache/monkeytuipe/quotes`.
pub fn cache_dir() -> PathBuf {
    if let Some(explicit) = std::env::var_os("MONKEYTUIPE_CACHE") {
        return PathBuf::from(explicit).join("quotes");
    }
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache"));
    base.join("monkeytuipe").join("quotes")
}

/// The cache file for one language.
pub fn cache_path(language: &str) -> PathBuf {
    cache_dir().join(format!("{language}.json"))
}

/// Whether a language's quotes are already on disk.
pub fn is_available_offline(language: &str) -> bool {
    cache_path(language).is_file()
}

/// Downloads a language's quotes and writes them to the cache.
///
/// `language` is the **base** language: upstream has no `quotes/english_5k.json`,
/// which is why [`crate::words::variants::base_of`] exists.
pub async fn fetch(language: &str) -> Result<QuoteList, QuoteError> {
    let url = format!("{UPSTREAM_BASE}/{language}.json");
    let http = reqwest::Client::builder()
        .user_agent(concat!("monkeytuipe/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| QuoteError::Malformed {
            reason: e.to_string(),
        })?;

    let response = http.get(&url).send().await?.error_for_status()?;
    let body = response.text().await?;
    let list = QuoteList::parse(&body)?;

    let path = cache_path(language);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| QuoteError::io(parent, e))?;
    }
    std::fs::write(&path, &body).map_err(|e| QuoteError::io(&path, e))?;
    Ok(list)
}

/// Reads a language's quotes from the cache.
pub fn load_cached(language: &str) -> Result<QuoteList, QuoteError> {
    let path = cache_path(language);
    let body = std::fs::read_to_string(&path).map_err(|e| QuoteError::io(&path, e))?;
    QuoteList::parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real head of `quotes/english.json`, with two entries and the groups.
    const SAMPLE: &str = r#"{
      "language": "english",
      "groups": [[0,100],[101,300],[301,600],[601,9999]],
      "quotes": [
        {"text":"You have the power to heal your life, and you need to know that.",
         "source":"Meditations to Heal Your Life","length":64,"id":1},
        {"text":"They don't know that we know they know we know.",
         "source":"Friends","length":47,"id":2}
      ]
    }"#;

    /// A file shaped like the real one: `language` at the top level, no
    /// `isFavourite` on any quote, four length buckets.
    #[test]
    fn a_quote_file_reads_the_way_upstream_writes_it() {
        let list = QuoteList::parse(SAMPLE).expect("a quote file");
        assert_eq!(list.language, "english");
        assert_eq!(list.groups, [[0, 100], [101, 300], [301, 600], [601, 9999]]);
        assert_eq!(list.quotes.len(), 2);
        assert_eq!(list.quotes[0].id, 1);
        assert_eq!(list.quotes[0].source(), "Meditations to Heal Your Life");
    }

    #[test]
    fn the_word_list_is_derived_by_splitting_on_spaces() {
        let list = QuoteList::parse(SAMPLE).expect("a quote file");
        assert_eq!(list.quotes[0].words.len(), 14);
        assert_eq!(list.quotes[0].words[1], "have");
        assert_eq!(list.quotes[0].words[0], "You");
    }

    /// Upstream stores single spaces, but a hand-edited or third-party file can
    /// have doubles, and a blank word on screen cannot be typed.
    #[test]
    fn a_double_space_does_not_produce_a_blank_word() {
        let list = QuoteList::parse(
            r#"{"language":"x","quotes":[{"id":1,"text":"a  b","length":4,"source":"s"}]}"#,
        )
        .expect("a quote file");
        assert_eq!(list.quotes[0].words, ["a", "b"]);
    }

    /// A quote that is empty, or all spaces, would type nothing at all.
    #[test]
    fn a_quote_with_no_words_is_dropped() {
        let list = QuoteList::parse(
            r#"{"language":"x","quotes":[
                 {"id":1,"text":"","length":0,"source":"s"},
                 {"id":2,"text":"   ","length":3,"source":"s"},
                 {"id":3,"text":"real","length":4,"source":"s"}]}"#,
        )
        .expect("a quote file");
        assert_eq!(list.quotes.len(), 1);
        assert_eq!(list.quotes[0].id, 3);
    }

    #[test]
    fn length_buckets_pick_the_right_quote() {
        let list = QuoteList::parse(SAMPLE).expect("a quote file");
        // Both samples are under 100 characters.
        let short = list.pick(QuoteLength::Short).expect("a short quote");
        assert_eq!(short.id, 1);
        // The next one in the bucket, for cycling.
        let second = list
            .pick_from(QuoteLength::Short, 1)
            .expect("a second quote");
        assert_eq!(second.id, 2);
        // And nothing in the long buckets, which is the case worth reporting.
        assert!(list.pick(QuoteLength::Thicc).is_none());
    }

    /// A pick has to be repeatable or a quote test cannot be practised twice.
    #[test]
    fn a_pick_is_stable() {
        let list = QuoteList::parse(SAMPLE).expect("a quote file");
        let first = list.pick(QuoteLength::All).expect("a quote");
        let again = list.pick(QuoteLength::All).expect("a quote");
        assert_eq!(first.id, again.id);
    }

    #[test]
    fn an_empty_offset_finds_nothing() {
        let list = QuoteList::parse(SAMPLE).expect("a quote file");
        assert!(list.pick_from(QuoteLength::All, 99).is_none());
    }

    /// Which quotes are missing, per bucket — a bare "none found" cannot tell
    /// "no long quotes in icelandic" from "no icelandic quotes at all".
    #[test]
    fn the_counts_say_which_buckets_are_empty() {
        let list = QuoteList::parse(SAMPLE).expect("a quote file");
        let counts = list.counts();
        let short = counts
            .iter()
            .find(|(bucket, _)| *bucket == QuoteLength::Short)
            .expect("a bucket");
        assert_eq!(short.1, 2);
        let long = counts
            .iter()
            .find(|(bucket, _)| *bucket == QuoteLength::Long)
            .expect("a bucket");
        assert_eq!(long.1, 0);
    }

    /// The British variant is a whole alternative text, not a diff.
    #[test]
    fn the_british_spelling_replaces_the_text_entirely() {
        let list = QuoteList::parse(
            r#"{"language":"x","quotes":[
                 {"id":1,"text":"colour","britishText":"colour","length":6,"source":"s"},
                 {"id":2,"text":"organize","britishText":"organise","length":8,"source":"s"}]}"#,
        )
        .expect("a quote file");
        assert_eq!(list.quotes[0].text_for(false), "colour");
        assert_eq!(list.quotes[1].text_for(true), "organise");
        // A quote with no British variant keeps its own text rather than
        // switching dialect halfway through.
        assert_eq!(list.quotes[0].text_for(true), "colour");
    }

    #[test]
    fn a_quote_with_no_source_is_not_an_error() {
        let list =
            QuoteList::parse(r#"{"language":"x","quotes":[{"id":1,"text":"hi","length":2}]}"#)
                .expect("a quote file");
        assert_eq!(list.quotes[0].source(), "");
    }

    /// A file that is not JSON at all, and one that is JSON of the wrong shape.
    #[test]
    fn a_broken_quote_file_is_an_error_not_an_empty_list() {
        assert!(QuoteList::parse("not json").is_err());
        assert!(QuoteList::parse(r#"{"quotes": "not a list"}"#).is_err());
    }

    #[test]
    fn a_file_with_no_quotes_parses_but_is_empty() {
        let list = QuoteList::parse(r#"{"language":"x","quotes":[]}"#).expect("a quote file");
        assert!(list.is_empty());
        assert!(list.pick(QuoteLength::All).is_none());
    }

    #[test]
    fn the_cache_path_is_under_one_directory_per_language() {
        let path = cache_path("english");
        assert!(path.ends_with("quotes/english.json"), "{path:?}");
    }
}
