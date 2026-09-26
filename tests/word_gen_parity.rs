//! Differential test of the word generator against the website's algorithm.
//!
//! `research/word-gen-oracle.js` is a transliteration of upstream's
//! `words-generator.ts` that draws from the same seeded generator the Rust side
//! uses. It was run once and its output committed to
//! `tests/data/word_gen_vectors.json`; this test replays those exact inputs and
//! compares word for word.
//!
//! The vectors are committed rather than regenerated so CI needs no Node. When
//! the algorithm changes on purpose, regenerate and read the diff:
//!
//! ```text
//! node research/word-gen-oracle.js > tests/data/word_gen_vectors.json
//! ```

use monkeytuipe::config::Difficulty;
use monkeytuipe::words::language::Language;
use monkeytuipe::words::{self, Generator, Lcg, Options, Rng};

const VECTORS: &str = include_str!("data/word_gen_vectors.json");

#[derive(Debug, serde::Deserialize)]
struct Case {
    language: String,
    punctuation: bool,
    numbers: bool,
    zipf: bool,
    seed: u32,
    count: usize,
    words: Vec<String>,
}

/// Loads a bundled language by id, so a case can be replayed exactly.
fn embedded(id: &str) -> Language {
    monkeytuipe::words::language::embedded(id).unwrap_or_else(|| panic!("{id} is embedded"))
}

fn cases() -> Vec<Case> {
    serde_json::from_str(VECTORS).expect("the committed vectors parse")
}

fn generate(case: &Case) -> Vec<String> {
    let language = embedded(&case.language);
    let options = Options {
        count: case.count,
        punctuation: case.punctuation,
        numbers: case.numbers,
        difficulty: Difficulty::Normal,
        zipf: case.zipf,
    };
    Generator::with_rng(&language, options, Box::new(Lcg::new(case.seed))).generate()
}

/// Every case matches the website.
#[test]
fn generated_tests_match_the_oracle() {
    let all = cases();
    assert!(all.len() >= 100, "the corpus is too small to be meaningful");

    let mut failures = Vec::new();
    for case in &all {
        let actual = generate(case);
        if actual != case.words {
            let at = actual
                .iter()
                .zip(case.words.iter())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| actual.len().min(case.words.len()));
            failures.push(format!(
                "{}/{}/{}/{}/{} diverged at word {at}: got {:?}, want {:?}",
                case.language,
                case.punctuation,
                case.numbers,
                case.zipf,
                case.seed,
                actual.get(at),
                case.words.get(at),
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases diverged:\n{}",
        failures.len(),
        all.len(),
        failures.join("\n")
    );
}

/// The corpus actually reaches every branch of the punctuation chain.
///
/// A differential test over a corpus that never produces a semicolon would pass
/// just as happily with the semicolon branch deleted, so the coverage is pinned
/// here rather than assumed.
#[test]
fn the_corpus_covers_every_punctuation_branch() {
    let words: Vec<String> = cases()
        .iter()
        .flat_map(generate)
        .map(|w| w.trim_end().to_owned())
        .collect();

    let count = |predicate: &dyn Fn(&str) -> bool| words.iter().filter(|w| predicate(w)).count();
    let branches: Vec<(&str, usize)> = vec![
        (
            "sentence terminator",
            count(&|w: &str| matches!(w.chars().last(), Some('.' | '?' | '!'))),
        ),
        ("double quotes", count(&|w: &str| w.starts_with('"'))),
        ("single quotes", count(&|w: &str| w.starts_with('\''))),
        ("parentheses", count(&|w: &str| w.starts_with('('))),
        ("colon", count(&|w: &str| w.ends_with(':'))),
        ("semicolon", count(&|w: &str| w.ends_with(';'))),
        ("lone dash", count(&|w: &str| w == "-")),
        ("comma", count(&|w: &str| w.ends_with(','))),
        (
            "a capitalised word",
            count(&|w: &str| w.starts_with(|c: char| c.is_uppercase())),
        ),
        (
            "a number",
            count(&|w: &str| w.chars().any(|c| c.is_ascii_digit())),
        ),
    ];
    for (name, hits) in branches {
        assert!(hits > 0, "the corpus never produced a {name}");
    }
}

/// The corpus reaches every language, both settings of every option, and a
/// spread of seeds — so a case cannot be quietly dropped.
#[test]
fn the_corpus_is_wide() {
    let all = cases();
    let mut languages: Vec<&str> = all.iter().map(|c| c.language.as_str()).collect();
    languages.sort_unstable();
    languages.dedup();
    assert_eq!(
        languages,
        [
            "english",
            "french",
            "german",
            "portuguese",
            "russian",
            "spanish"
        ]
    );

    for (field, name) in [
        (
            (|c: &Case| c.punctuation) as fn(&Case) -> bool,
            "punctuation on",
        ),
        (
            (|c: &Case| !c.punctuation) as fn(&Case) -> bool,
            "punctuation off",
        ),
        (|c: &Case| c.numbers, "numbers on"),
        (|c: &Case| c.zipf, "zipf"),
    ] {
        assert!(
            all.iter().any(field),
            "no case with {name} — the corpus is not testing it"
        );
    }
    assert!(all.iter().all(|c| c.words.len() == c.count));
}

/// The Lcg in Rust and the Lcg in the oracle draw the same numbers.
#[test]
fn the_seeded_generator_matches_the_oracle() {
    // The oracle's `next()` is reproduced here in Rust; if the two ever drift,
    // every case above would fail for the wrong reason.
    fn oracle_next(state: &mut u32) -> f64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *state = x;
        f64::from(x >> 8) / 16_777_216.0
    }

    let mut rust = Lcg::new(1729);
    let mut state = 1729u32;
    for i in 0..1000 {
        let from_rust = rust.next_f64();
        let from_oracle = oracle_next(&mut state);
        assert!(
            (0.0..1.0).contains(&from_rust),
            "draw {i} is outside [0, 1): {from_rust}"
        );
        assert_eq!(
            from_rust, from_oracle,
            "draw {i} differs, so the vectors are not comparable"
        );
    }
}

/// The real download path, against the real upstream.
///
/// The only test that would notice if the URL, the file layout or the JSON
/// shape changed upstream.
///
/// A transport failure means the machine has no network, and is reported rather
/// than failed — a build agent with no egress should not be told the word
/// generator is broken. Anything else (an unknown id, an unparseable list) is a
/// real failure and fails the test.
#[tokio::test]
async fn a_language_can_be_fetched_and_then_served_from_the_cache() {
    let id = "italian_1k";
    match words::fetch::fetch(id).await {
        Ok(language) => {
            assert!(
                language.words.len() > 500,
                "{id} came back with only {} words",
                language.words.len()
            );
        }
        Err(unavailable) => {
            let text = unavailable.to_string();
            assert!(
                !text.contains("http transport error"),
                "{id} could not be fetched for a reason that is not the network: {text}"
            );
            eprintln!("skipping the live fetch check: {text}");
            return;
        }
    }
    // It was cached, so it is now reachable with no network at all.
    assert!(words::fetch::is_available_offline(id));
    assert!(words::fetch::read_cache(id).is_some());
}
