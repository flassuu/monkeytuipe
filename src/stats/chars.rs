//! Per-word character counting, ported from
//! `frontend/src/ts/utils/strings.ts::countChars`.
//!
//! This is deliberately **post-hoc**: it is fed the final input string of a
//! word, after every backspace has already been applied. That is how monkeytype
//! counts, and it is why the numbers cannot be maintained incrementally per
//! keystroke — only per completed word.

use std::ops::AddAssign;

/// The five counters `countChars` produces.
///
/// Counters are accumulated with `+=` as each word is completed, so scoring a
/// finished test is a single pass over the words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharCounts {
    /// Characters that matched, ignoring word boundaries.
    pub all_correct: u32,
    /// Characters in words that count as correct — see [`count_chars`] on
    /// `credit_partial`.
    pub correct_word: u32,
    /// Characters that were typed but wrong.
    pub incorrect: u32,
    /// Characters past the end of the word, or a space typed in its place.
    pub extra: u32,
    /// Characters of the word that were never reached.
    pub missed: u32,
}

impl CharCounts {
    /// The denominator of `charTotal`: `allCorrect + incorrect + extra`.
    ///
    /// Note that `missed` is **not** included, even though it is reported
    /// separately in `charStats`.
    pub fn total(&self) -> u32 {
        self.all_correct + self.incorrect + self.extra
    }

    /// The `charStats` array in the order the API expects it.
    pub fn to_stats_array(&self) -> [u32; 4] {
        [self.correct_word, self.incorrect, self.extra, self.missed]
    }
}

impl AddAssign for CharCounts {
    fn add_assign(&mut self, rhs: Self) {
        self.all_correct += rhs.all_correct;
        self.correct_word += rhs.correct_word;
        self.incorrect += rhs.incorrect;
        self.extra += rhs.extra;
        self.missed += rhs.missed;
    }
}

/// Counts one word's characters.
///
/// `credit_partial` decides whether a word that is only a prefix of the target
/// still contributes `correct_word`, and whether unreached characters count as
/// `missed`. It is true for timed tests, for bailed-out tests, and for a
/// trailing partial word when `countPartialLastWord` is set — in a timed test
/// the last, unfinished word **is** credited.
///
/// Iteration is over UTF-16 code units, not `char`s, because the original
/// indexes JavaScript strings by code unit. For the ASCII word lists monkeytype
/// ships the two agree; for text outside the Basic Multilingual Plane they do
/// not, and a surrogate pair would be counted as two characters here exactly as
/// it is upstream.
pub fn count_chars(input: &str, target: &str, credit_partial: bool) -> CharCounts {
    let mut counts = CharCounts::default();

    // `inputWord === targetWord` and `targetWord.startsWith(inputWord)` operate
    // on whole strings, so they are answered from the `&str`s.
    let word_correct = input == target;
    let word_partially_correct = target.starts_with(input);

    let input_units: Vec<u16> = input.encode_utf16().collect();
    let target_units: Vec<u16> = target.encode_utf16().collect();

    for i in 0..input_units.len().max(target_units.len()) {
        // `undefined` past the end of a string, as in the original.
        let input_char = input_units.get(i).copied();
        let target_char = target_units.get(i).copied();

        if input_char == target_char {
            if target_char == Some(SPACE) && !word_correct {
                // A space in a word that is not fully correct is an extra.
                counts.extra += 1;
            } else {
                counts.all_correct += 1;
            }
            if word_correct || (credit_partial && word_partially_correct) {
                counts.correct_word += 1;
            }
        } else if input_char.is_none() {
            // Missed character.
            if !credit_partial {
                counts.missed += 1;
            }
        } else if target_char.is_none()
            || (target_char == Some(SPACE) && input_char != Some(SPACE) && !input.contains(' '))
        {
            // Past the end of the word, or typed in place of the word-ending space.
            counts.extra += 1;
        } else {
            counts.incorrect += 1;
        }
    }

    counts
}

const SPACE: u16 = b' ' as u16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_word_is_all_correct() {
        let counts = count_chars("word", "word", true);
        assert_eq!(
            counts,
            CharCounts {
                all_correct: 4,
                correct_word: 4,
                incorrect: 0,
                extra: 0,
                missed: 0,
            }
        );
    }

    #[test]
    fn the_word_ending_space_counts_as_correct_when_the_word_is_exact() {
        // The trailing space is part of the target, so an exact match is correct.
        let counts = count_chars("word ", "word ", true);
        assert_eq!(counts.all_correct, 5);
        assert_eq!(counts.correct_word, 5);
        assert_eq!(counts.extra, 0);
    }

    #[test]
    fn a_space_in_an_incorrect_word_is_extra() {
        // "wrld " vs "word " — the final space matches but the word is not
        // correct, so it is extra rather than correct.
        let counts = count_chars("wrld ", "word ", true);
        assert_eq!(counts.extra, 1);
        assert_eq!(counts.incorrect, 2);
        assert_eq!(counts.missed, 0);
    }

    #[test]
    fn unreached_characters_are_missed_only_without_credit() {
        let partial = count_chars("wo", "word", false);
        assert_eq!(partial.missed, 2);
        assert_eq!(
            partial.correct_word, 0,
            "not credited when credit_partial is false"
        );

        let credited = count_chars("wo", "word", true);
        assert_eq!(credited.missed, 0, "missed is suppressed by credit_partial");
        assert_eq!(credited.correct_word, 2, "a prefix is credited");
    }

    #[test]
    fn extra_characters_past_the_target_are_extra() {
        let counts = count_chars("words", "word", true);
        assert_eq!(counts.extra, 1);
        assert_eq!(counts.incorrect, 0);
    }

    #[test]
    fn typing_a_non_space_over_the_ending_space_is_extra() {
        // "wordx" against "word " — nothing contains a space, so the mismatch
        // at the space counts as extra, not incorrect.
        let counts = count_chars("wordx", "word ", true);
        assert_eq!(counts.extra, 1);
        assert_eq!(counts.incorrect, 0);
    }

    #[test]
    fn a_real_incorrect_character_is_incorrect() {
        let counts = count_chars("wird", "word", true);
        assert_eq!(counts.incorrect, 1);
        assert_eq!(counts.all_correct, 3);
    }

    #[test]
    fn an_empty_input_leaves_the_whole_word_missed() {
        assert_eq!(
            count_chars("", "word", false),
            CharCounts {
                missed: 4,
                ..Default::default()
            }
        );
    }

    #[test]
    fn total_excludes_missed() {
        let counts = count_chars("wo", "word", false);
        assert_eq!(counts.total(), 2, "allCorrect + incorrect + extra");
        assert_ne!(counts.total(), 4);
    }

    #[test]
    fn an_over_long_word_earns_no_correct_word_credit() {
        // "words" is not "word", and "word" is not a prefix of "words", so the
        // four matching characters count as all_correct but nothing is credited
        // to correct_word even with credit_partial.
        let counts = count_chars("words", "word", true);
        assert_eq!(counts.all_correct, 4);
        assert_eq!(counts.correct_word, 0, "a longer word is not a prefix");
        assert_eq!(counts.extra, 1);
    }

    #[test]
    fn stats_array_is_in_api_order() {
        // [correctWord, incorrect, extra, missed]
        let counts = count_chars("wird", "word", true);
        assert_eq!(counts.to_stats_array(), [0, 1, 0, 0]);
        let exact = count_chars("word", "word", true);
        assert_eq!(exact.to_stats_array(), [4, 0, 0, 0]);
    }

    #[test]
    fn counters_accumulate_across_words() {
        let mut total = CharCounts::default();
        total += count_chars("word", "word", true);
        total += count_chars("wird", "word", true);
        assert_eq!(total.all_correct, 7);
        assert_eq!(total.incorrect, 1);
    }

    #[test]
    fn utf16_indexing_matches_javascript_for_astral_characters() {
        // "a" + U+1F600 is one `char` but three UTF-16 code units, because the
        // emoji is a surrogate pair. Upstream indexes JavaScript strings by
        // code unit and therefore counts three characters; so must we.
        let counts = count_chars("a\u{1F600}", "a\u{1F600}", true);
        assert_eq!(counts.all_correct, 3, "counted in UTF-16 code units");
        assert_eq!(counts.correct_word, 3);
    }
}
