//! A monkeytype word list, and the ones bundled into the binary.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The base (200-word) list of every common language, compiled in.
///
/// Upstream ships ~450 lists totalling ~140 MB, so embedding all of them is not
/// an option and embedding none of them would make a cold start need the network.
/// These six cover the languages most tests are actually run in and cost 17 kB
/// together; anything else is fetched once and cached — see [`super::fetch`].
///
/// Refresh with `tools/fetch-languages.sh`.
pub const EMBEDDED: &[(&str, &str)] = &[
    ("english", include_str!("data/english.json")),
    ("russian", include_str!("data/russian.json")),
    ("german", include_str!("data/german.json")),
    ("spanish", include_str!("data/spanish.json")),
    ("french", include_str!("data/french.json")),
    ("portuguese", include_str!("data/portuguese.json")),
];

/// A word list plus the handful of properties the generator and the renderer act
/// on. Every field beyond `name` and `words` is optional upstream, so all of
/// them default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Language {
    /// The upstream id, e.g. `english_1k`. Also the config value and the cache
    /// file name.
    #[serde(default)]
    pub name: String,
    pub words: Vec<String>,

    /// The list is already sorted most-common-first, so Zipf sampling by index
    /// is meaningful. Lists without this are in arbitrary order, and biasing
    /// towards their start would just make word 0 the most frequent.
    #[serde(default, rename = "orderedByFrequency")]
    pub ordered_by_frequency: bool,

    /// The list already carries its own punctuation, so the generator must not
    /// add any.
    #[serde(default, rename = "originalPunctuation")]
    pub original_punctuation: bool,

    /// Letters are joined rather than spaced (Arabic, and friends), which the
    /// renderer needs for the caret.
    #[serde(default, rename = "joiningScript")]
    pub joining_script: bool,

    /// Text runs right to left.
    #[serde(default, rename = "rightToLeft")]
    pub right_to_left: bool,
}

impl Language {
    /// The upstream id. Falls back to the list's own `name`, and to `unknown`
    /// for a list that has neither.
    pub fn id(&self) -> &str {
        if self.name.is_empty() {
            "unknown"
        } else {
            &self.name
        }
    }

    /// The human-readable name, which upstream keeps equal to the id in practice.
    pub fn label(&self) -> &str {
        self.id()
    }

    /// True when the list has nothing to generate from.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({} words)", self.id(), self.words.len())
    }
}

/// Looks a language up among the compiled-in ones.
pub fn embedded(id: &str) -> Option<Language> {
    EMBEDDED
        .iter()
        .find(|(name, _)| *name == id)
        .and_then(|(_, json)| parse(json))
}

/// Parses a list as downloaded from upstream.
pub fn parse(json: &str) -> Option<Language> {
    serde_json::from_str::<Language>(json)
        .ok()
        .filter(|language| !language.is_empty())
}

/// Every language the binary can produce without a network round trip.
pub fn embedded_ids() -> impl Iterator<Item = &'static str> {
    EMBEDDED.iter().map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_list_parses_and_has_words() {
        assert!(EMBEDDED.len() >= 5, "a handful of languages is the point");
        for (id, json) in EMBEDDED {
            let language = parse(json).unwrap_or_else(|| panic!("{id} is not valid JSON"));
            assert!(!language.is_empty(), "{id} has no words");
            assert!(
                !language.words.iter().any(String::is_empty),
                "{id} has an empty word"
            );
        }
    }

    #[test]
    fn the_embedded_languages_are_the_popular_ones() {
        let ids: Vec<&str> = embedded_ids().collect();
        for wanted in ["english", "russian", "german"] {
            assert!(ids.contains(&wanted), "{wanted} must work offline");
        }
    }

    #[test]
    fn the_english_list_is_frequency_ordered_and_russian_is_too() {
        assert!(embedded("english").expect("english").ordered_by_frequency);
        assert!(embedded("russian").expect("russian").ordered_by_frequency);
        // A list without the flag must not be treated as ordered, or Zipf
        // sampling would just favour whatever happens to be first.
        assert!(!embedded("german").expect("german").ordered_by_frequency);
    }

    #[test]
    fn an_unknown_id_is_not_embedded() {
        assert!(embedded("klingon").is_none());
    }

    #[test]
    fn a_list_without_the_optional_fields_still_parses() {
        let language = parse(r#"{"name":"x","words":["a","b"]}"#).expect("parses");
        assert_eq!(language.id(), "x");
        assert!(!language.ordered_by_frequency);
        assert!(!language.right_to_left);
        assert!(!language.joining_script);
        assert!(!language.original_punctuation);
    }

    #[test]
    fn a_nameless_list_falls_back_to_a_placeholder_id() {
        let language = parse(r#"{"words":["a"]}"#).expect("parses");
        assert_eq!(language.id(), "unknown");
        assert_eq!(language.to_string(), "unknown (1 words)");
    }

    #[test]
    fn an_empty_or_broken_list_is_rejected() {
        assert!(parse(r#"{"name":"x","words":[]}"#).is_none());
        assert!(parse("not json").is_none());
    }

    /// The embedded files are the ones upstream publishes, byte for byte.
    ///
    /// A silent edit to a bundled list would change results without changing
    /// the code, so the byte count is pinned here as a tripwire: re-run
    /// `tools/fetch-languages.sh` and update these numbers deliberately.
    #[test]
    fn the_embedded_files_are_the_expected_size() {
        let sizes: Vec<(&str, usize)> = EMBEDDED
            .iter()
            .map(|(id, json)| (*id, json.len()))
            .collect();
        assert_eq!(
            sizes,
            vec![
                ("english", 2540),
                ("russian", 3511),
                ("german", 2669),
                ("spanish", 2627),
                ("french", 2341),
                ("portuguese", 2815),
            ]
        );
    }
}
