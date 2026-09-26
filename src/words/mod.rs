//! Word generation.
//!
//! A port of the website's `frontend/src/ts/test/words-generator.ts` (plus
//! `wordset.ts` and the `zipfyRandomArrayIndex` helper it uses), covering the
//! parts that apply to plain language tests: Zipf or uniform sampling, the
//! de-duplication rules, punctuation, and numbers. Funboxes, quotes, custom
//! text and zen are not here.
//!
//! Randomness is injected through [`Rng`], because a generator whose output
//! cannot be pinned cannot be tested. The [`Lcg`] in this module is the same
//! algorithm `research/word-gen-oracle.js` uses, so the two can be compared
//! word for word — see `tests/word_gen_parity.rs`.

pub mod fetch;
pub mod language;
pub mod quotes;
pub mod variants;

use unicode_width::UnicodeWidthChar;

use crate::config::Difficulty;

pub use language::Language;

/// Source of randomness, so a test can pin it.
///
/// Deliberately just one method: the website draws from `Math.random` at fixed
/// points in the punctuation chain, and a generator with more entropy sources
/// would be one more thing to keep in step.
pub trait Rng {
    /// A value in `[0, 1)`, as `Math.random` produces.
    fn next_f64(&mut self) -> f64;
}

impl Rng for rand::rngs::StdRng {
    fn next_f64(&mut self) -> f64 {
        rand::Rng::random(self)
    }
}

/// The generator's default source of randomness.
///
/// [`rand::rng`] is thread-local and not `Send`, so it cannot sit behind the
/// boxed [`Rng`] the generator holds. `StdRng` is the `Send` equivalent.
pub fn default_rng() -> Box<dyn Rng + Send> {
    use rand::SeedableRng;
    Box::new(rand::rngs::StdRng::from_os_rng())
}

/// A seeded `xorshift32`, so a run can be reproduced exactly.
///
/// The same generator is implemented in `research/word-gen-oracle.js`; the
/// constants and the order of the draws are the contract between the two.
#[derive(Debug, Clone)]
pub struct Lcg {
    state: u32,
}

impl Lcg {
    /// Seeds the generator. Zero is remapped, since xorshift cannot leave it.
    pub fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x9e37_79b9 } else { seed },
        }
    }
}

impl Rng for Lcg {
    fn next_f64(&mut self) -> f64 {
        // xorshift32, then the usual 53-bit float assembly.
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        f64::from(x >> 8) / (1u32 << 24) as f64
    }
}

/// The punctuation characters the generator will not emit when punctuation is
/// off. Upstream writes this as a regex character class.
const BARE_PUNCTUATION: &str = "-=_+[]{};'\u{5c}:\"|,./<>?";

/// Whether the character after this one starts a new sentence.
fn starts_sentence(last: char) -> bool {
    matches!(last, '?' | '!' | '.' | '\u{61f}')
}

/// The last character of a word, by UTF-16 code unit as the website reads it.
fn last_char(word: &str) -> Option<char> {
    word.chars().next_back()
}

/// `capitalizeFirstLetterOfEachWord`: uppercase the first character of every
/// space-separated piece.
fn capitalize_each_word(text: &str) -> String {
    text.split(' ')
        .map(|piece| {
            let mut chars = piece.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `getNumbers(len)`: a run of digits whose first digit is not zero.
fn random_number(max_len: usize, rng: &mut dyn Rng) -> String {
    let len = 1 + random_index(max_len, rng);
    (0..len)
        .map(|i| {
            let range = if i == 0 { 9 } else { 10 };
            char::from_digit(
                random_index(range, rng) as u32 + if i == 0 { 1 } else { 0 },
                10,
            )
            .unwrap_or('0')
        })
        .collect()
}

/// `randomIntFromRange(0, n)`: a uniform index into an `n`-element array.
fn random_index(len: usize, rng: &mut dyn Rng) -> usize {
    if len == 0 {
        return 0;
    }
    let draw = rng.next_f64();
    ((draw * len as f64).floor() as usize).min(len - 1)
}

/// `zipfyRandomArrayIndex`: an index biased towards the start of the array,
/// which holds the most frequent words when the list is frequency-ordered.
///
/// The website approximates the harmonic number `H_N` as `ln(N + 0.5) + γ` and
/// inverts the CDF analytically. The constants and the shape of the
/// approximation are load-bearing: changing them changes which words a test
/// contains, and therefore every result derived from it.
pub fn zipf_index(len: usize, rng: &mut dyn Rng) -> usize {
    if len == 0 {
        return 0;
    }
    const GAMMA: f64 = 0.577_215_664_901_532_9; // Euler–Mascheroni
    let harmonic = ((len as f64) + 0.5).ln() + GAMMA;
    let r = rng.next_f64();
    let index = (r * harmonic - GAMMA).exp() - 0.5;
    (index.floor().max(0.0) as usize).min(len - 1)
}

/// Everything the generator needs beyond the word list itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// How many words to produce.
    pub count: usize,
    pub punctuation: bool,
    pub numbers: bool,
    pub difficulty: Difficulty,
    /// Bias sampling towards the frequent end of a frequency-ordered list.
    pub zipf: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            count: 100,
            punctuation: true,
            numbers: false,
            difficulty: Difficulty::Normal,
            zipf: false,
        }
    }
}

impl Options {
    /// [`Self::default`] for a timed test of `seconds`.
    ///
    /// The website keeps a rolling batch of 100 words and regenerates it when
    /// it runs dry. Here the batch is sized from the duration instead, so a
    /// long test cannot run out mid-word: six words per second is 360 wpm,
    /// which is far past anything a human reaches and leaves a wide margin.
    pub fn for_duration(seconds: u32) -> Self {
        Self {
            count: (seconds as usize).saturating_mul(6).max(100),
            ..Self::default()
        }
    }
}

/// Generates the words of one test.
///
/// Holds only what the website's module-level variables hold: the last two raw
/// words, for de-duplication, and any leftover piece of a multi-word entry.
pub struct Generator<'a> {
    language: &'a Language,
    options: Options,
    rng: Box<dyn Rng + Send>,
    /// The two most recent raw words, oldest first. Upstream keeps the previous
    /// two so a three-word run of the same word cannot slip through.
    history: Vec<String>,
    /// Leftover pieces of a raw entry that contained spaces.
    section: Vec<String>,
    /// The Spanish mark that has to close the sentence a `¿`/`¡` opened.
    ///
    /// Module-level state upstream, and genuinely cross-call: `¿Hola` opens an
    /// inverted question and the `?` that closes it lands on a *later* word.
    spanish_closer: Option<&'static str>,
}

impl<'a> Generator<'a> {
    pub fn new(language: &'a Language, options: Options) -> Self {
        Self::with_rng(language, options, default_rng())
    }

    /// A generator whose randomness comes from `rng`, for a reproducible run.
    pub fn with_rng(language: &'a Language, options: Options, rng: Box<dyn Rng + Send>) -> Self {
        Self {
            language,
            options,
            rng,
            history: Vec::with_capacity(2),
            section: Vec::new(),
            spanish_closer: None,
        }
    }

    /// The most recent raw word, for [`Self::with_rng`] callers that need to
    /// assert against the de-duplication rule.
    pub fn last_word(&self) -> Option<&str> {
        self.history.last().map(String::as_str)
    }

    /// Produces the whole test, each word carrying its trailing commit space.
    pub fn generate(&mut self) -> Vec<String> {
        (0..self.options.count)
            .map(|index| self.next_word(index))
            .collect()
    }

    /// One word, with the inter-word commit character attached.
    fn next_word(&mut self, index: usize) -> String {
        let raw = self.next_raw_word(index);
        self.remember(&raw);
        if raw.ends_with('\n') {
            raw
        } else {
            format!("{raw} ")
        }
    }

    fn remember(&mut self, raw: &str) {
        if self.history.len() == 2 {
            self.history.remove(0);
        }
        self.history.push(raw.to_owned());
    }

    /// The bare word, before its commit character.
    fn next_raw_word(&mut self, index: usize) -> String {
        let mut word = self.pick(index);
        if let Some(piece) = self.section.pop() {
            word = piece;
        } else {
            word = normalise_spaces(&word);
            let mut pieces: Vec<String> = word.split(' ').map(str::to_owned).collect();
            // A single-word entry leaves nothing queued; the popped value above
            // only exists when the previous entry had more than one piece.
            if pieces.len() > 1 {
                word = pieces.remove(0);
                pieces.reverse();
                self.section = pieces;
            }
        }

        if !self.options.punctuation
            && has_ascii_uppercase(&word)
            && !keeps_case(self.language.id())
        {
            word = word.to_lowercase();
        }

        if self.options.punctuation && !self.language.original_punctuation {
            let previous = self.history.last().cloned();
            word = punctuate(
                previous.as_deref(),
                &word,
                index,
                self.options.count,
                self.language,
                self.rng.as_mut(),
                &mut self.spanish_closer,
            );
        }

        if self.options.numbers && self.rng.next_f64() < 0.1 {
            word = random_number(4, self.rng.as_mut());
        }

        word
    }

    /// Draws a word, retrying while it would be a repeat or would carry
    /// characters the current options rule out.
    ///
    /// The 100-attempt cap mirrors the website's "infinite loop emergency stop
    /// button": with a list of one word and no punctuation, every draw is
    /// rejected, and the cap is what stops the test from hanging.
    fn pick(&mut self, _index: usize) -> String {
        let mut attempt = 0;
        loop {
            let word = self.sample();
            let first = normalise_spaces(&word)
                .split(' ')
                .next()
                .unwrap_or_default()
                .to_lowercase();
            if attempt >= 100 || !self.is_rejected(&first, &word) {
                return word;
            }
            attempt += 1;
        }
    }

    fn is_rejected(&self, first: &str, word: &str) -> bool {
        // Compared on the normalised form, because the previous word may have
        // been capitalised (`The`) and the new one drawn in lowercase (`the`).
        if self
            .history
            .iter()
            .any(|seen| comparison_key(seen) == comparison_key(first))
        {
            return true;
        }
        if !self.options.punctuation {
            // A bare "I" reads as a typo once it is lowercased with everything
            // else, and the special characters in the list are punctuation the
            // test is not asking for.
            if word == "I" {
                return true;
            }
            if !self.language.id().starts_with("code")
                && word.chars().any(|c| BARE_PUNCTUATION.contains(c))
            {
                return true;
            }
        }
        if !self.options.numbers && word.chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
        false
    }

    fn sample(&mut self) -> String {
        let len = self.language.words.len();
        if len == 0 {
            return String::new();
        }
        let index = if self.options.zipf && self.language.ordered_by_frequency {
            zipf_index(len, self.rng.as_mut())
        } else {
            random_index(len, self.rng.as_mut())
        };
        self.language.words[index].clone()
    }
}

/// The website's `previousWord.replace(/[.?!":\-,]/g, "").toLowerCase()`.
///
/// A word's identity for de-duplication: case and trailing punctuation do not
/// make it a different word.
fn comparison_key(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, '.' | '?' | '!' | '"' | ':' | '-' | ','))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Collapses runs of spaces and trims the ends, as the website's two
/// `replace` calls do.
///
/// Spaces only, deliberately: a newline or tab inside a word is part of the
/// target, and `split_whitespace` would silently delete it.
fn normalise_spaces(word: &str) -> String {
    word.split(' ')
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_ascii_uppercase(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_uppercase())
}

/// Languages whose capitalisation is meaningful enough to survive a
/// punctuation-free test.
fn keeps_case(language_id: &str) -> bool {
    language_id.starts_with("german")
        || language_id.starts_with("swiss_german")
        || language_id.starts_with("code")
        || language_id.starts_with("klingon")
}

/// The display width of a word, for the renderer.
pub fn display_width(word: &str) -> usize {
    word.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// The punctuation pass, port of `punctuateWord`.
///
/// The website's chain is a sequence of `else if`s, each of which draws from
/// `Math.random` and may or may not be reached. The order of the draws is
/// therefore part of the behaviour, and is reproduced here draw for draw.
fn punctuate(
    previous: Option<&str>,
    word: &str,
    index: usize,
    limit: usize,
    language: &Language,
    rng: &mut dyn Rng,
    // Spanish opens an inverted-mark sentence and closes it on whichever later
    // word gets a terminator, so the state has to outlive this call.
    spanish_closer: &mut Option<&'static str>,
) -> String {
    let base = language_base(language);
    let last = previous.and_then(last_char);
    let mut word = word.to_owned();

    let is_not = |c: char| last != Some(c);
    let is_final = index + 1 == limit;
    // The word before the last is excluded from the random terminator, so the
    // last word can always end the sentence cleanly.
    let is_penultimate = index + 2 == limit;

    if base != "code" && base != "georgian" && (index == 0 || last.is_some_and(starts_sentence)) {
        word = capitalize_each_word(&word);
        if base == "turkish" {
            word = word.replace('I', "\u{130}");
        }
        if base == "spanish" {
            let draw = rng.next_f64();
            // The closer is the plain mark: `¿` goes on the opening word and
            // the `?` that answers it goes on whichever later word ends the
            // sentence.
            if draw > 0.9 {
                word = format!("\u{bf}{word}");
                *spanish_closer = Some("?");
            } else if draw > 0.8 {
                word = format!("\u{a1}{word}");
                *spanish_closer = Some("!");
            }
        }
    } else if (rng.next_f64() < 0.1 && is_not('.') && is_not(',') && !is_penultimate) || is_final {
        if let Some(closer) = *spanish_closer {
            word.push_str(closer);
            *spanish_closer = None;
        } else {
            let draw = rng.next_f64();
            if draw <= 0.8 {
                word.push(match base {
                    "kurdish" => '.',
                    "nepali" | "bangla" | "hindi" => '\u{964}',
                    "japanese" | "chinese" => '\u{3002}',
                    _ => '.',
                });
            } else if draw < 0.9 {
                match base {
                    // French types a question mark on its own.
                    "french" => {
                        word = "?".to_owned();
                    }
                    "arabic" | "persian" | "urdu" | "kurdish" => word.push('\u{61f}'),
                    "greek" => word.push(';'),
                    "japanese" | "chinese" => word.push('\u{ff1f}'),
                    _ => word.push('?'),
                }
            } else {
                match base {
                    "french" => {
                        word = "!".to_owned();
                    }
                    "japanese" | "chinese" => word.push('\u{ff01}'),
                    _ => word.push('!'),
                }
            }
        }
    } else if rng.next_f64() < 0.01 && is_not(',') && is_not('.') && base != "russian" {
        word = format!("\"{word}\"");
    } else if rng.next_f64() < 0.011
        && is_not(',')
        && is_not('.')
        && base != "russian"
        && base != "ukrainian"
        && base != "slovak"
    {
        word = format!("'{word}'");
    } else if rng.next_f64() < 0.012 && is_not(',') && is_not('.') {
        if base == "japanese" || base == "chinese" {
            word = format!("\u{ff08}{word}\u{ff09}");
        } else {
            word = format!("({word})");
        }
    } else if rng.next_f64() < 0.013
        && is_not(',')
        && is_not('.')
        && is_not(';')
        && is_not('\u{61b}')
        && is_not(':')
        && is_not('\u{ff1b}')
        && is_not('\u{ff1a}')
    {
        if base == "french" {
            word = ":".to_owned();
        } else {
            word.push(':');
        }
    } else if rng.next_f64() < 0.014 && is_not(',') && is_not('.') && previous != Some("-") {
        word = "-".to_owned();
    } else if rng.next_f64() < 0.015
        && is_not(',')
        && is_not('.')
        && is_not(';')
        && is_not('\u{61b}')
        && is_not('\u{ff1b}')
        && is_not('\u{ff1a}')
    {
        match base {
            "french" => {
                word = ":".to_owned();
            }
            // Greek types an ano teleia, which is indistinguishable from a full
            // stop and has no key of its own.
            "greek" => {
                word = ".".to_owned();
            }
            "arabic" | "kurdish" => word.push('\u{61b}'),
            _ => word.push(';'),
        }
    } else if rng.next_f64() < 0.2 && is_not(',') {
        word.push(match base {
            "arabic" | "urdu" | "persian" | "kurdish" => '\u{60c}',
            "japanese" => '\u{3001}',
            "chinese" => '\u{ff0c}',
            _ => ',',
        });
    }

    // A tab or newline is part of the word, not decoration, so it moves to the
    // end where the typist will meet it last.
    if word.contains('\t') || word.contains('\n') {
        let newline = word.contains('\n');
        let cleaned: String = word.chars().filter(|c| *c != '\t' && *c != '\n').collect();
        word = if newline {
            format!("{cleaned}\n")
        } else {
            format!("{cleaned}\t")
        };
    }

    word
}

/// The language id without its size suffix: `english_1k` is `english`.
fn language_base(language: &Language) -> &str {
    let id = language.id();
    id.split('_').next().unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn language(words: &[&str]) -> Language {
        Language {
            name: "test".to_owned(),
            words: words.iter().map(|w| (*w).to_owned()).collect(),
            ordered_by_frequency: true,
            original_punctuation: false,
            joining_script: false,
            right_to_left: false,
        }
    }

    fn generate(words: &[&str], options: Options, seed: u32) -> Vec<String> {
        let language = language(words);
        Generator::with_rng(&language, options, Box::new(Lcg::new(seed))).generate()
    }

    /// A rough, word-based list. Only the shapes matter.
    const LIST: &[&str] = &[
        "the", "be", "of", "and", "a", "to", "in", "he", "have", "it", "that", "for", "they", "i",
        "with", "as", "not", "on", "she", "at", "by", "this", "we", "you", "do", "but", "from",
        "or", "which", "one", "would", "all", "there", "their", "what", "about",
    ];

    #[test]
    fn the_lcg_is_reproducible() {
        let a: Vec<f64> = (0..5)
            .scan(Lcg::new(42), |rng, _| Some(rng.next_f64()))
            .collect();
        let b: Vec<f64> = (0..5)
            .scan(Lcg::new(42), |rng, _| Some(rng.next_f64()))
            .collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| (0.0..1.0).contains(v)));
    }

    #[test]
    fn every_word_carries_its_commit_space() {
        let words = generate(LIST, Options::default(), 7);
        assert_eq!(words.len(), 100);
        for word in &words {
            assert!(word.ends_with(' '), "{word:?} has no commit character");
            assert!(!word.contains("  "), "{word:?} has a doubled space");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_test() {
        let options = Options::default();
        assert_eq!(generate(LIST, options, 11), generate(LIST, options, 11));
    }

    #[test]
    fn a_different_seed_gives_a_different_test() {
        let options = Options::default();
        assert_ne!(generate(LIST, options, 11), generate(LIST, options, 12));
    }

    #[test]
    fn a_word_never_repeats_back_to_back() {
        for seed in 0..24 {
            let words = generate(LIST, Options::default(), seed);
            let bare: Vec<&str> = words.iter().map(|w| w.trim_end()).collect();
            for pair in bare.windows(2) {
                let first = pair[0].to_lowercase();
                let second = pair[1].to_lowercase();
                assert_ne!(first, second, "seed {seed} repeated a word: {pair:?}");
            }
        }
    }

    #[test]
    fn the_first_word_is_capitalised() {
        for seed in 0..8 {
            let words = generate(LIST, Options::default(), seed);
            assert!(
                words[0].starts_with(char::is_uppercase),
                "seed {seed} started with {:?}",
                words[0]
            );
        }
    }

    #[test]
    fn the_last_word_ends_the_sentence() {
        for seed in 0..16 {
            let words = generate(LIST, Options::default(), seed);
            let last = &words[words.len() - 1];
            assert!(
                last.ends_with(". ") || last.ends_with("? ") || last.ends_with("! "),
                "seed {seed} ended with {last:?}"
            );
        }
    }

    #[test]
    fn punctuation_off_keeps_words_bare() {
        let options = Options {
            punctuation: false,
            ..Options::default()
        };
        for seed in 0..16 {
            for word in generate(LIST, options, seed) {
                let bare = word.trim_end();
                assert!(!bare.contains(','), "seed {seed} produced {bare:?}");
                assert!(!bare.ends_with('.'), "seed {seed} produced {bare:?}");
                assert!(
                    !bare.chars().any(|c| BARE_PUNCTUATION.contains(c)),
                    "seed {seed} produced {bare:?}"
                );
            }
        }
    }

    #[test]
    fn numbers_off_means_no_digits() {
        for word in generate(LIST, Options::default(), 3) {
            assert!(
                !word.chars().any(|c| c.is_ascii_digit()),
                "digits appeared with numbers off: {word:?}"
            );
        }
    }

    #[test]
    fn numbers_on_occasionally_replaces_a_word_with_digits() {
        let options = Options {
            numbers: true,
            ..Options::default()
        };
        let words = generate(LIST, options, 5);
        let with_digits = words
            .iter()
            .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
            .count();
        assert!(
            (5..=25).contains(&with_digits),
            "about one word in ten should be a number, got {with_digits}"
        );
        for word in words
            .iter()
            .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
        {
            let digits: String = word
                .trim_end()
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            assert!(
                !digits.starts_with('0'),
                "a number cannot lead with zero: {word:?}"
            );
            assert!(
                digits.len() <= 4,
                "a number is at most four digits: {word:?}"
            );
        }
    }

    /// A list of one word rejects every draw, so this only terminates because
    /// of the 100-attempt cap. The website has the same escape hatch.
    ///
    /// The words are not all `hello`: the punctuation pass may replace a word
    /// outright with `-` or `:`.
    #[test]
    fn a_single_word_list_does_not_hang() {
        let words = generate(&["hello"], Options::default(), 1);
        assert_eq!(words.len(), 100);
        for word in &words {
            assert!(!word.trim_end().is_empty(), "a word may not be empty");
        }
        assert!(
            words.iter().any(|w| w.to_lowercase().contains("hello")),
            "the list's own word should still show up"
        );
    }

    #[test]
    fn zipf_favours_the_frequent_end() {
        let list: Vec<String> = (0..200).map(|i| format!("w{i}")).collect();
        let refs: Vec<&str> = list.iter().map(String::as_str).collect();
        let mut language = language(&refs);
        language.ordered_by_frequency = true;

        let sample = |zipf: bool| {
            let options = Options {
                zipf,
                punctuation: false,
                count: 4000,
                ..Options::default()
            };
            Generator::with_rng(&language, options, Box::new(Lcg::new(99))).generate()
        };
        let head = |words: Vec<String>| {
            words
                .iter()
                .filter(|w| {
                    w.trim_end().starts_with('w') && {
                        let n: usize = w.trim_start_matches('w').trim_end().parse().unwrap_or(9999);
                        n < 20
                    }
                })
                .count()
        };
        let uniform = head(sample(false));
        let zipped = head(sample(true));
        assert!(
            zipped > uniform * 3,
            "zipf should concentrate on the head: {zipped} vs {uniform}"
        );
    }

    #[test]
    fn zipf_is_ignored_when_the_list_is_not_frequency_ordered() {
        let list: Vec<String> = (0..200).map(|i| format!("w{i}")).collect();
        let refs: Vec<&str> = list.iter().map(String::as_str).collect();
        let mut language = language(&refs);
        language.ordered_by_frequency = false;
        let options = Options {
            zipf: true,
            ..Options::default()
        };
        let words = Generator::with_rng(&language, options, Box::new(Lcg::new(4))).generate();
        let head = words
            .iter()
            .filter(|w| {
                w.trim_end()
                    .trim_start_matches('w')
                    .trim_end()
                    .parse::<usize>()
                    .is_ok_and(|n| n < 20)
            })
            .count();
        assert!(
            head < 400,
            "an unordered list must not be biased, got {head} of the head in 100"
        );
    }

    #[test]
    fn zipf_index_stays_inside_the_array() {
        let mut rng = Lcg::new(7);
        for len in 1..300 {
            for _ in 0..20 {
                assert!(zipf_index(len, &mut rng) < len);
            }
        }
    }

    #[test]
    fn a_zero_length_list_is_safe() {
        assert_eq!(zipf_index(0, &mut Lcg::new(1)), 0);
        assert_eq!(random_index(0, &mut Lcg::new(1)), 0);
        // `getNumbers(0)` upstream is `randomIntFromRange(1, 0)`, which is 1:
        // an empty range degenerates to its minimum rather than to nothing.
        assert_eq!(random_number(0, &mut Lcg::new(1)).len(), 1);
    }

    #[test]
    fn random_numbers_never_lead_with_zero() {
        let mut rng = Lcg::new(21);
        for _ in 0..500 {
            let n = random_number(4, &mut rng);
            assert!(!n.is_empty());
            assert!(n.len() <= 4);
            assert!(!n.starts_with('0'), "{n}");
            assert!(n.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn a_timed_test_gets_a_batch_nobody_can_type_through() {
        assert_eq!(Options::for_duration(30).count, 180);
        assert_eq!(Options::for_duration(1).count, 100, "there is a floor");
        assert_eq!(Options::for_duration(122).count, 732);
    }

    #[test]
    fn capitalising_handles_cyrillic_and_spaces() {
        assert_eq!(capitalize_each_word("word"), "Word");
        assert_eq!(capitalize_each_word("и"), "И");
        assert_eq!(capitalize_each_word("two words"), "Two Words");
        assert_eq!(capitalize_each_word(""), "");
    }

    /// A newline is a commit character, so it ends the word instead of being
    /// followed by a space.
    ///
    /// The move to the end happens inside the punctuation pass, exactly as
    /// upstream does it, so it only applies when punctuation is on. With
    /// punctuation off the raw entry is emitted untouched — the newline is not
    /// the last character, so a space is appended behind it.
    #[test]
    fn a_newline_commits_the_word() {
        let mut language = language(LIST);
        language.words = vec!["a\nb".to_owned()];

        let with_punctuation = Generator::with_rng(
            &language,
            Options {
                count: 1,
                punctuation: true,
                ..Options::default()
            },
            Box::new(Lcg::new(1)),
        )
        .generate();
        // The first word of a test is capitalised on the way through the
        // punctuation pass, so the moved newline lands after `Ab`.
        assert_eq!(with_punctuation, vec!["Ab\n".to_owned()]);

        let without = Generator::with_rng(
            &language,
            Options {
                count: 1,
                punctuation: false,
                ..Options::default()
            },
            Box::new(Lcg::new(1)),
        )
        .generate();
        assert_eq!(without, vec!["a\nb ".to_owned()]);
    }

    #[test]
    fn spaces_inside_an_entry_are_collapsed_not_deleted() {
        // `split_whitespace` would fold a newline away; the website only ever
        // collapses runs of spaces.
        assert_eq!(normalise_spaces("a  b"), "a b");
        assert_eq!(normalise_spaces(" a b "), "a b");
        assert_eq!(normalise_spaces("a\nb"), "a\nb");
        assert_eq!(normalise_spaces("a\tb"), "a\tb");
    }
}
