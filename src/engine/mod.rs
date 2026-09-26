//! The typing engine: a test in progress, and the counters that go with it.
//!
//! Deliberately free of ratatui, crossterm and the clock. The only way time
//! enters is [`Test::finish`], so every scoring rule here can be tested by
//! feeding it characters.
//!
//! Two counters are maintained, and they are not the same thing:
//!
//! - [`crate::stats::Inputs`] counts **keystrokes** — one per character pressed,
//!   correct or not. This is the accuracy the website shows.
//! - [`CharCounts`] is computed **after the fact** from each word's final
//!   input, so backspaces are already applied. This is what feeds `charStats`.
//!
//! Mixing them up is the classic way to end up with an accuracy that disagrees
//! with the website, so they are kept in separate fields and separate tests.

pub mod word;

pub use word::{Word, WordState};

use crate::stats::{CharCounts, Inputs};

/// How long the test lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A fixed number of seconds. The last, unfinished word is credited.
    Time,
    /// A fixed number of words. The last word must be finished properly.
    Words,
    /// A fixed passage; the final word ends with a newline rather than a space.
    Quote,
    /// Endless practice with no target text and nothing to score.
    Zen,
    /// A passage the user typed in themselves, run to the end.
    Custom,
}

impl Mode {
    /// Whether an unfinished trailing word counts.
    ///
    /// A timed test is still "going" when the clock runs out, so a half-typed
    /// word is scored as a partial one. A word-count test has no such grace.
    pub fn credits_partial_words(self) -> bool {
        matches!(self, Mode::Time)
    }

    /// Whether the test is measured against a clock.
    ///
    /// A word-count test of zero words is "as many as you can in no time at
    /// all", which the website treats as timed, so the phrase carries over.
    /// Zen is timed too, or it would have no clock to run on.
    pub fn is_timed(self, mode2: u32) -> bool {
        matches!(self, Mode::Time | Mode::Zen) || (self == Mode::Words && mode2 == 0)
    }

    /// Whether the test has a target to be scored against.
    ///
    /// Zen has none, which is the whole point of it: there is nothing to be right
    /// or wrong about, so the result screen says so rather than showing zeros
    /// that look like a failed test.
    pub fn is_scored(self) -> bool {
        self != Mode::Zen
    }

    /// Whether the word list runs out.
    ///
    /// Zen does not: the engine appends a fresh empty word as each one is
    /// committed, so a zen test can run until the clock stops it.
    pub fn is_endless(self) -> bool {
        self == Mode::Zen
    }
}

/// What a call to [`Test::input`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keystroke {
    /// Entered as a character in the active word.
    Typed { correct: bool },
    /// Ended the active word, because a space or newline was pressed.
    Committed { correct: bool },
    /// The word was jumped over, which is not a character and is not scored.
    Skipped,
    /// The test was already over, so nothing happened.
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Test {
    words: Vec<Word>,
    active: usize,
    mode: Mode,
    /// Seconds for [`Mode::Time`], word count for [`Mode::Words`].
    mode2: u32,
    inputs: Inputs,
    chars: CharCounts,
    started: bool,
    finished: bool,
}

impl Test {
    /// Builds a test over already-prepared word texts.
    ///
    /// The last word of a quote commits with a newline, which is what the
    /// website does: in quote mode the passage ends with a line break rather
    /// than another space.
    pub fn new(texts: Vec<String>, mode: Mode, mode2: u32) -> Self {
        // A passage ends with a line break rather than another space, which is
        // what the website does for both quotes and custom text. Zen is neither:
        // it never ends.
        let last_is_passage_end = matches!(mode, Mode::Quote | Mode::Custom);
        let count = texts.len();
        let words = texts
            .into_iter()
            .enumerate()
            .map(|(i, text)| {
                let commit = if last_is_passage_end && i + 1 == count {
                    '\n'
                } else {
                    ' '
                };
                Word::new(text, commit)
            })
            .collect();
        Self {
            words,
            active: 0,
            mode,
            mode2,
            inputs: Inputs::default(),
            chars: CharCounts::default(),
            started: false,
            finished: false,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn mode2(&self) -> u32 {
        self.mode2
    }

    pub fn words(&self) -> &[Word] {
        &self.words
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_word(&self) -> &Word {
        &self.words[self.active.min(self.words.len().saturating_sub(1))]
    }

    /// Per-keystroke accuracy counters.
    pub fn inputs(&self) -> Inputs {
        self.inputs
    }

    /// Accuracy as a percentage, live.
    pub fn accuracy(&self) -> f64 {
        self.inputs.accuracy()
    }

    /// Post-hoc character counts, over every word scored so far.
    ///
    /// The word being typed is not included until it is committed or the test
    /// ends, matching how the website only scores what is in the event log.
    pub fn chars(&self) -> CharCounts {
        self.chars
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Whether the typist has touched anything yet.
    ///
    /// The clock does not run until this is true, so time spent reading the
    /// words is not charged to the score.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Whether the test is under way, i.e. started but not yet over.
    pub fn is_running(&self) -> bool {
        self.started && !self.finished
    }

    /// How far through the test the typist is, from 0 to 1.
    pub fn progress(&self) -> f64 {
        let total = self.words.len();
        if total == 0 {
            return 0.0;
        }
        self.active as f64 / total as f64
    }

    /// Feeds one character in.
    ///
    /// Any space or newline commits the active word, whatever the target's own
    /// commit character is — the website treats both as separators — but
    /// whether the character was *right* is judged against the target, so
    /// pressing space on a quote's last line is an error rather than a commit.
    pub fn input(&mut self, c: char) -> Keystroke {
        if self.finished {
            return Keystroke::Ignored;
        }
        self.started = true;
        let correct = self.active_word().accepts(c);
        let is_commit = c == ' ' || c == '\n';

        // Accuracy is per keystroke, so this happens for every character,
        // including the commit and including the ones about to be backspaced.
        self.inputs.record(correct);

        if is_commit {
            // Commit first: the commit character has to be part of the input
            // before the word can be scored against its target.
            self.active_word_mut().commit_with(c, correct);
            self.score_active_word(false, false);
            self.advance();
            Keystroke::Committed { correct }
        } else {
            self.active_word_mut().push(c);
            Keystroke::Typed { correct }
        }
    }

    /// Jumps over the active word, contributing nothing to the score.
    pub fn skip(&mut self) -> Keystroke {
        if self.finished {
            return Keystroke::Ignored;
        }
        self.started = true;
        self.active_word_mut().skip();
        self.advance();
        Keystroke::Skipped
    }

    /// Removes the last character, walking back into the previous word when the
    /// active one is empty.
    ///
    /// Scoring is *not* unwound: the counters are rebuilt from the words when
    /// the test ends, so a backspace that changes the totals shows up there
    /// rather than as a decrement here.
    pub fn backspace(&mut self) -> bool {
        if self.finished {
            return false;
        }
        if self.active_word_mut().pop() {
            return true;
        }
        if self.active == 0 {
            return false;
        }
        self.active -= 1;
        self.active_word_mut().pop()
    }

    /// Ends the test, scoring whatever is left of the active word.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.score_active_word(self.mode.credits_partial_words(), true);
        self.finished = true;
    }

    /// Whether the test should stop now, for a reason other than the clock.
    pub fn is_complete(&self) -> bool {
        !self.mode.is_endless() && self.active >= self.words.len()
    }

    fn active_word_mut(&mut self) -> &mut Word {
        let index = self.active.min(self.words.len().saturating_sub(1));
        &mut self.words[index]
    }

    /// Adds the active word's characters to the running total.
    ///
    /// The two flags are independent. `credit_partial` decides whether a prefix
    /// counts, which is false for a word the typist committed on purpose.
    /// `is_final_word` decides whether a mistyped commit character is trimmed
    /// off, which only applies to the word the test stopped on.
    fn score_active_word(&mut self, credit_partial: bool, is_final_word: bool) {
        if self.active < self.words.len() {
            self.chars += self.words[self.active].count(credit_partial, is_final_word);
        }
    }

    fn advance(&mut self) {
        if self.active < self.words.len() {
            self.active += 1;
        }
        if self.mode.is_endless() {
            // Zen keeps going: a fresh blank word appears as the last one is
            // committed. Appending rather than reusing the last word is what
            // keeps its characters from being committed twice, and it is why a
            // zen test's word count grows as you type.
            if self.active >= self.words.len() {
                self.words.push(Word::new(String::new(), ' '));
            }
            return;
        }
        if self.is_complete() {
            self.finish();
        }
    }
}

/// Keeps `count_chars` reachable from this module for tests that build a word
/// by hand and compare against the engine's own bookkeeping.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::count_chars;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn type_text(test: &mut Test, text: &str) -> Vec<Keystroke> {
        text.chars().map(|c| test.input(c)).collect()
    }

    #[test]
    fn a_new_test_starts_on_the_first_word() {
        let test = Test::new(words(&["a", "b"]), Mode::Words, 2);
        assert_eq!(test.active_index(), 0);
        assert_eq!(test.active_word().text(), "a");
        assert!(!test.is_finished());
    }

    #[test]
    fn typing_advances_the_caret_within_a_word_but_not_across() {
        let mut test = Test::new(words(&["a", "b"]), Mode::Words, 2);
        assert_eq!(test.input('a'), Keystroke::Typed { correct: true });
        assert_eq!(test.active_index(), 0, "the word only ends on a space");
        // The caret has moved to the target's word-ending space, so 'b' is wrong.
        assert_eq!(test.input('b'), Keystroke::Typed { correct: false });
        assert_eq!(test.active_index(), 0);
    }

    #[test]
    fn a_space_commits_the_word_and_moves_on() {
        let mut test = Test::new(words(&["a", "b"]), Mode::Words, 2);
        test.input('a');
        assert_eq!(test.input(' '), Keystroke::Committed { correct: true });
        assert_eq!(test.active_index(), 1);
        assert_eq!(test.active_word().text(), "b");
    }

    #[test]
    fn the_last_word_of_a_quote_commits_with_a_newline() {
        let test = Test::new(words(&["hello", "there"]), Mode::Quote, 0);
        assert_eq!(test.words()[0].commit(), ' ');
        assert_eq!(
            test.words()[1].commit(),
            '\n',
            "a passage ends with a line break"
        );
    }

    #[test]
    fn pressing_space_on_a_quote_ending_commits_but_is_wrong() {
        let mut test = Test::new(words(&["there"]), Mode::Quote, 0);
        type_text(&mut test, "there");
        assert_eq!(
            test.input(' '),
            Keystroke::Committed { correct: false },
            "the target wanted a newline"
        );
    }

    #[test]
    fn a_newline_commits_an_ordinary_word_too() {
        let mut test = Test::new(words(&["a", "b"]), Mode::Words, 2);
        test.input('a');
        assert_eq!(test.input('\n'), Keystroke::Committed { correct: false });
        assert_eq!(test.active_index(), 1);
    }

    #[test]
    fn accuracy_counts_every_keystroke_including_wrong_ones() {
        let mut test = Test::new(words(&["cat"]), Mode::Words, 1);
        type_text(&mut test, "cot");
        test.input(' ');

        assert_eq!(test.inputs().correct, 3, "c, o and the space");
        assert_eq!(test.inputs().incorrect, 1, "x is wrong");
        assert_eq!(test.inputs().total(), 4);
        assert!((test.accuracy() - 75.0).abs() < 1e-9);
    }

    #[test]
    fn accuracy_with_no_keystrokes_is_zero_not_nan() {
        let test = Test::new(words(&["a"]), Mode::Words, 1);
        assert_eq!(test.accuracy(), 0.0);
        assert!(!test.accuracy().is_nan());
    }

    #[test]
    fn accuracy_includes_the_commit_character() {
        // The website penalises a wrong space like any other wrong character.
        let mut test = Test::new(words(&["ab"]), Mode::Words, 1);
        test.input('a');
        test.input('!');
        test.input(' ');
        assert_eq!(test.inputs().correct, 2, "a and the space");
        assert_eq!(test.inputs().incorrect, 1);
        assert_eq!(test.inputs().total(), 3);
    }

    #[test]
    fn a_space_typed_past_the_end_of_the_target_is_wrong() {
        // The caret is already past the word-ending space, so this space is an
        // overflow character rather than a commit.
        let mut test = Test::new(words(&["a"]), Mode::Words, 1);
        test.input('a');
        test.input('!');
        test.input(' ');
        assert_eq!(test.inputs().correct, 1, "only the a");
        assert_eq!(test.inputs().incorrect, 2, "the ! and the overflow space");
    }

    #[test]
    fn char_counts_are_only_updated_when_a_word_is_committed() {
        let mut test = Test::new(words(&["word", "next"]), Mode::Words, 2);
        type_text(&mut test, "wo");
        assert_eq!(
            test.chars(),
            CharCounts::default(),
            "not scored while typing"
        );

        test.input('r');
        test.input('d');
        assert_eq!(test.chars(), CharCounts::default());

        test.input(' ');
        assert_eq!(test.chars().correct_word, 5, "four letters and the space");
    }

    #[test]
    fn the_counts_of_a_finished_test_match_counting_its_words_directly() {
        let mut test = Test::new(words(&["cat", "dog", "sun"]), Mode::Words, 3);
        for typed in ["cat", "dox", "sun"] {
            type_text(&mut test, typed);
            test.input(' ');
        }

        let mut expected = CharCounts::default();
        expected += count_chars("cat ", "cat ", false);
        expected += count_chars("dox ", "dog ", false);
        expected += count_chars("sun ", "sun ", false);
        assert_eq!(test.chars(), expected);
        assert_eq!(test.chars().correct_word, 8, "cat and sun, but not dox");
        assert_eq!(test.chars().incorrect, 1);
        assert_eq!(test.chars().extra, 1, "the space of the wrong word");
    }

    #[test]
    fn a_timed_test_credits_the_unfinished_last_word() {
        let mut test = Test::new(words(&["word"]), Mode::Time, 10);
        type_text(&mut test, "wo");
        test.finish();

        assert_eq!(test.chars().correct_word, 2, "the prefix counts");
        assert_eq!(test.chars().missed, 0, "and nothing is missed");
    }

    #[test]
    fn a_word_count_test_does_not_credit_the_unfinished_last_word() {
        let mut test = Test::new(words(&["word"]), Mode::Words, 1);
        type_text(&mut test, "wo");
        test.finish();

        assert_eq!(test.chars().correct_word, 0);
        assert_eq!(
            test.chars().missed,
            3,
            "r, d and the word-ending space, which is part of the target"
        );
    }

    #[test]
    fn backspace_walks_into_the_previous_word() {
        let mut test = Test::new(words(&["one", "two"]), Mode::Words, 2);
        type_text(&mut test, "one ");
        assert_eq!(test.active_index(), 1);

        assert!(test.backspace(), "an empty word steps backwards");
        assert_eq!(test.active_index(), 0);
        assert_eq!(test.active_word().text(), "one");
        assert_eq!(
            test.active_word().state(),
            WordState::Typing,
            "and becomes editable"
        );
    }

    #[test]
    fn backspace_at_the_very_first_word_does_nothing() {
        let mut test = Test::new(words(&["one"]), Mode::Words, 1);
        assert!(!test.backspace());
        assert_eq!(test.active_index(), 0);
    }

    #[test]
    fn input_after_the_test_is_over_is_ignored() {
        let mut test = Test::new(words(&["a"]), Mode::Words, 1);
        test.input('a');
        test.input(' ');
        assert!(test.is_finished());
        assert_eq!(test.input('b'), Keystroke::Ignored);
        assert_eq!(test.active_index(), 1, "the caret did not run away");
    }

    #[test]
    fn skipping_a_word_lands_on_the_next_one_and_is_not_scored() {
        let mut test = Test::new(words(&["one", "two"]), Mode::Words, 2);
        assert_eq!(test.skip(), Keystroke::Skipped);
        assert_eq!(test.active_index(), 1);
        assert_eq!(test.words()[0].state(), WordState::Skipped);
        assert_eq!(test.inputs().total(), 0, "a skip is not a character");
        assert_eq!(test.chars(), CharCounts::default());
    }

    #[test]
    fn finishing_the_last_word_finishes_the_test() {
        let mut test = Test::new(words(&["a", "b"]), Mode::Words, 2);
        test.input('a');
        test.input(' ');
        assert!(!test.is_finished());
        test.input('b');
        test.input(' ');
        assert!(test.is_finished());
        assert_eq!(test.chars().correct_word, 4);
    }

    #[test]
    fn finishing_twice_scores_the_active_word_only_once() {
        let mut test = Test::new(words(&["word"]), Mode::Time, 10);
        type_text(&mut test, "wo");
        test.finish();
        let after_first = test.chars();
        test.finish();
        assert_eq!(test.chars(), after_first);
    }

    #[test]
    fn progress_tracks_words_typed() {
        let mut test = Test::new(words(&["a", "b", "c", "d"]), Mode::Words, 4);
        assert_eq!(test.progress(), 0.0);
        for expected in 1..4 {
            test.input(' ');
            assert_eq!(test.progress(), expected as f64 / 4.0);
        }
    }

    #[test]
    fn progress_of_an_empty_test_is_zero() {
        let test = Test::new(Vec::new(), Mode::Words, 0);
        assert_eq!(test.progress(), 0.0);
    }

    #[test]
    fn a_backspaced_character_still_counts_towards_accuracy() {
        // The website judges each keystroke as it happens and never refunds it,
        // so accuracy and the final character counts are allowed to disagree.
        let mut test = Test::new(words(&["cat"]), Mode::Words, 1);
        test.input('c');
        test.input('x');
        test.backspace();
        test.input('a');
        test.input('t');
        test.input(' ');

        assert_eq!(test.inputs().correct, 4, "c, a, t and the space");
        assert_eq!(test.inputs().incorrect, 1, "the x, despite being undone");
        assert_eq!(
            test.chars().correct_word,
            4,
            "the word itself ended up exact"
        );
    }
}
