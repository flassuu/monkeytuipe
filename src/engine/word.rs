//! One target word, and what the typist has entered for it.
//!
//! The split between [`Word::input`] and [`Word::commit`] is the whole point of
//! this file. The commit character is never stored as input: pressing space
//! *ends* the word rather than appending to it. It is re-attached only when the
//! word is scored, because that is what the website's event log holds.

use crate::stats::{count_chars, CharCounts};

/// How a word ended, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordState {
    /// Not reached yet.
    #[default]
    Untouched,
    /// Being typed right now, so its characters are still being decided.
    Typing,
    /// Every character matched, the commit character included.
    Correct,
    /// The typist moved on with at least one character wrong.
    Incorrect,
    /// Jumped over with the skip key, contributing nothing.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The target text, without its commit character.
    text: String,
    /// The character that ends this word: usually a space, a newline for the
    /// last word of a quote.
    commit: char,
    /// What the typist has entered, excluding the commit character.
    input: String,
    state: WordState,
    /// The commit character the typist actually pressed, once they have.
    commit_typed: Option<char>,
    /// Whether that commit character matched [`Word::commit`].
    commit_correct: bool,
}

impl Word {
    /// Builds a word from its target text and the character that ends it.
    pub fn new(text: impl Into<String>, commit: char) -> Self {
        Self {
            text: text.into(),
            commit,
            input: String::new(),
            state: WordState::default(),
            commit_typed: None,
            commit_correct: false,
        }
    }

    /// The target text, without the commit character.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The character that ends this word.
    pub fn commit(&self) -> char {
        self.commit
    }

    /// What the typist has entered so far, without the commit character.
    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn state(&self) -> WordState {
        self.state
    }

    pub fn is_touched(&self) -> bool {
        self.state != WordState::Untouched
    }

    /// The number of characters committed so far, in UTF-16 code units, which
    /// is how the website indexes a string.
    pub fn input_len_utf16(&self) -> usize {
        self.input.encode_utf16().count()
    }

    /// The target character at a UTF-16 index, or `None` past the end.
    ///
    /// `None` is also returned for either half of a surrogate pair: a lone
    /// surrogate cannot be produced by a keystroke, so nothing can match there.
    /// The website's `data === targetWord[i]` cannot match there either, which
    /// is why a target containing an astral character is untypeable in the same
    /// way in both.
    pub fn char_at(&self, index: usize) -> Option<char> {
        let mut seen = 0usize;
        for c in self.text.chars() {
            let width = c.len_utf16();
            if index >= seen && index < seen + width {
                return (width == 1).then_some(c);
            }
            seen += width;
        }
        // The commit character occupies the unit right after the text.
        (index == seen).then_some(self.commit)
    }

    /// Whether `c` is the right character at the current caret position.
    ///
    /// Past the end of the target is an error, not a freebie, matching
    /// `isCharCorrect`.
    pub fn accepts(&self, c: char) -> bool {
        self.char_at(self.input_len_utf16()) == Some(c)
    }

    /// Appends a non-commit character.
    pub fn push(&mut self, c: char) {
        self.input.push(c);
        if self.state == WordState::Untouched {
            self.state = WordState::Typing;
        }
    }

    /// Records that the typist committed the word with `c`.
    pub fn commit_with(&mut self, c: char, correct: bool) {
        self.commit_typed = Some(c);
        self.commit_correct = correct;
        self.state = if self.is_exact() {
            WordState::Correct
        } else {
            WordState::Incorrect
        };
    }

    /// Removes the last thing entered, which is the commit character when the
    /// word has one. Returns whether anything was removed.
    ///
    /// Removing the commit character puts a finished word back into the middle
    /// of being typed, which is what lets backspace walk backwards through a
    /// test and fix a mistake. The order matters: the commit character was the
    /// final keystroke, so it is the first thing to go.
    pub fn pop(&mut self) -> bool {
        if self.commit_typed.take().is_some() {
            self.commit_correct = false;
            self.state = self.in_progress_state();
            return true;
        }
        if self.input.pop().is_some() {
            self.state = self.in_progress_state();
            return true;
        }
        false
    }

    fn in_progress_state(&self) -> WordState {
        if self.input.is_empty() {
            WordState::Untouched
        } else {
            WordState::Typing
        }
    }

    /// Marks the word as jumped over.
    pub fn skip(&mut self) {
        self.state = WordState::Skipped;
    }

    /// Puts the word back to untouched, for a backspace that walks into it.
    pub fn reset(&mut self) {
        self.input.clear();
        self.commit_typed = None;
        self.commit_correct = false;
        self.state = WordState::Untouched;
    }

    /// Whether input and target match character for character, commit included.
    fn is_exact(&self) -> bool {
        let typed = self.commit_typed.unwrap_or(self.commit);
        if typed != self.commit {
            return false;
        }
        self.input == self.text
    }

    /// The input string to score this word with.
    ///
    /// `is_final_word` mirrors the website's treatment of a mistyped commit
    /// character on the last word: it is trimmed off entirely, so a wrong space
    /// counts as nothing rather than as an error.
    pub fn input_for_counting(&self, is_final_word: bool) -> String {
        let mut out = String::with_capacity(self.input.len() + 1);
        out.push_str(&self.input);
        if let Some(c) = self.commit_typed {
            if !is_final_word || self.commit_correct {
                out.push(c);
            }
        }
        out
    }

    /// Scores this word.
    ///
    /// `credit_partial` is false for any word the typist deliberately
    /// committed; see [`crate::stats::count_chars`]. `is_final_word` enables the
    /// trailing-trim rule for a word the test ended on.
    pub fn count(&self, credit_partial: bool, is_final_word: bool) -> CharCounts {
        let input = self.input_for_counting(is_final_word);
        let mut target = String::with_capacity(self.text.len() + 1);
        target.push_str(&self.text);
        target.push(self.commit);
        count_chars(&input, &target, credit_partial)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word() -> Word {
        Word::new("word", ' ')
    }

    #[test]
    fn a_fresh_word_is_untouched_and_empty() {
        let w = word();
        assert_eq!(w.state(), WordState::Untouched);
        assert_eq!(w.input(), "");
        assert!(!w.is_touched());
        assert_eq!(w.text(), "word");
        assert_eq!(w.commit(), ' ');
    }

    #[test]
    fn the_commit_character_is_reachable_as_a_target() {
        let w = word();
        for (i, expected) in "word ".chars().enumerate() {
            assert_eq!(w.char_at(i), Some(expected), "index {i}");
        }
        assert_eq!(w.char_at(5), None, "past the end");
    }

    #[test]
    fn accepts_tracks_the_caret() {
        let mut w = word();
        assert!(w.accepts('w'));
        w.push('w');
        assert!(w.accepts('o'));
        w.push('x');
        assert!(!w.accepts('o'), "the caret did not move past the mistake");
        assert!(w.accepts('r'), "mistakes are not barriers");
    }

    #[test]
    fn a_character_past_the_target_is_rejected() {
        let mut w = word();
        for c in "word ".chars() {
            w.push(c);
        }
        assert_eq!(w.input_len_utf16(), 5);
        assert!(!w.accepts('!'), "there is nothing left to match");
    }

    #[test]
    fn commit_marks_the_word_correct_only_when_everything_matched() {
        let mut good = word();
        for c in "word".chars() {
            good.push(c);
        }
        good.commit_with(' ', true);
        assert_eq!(good.state(), WordState::Correct);

        let mut bad = word();
        bad.push('w');
        bad.push('x');
        bad.commit_with(' ', true);
        assert_eq!(bad.state(), WordState::Incorrect);
    }

    #[test]
    fn a_wrong_commit_character_does_not_make_the_word_correct() {
        // The last word of a quote commits with a newline; a space is wrong even
        // when every letter matched.
        let mut w = Word::new("done", '\n');
        for c in "done".chars() {
            w.push(c);
        }
        w.commit_with(' ', false);
        assert_eq!(w.state(), WordState::Incorrect);
    }

    #[test]
    fn a_wrong_commit_character_is_trimmed_from_the_scored_input() {
        let mut w = Word::new("done", '\n');
        for c in "done".chars() {
            w.push(c);
        }
        w.commit_with(' ', false);

        // Committed words keep the character...
        assert_eq!(w.input_for_counting(false), "done ");
        // ...but the final word drops it, as the website does.
        assert_eq!(w.input_for_counting(true), "done");
    }

    #[test]
    fn backspace_undoes_a_commit() {
        let mut w = word();
        for c in "word".chars() {
            w.push(c);
        }
        w.commit_with(' ', true);
        assert_eq!(w.state(), WordState::Correct);

        assert!(w.pop(), "the first backspace takes the commit character");
        assert_eq!(w.state(), WordState::Typing, "the word is editable again");
        assert_eq!(w.input(), "word", "the letters are still there");

        assert!(w.pop());
        assert_eq!(w.input(), "wor");
    }

    #[test]
    fn backspace_walks_back_through_a_word_and_empties_it() {
        let mut w = word();
        for c in "wor".chars() {
            w.push(c);
        }
        assert_eq!(w.state(), WordState::Typing);
        assert!(w.pop());
        assert!(w.pop());
        assert_eq!(w.state(), WordState::Typing);
        assert!(w.pop());
        assert_eq!(w.state(), WordState::Untouched);
        assert!(!w.pop(), "nothing left to remove");
    }

    #[test]
    fn backspace_on_an_empty_word_reports_that_it_did_nothing() {
        assert!(!word().pop());
    }

    #[test]
    fn skipping_marks_the_word_and_keeps_it_out_of_the_way() {
        let mut w = word();
        w.skip();
        assert_eq!(w.state(), WordState::Skipped);
        assert!(w.is_touched());
    }

    #[test]
    fn counting_an_unfinished_word_credits_nothing_by_default() {
        let mut w = word();
        for c in "wo".chars() {
            w.push(c);
        }
        let counts = w.count(false, true);
        assert_eq!(counts.missed, 3, "r, d and the word-ending space");
        assert_eq!(counts.correct_word, 0);
    }

    #[test]
    fn counting_an_unfinished_word_credits_a_prefix_when_asked() {
        let mut w = word();
        for c in "wo".chars() {
            w.push(c);
        }
        let counts = w.count(true, true);
        assert_eq!(counts.correct_word, 2);
        assert_eq!(
            counts.missed, 0,
            "a partial word is not penalised for being short"
        );
    }

    #[test]
    fn counting_a_committed_word_credits_the_commit_character() {
        let mut w = word();
        for c in "word".chars() {
            w.push(c);
        }
        w.commit_with(' ', true);
        let counts = w.count(false, false);
        assert_eq!(counts.all_correct, 5, "four letters and the space");
        assert_eq!(counts.correct_word, 5);
    }

    #[test]
    fn a_target_beyond_the_bmp_cannot_be_matched_partway() {
        // The emoji occupies two UTF-16 units. Neither unit can be matched by a
        // keystroke, so a word containing one is untypeable — exactly as on the
        // website, where `data === targetWord[i]` is comparing against a lone
        // surrogate. The commit character still follows it.
        let w = Word::new("a\u{1F600}", ' ');
        assert_eq!(w.char_at(0), Some('a'));
        assert_eq!(w.char_at(1), None, "the high surrogate");
        assert_eq!(w.char_at(2), None, "the low surrogate");
        assert_eq!(w.char_at(3), Some(' '), "the commit still follows");
    }
}
