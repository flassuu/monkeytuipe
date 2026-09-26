//! Rebuilding what a typist had entered, from the event log.
//!
//! The log records *that* a character was entered and whether it was right; it
//! does not record what the input element contained. Anything that needs the
//! text — the chart's cumulative wpm, the per-key breakdown, a replay of a
//! finished test — has to put it back together, and the rules for doing that are
//! fiddly enough to be worth having in one place.
//!
//! Two things make it more than a stack of pushes.
//!
//! First, the character that ends a word is stored apart from the word's input,
//! so the two views differ at rest and have to be kept in step. Here the commit
//! character *is* part of the reconstructed input, and taking it back off is what
//! a backspace does — which is exactly what the website's `inputValue` does, and
//! why its replay works.
//!
//! Second, the word being typed may be credited or not, and that is decided by
//! which word is the active one, which changes as the test goes along. A word's
//! contribution therefore has to be recomputed when the active word moves, not
//! accumulated once.

use std::collections::BTreeMap;

use super::chars::count_chars;
use super::event_log::Event;

/// A test rebuilt from its log, up to whatever point the log has been applied.
#[derive(Debug, Clone)]
pub struct Replay<'a> {
    /// What has been entered in each word.
    inputs: BTreeMap<usize, String>,
    /// The word the typist is in, which is the one allowed to be half-typed.
    active: usize,
    targets: &'a [String],
    /// Whether a half-typed word counts. True only on a clock.
    credit_partial: bool,
}

impl<'a> Replay<'a> {
    /// Starts a replay against `targets`, each of which must include its own
    /// commit character.
    ///
    /// `is_timed` decides both whether the word being typed counts towards wpm
    /// and, in [`crate::stats::chart`], whether the chart needs a fractional
    /// trailing bucket. It comes from [`crate::engine::Mode::is_timed`].
    pub fn new(targets: &'a [String], is_timed: bool) -> Self {
        Self {
            inputs: BTreeMap::new(),
            active: 0,
            targets,
            credit_partial: is_timed,
        }
    }

    /// Advances the replay by one event.
    pub fn apply(&mut self, event: Event) {
        let word = event.word();
        match event {
            Event::Insert { ch, .. } => {
                self.inputs.entry(word).or_default().push(ch);
                // A space or newline ends the word, so the caret is already on
                // the next one.
                self.active = if ch == ' ' || ch == '\n' {
                    word + 1
                } else {
                    word
                };
            }
            Event::Delete { .. } => {
                if let Some(input) = self.inputs.get_mut(&word) {
                    input.pop();
                }
                // A backspace always leaves the caret in the word it edited,
                // which is how a deletion that walked back out of a committed
                // word makes that word count as unfinished again.
                self.active = word;
            }
            Event::Skip { .. } => {
                self.inputs.insert(word, String::new());
                self.active = word + 1;
            }
        }
    }

    /// The word the typist is in.
    pub fn active(&self) -> usize {
        self.active
    }

    /// What has been entered in `word`.
    pub fn input(&self, word: usize) -> &str {
        self.inputs.get(&word).map_or("", String::as_str)
    }

    /// The target for `word`, commit character included.
    ///
    /// A word with no target in the list cannot be scored against anything, so
    /// what was typed stands in for the target — which is what the website does
    /// when a word is missing from `targetWords`.
    pub fn target(&self, word: usize) -> &str {
        self.targets
            .get(word)
            .map_or_else(|| self.input(word), String::as_str)
    }

    /// Every word that has been touched, in order, as `(index, input)`.
    pub fn words(&self) -> impl Iterator<Item = (usize, &str)> {
        self.inputs.iter().map(|(i, input)| (*i, input.as_str()))
    }

    /// `correct_word` for one word, as it stands right now.
    pub fn word_counts(&self, word: usize) -> u32 {
        let credit = self.credit_partial && word == self.active;
        count_chars(self.input(word), self.target(word), credit).correct_word
    }

    /// The cumulative `correct_word` the wpm series divides by time.
    ///
    /// Words *after* the active one are excluded, which is what the website's
    /// `break` on the active word does: once you backspace across a commit, the
    /// word you left behind is no longer part of the score.
    pub fn correct_word(&self) -> u32 {
        self.inputs
            .iter()
            .filter(|(word, input)| **word <= self.active && !input.is_empty())
            .map(|(word, _)| self.word_counts(*word))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| format!("{t} ")).collect()
    }

    fn insert(word: usize, index: usize, correct: bool, ch: char) -> Event {
        Event::Insert {
            word,
            index,
            correct,
            ch,
        }
    }

    fn typed(log: &mut EventLog, events: &[Event]) {
        for (i, event) in events.iter().enumerate() {
            log.push(i as f64, *event);
        }
    }

    use super::super::event_log::EventLog;

    #[test]
    fn a_replay_rebuilds_what_was_typed() {
        let list = targets(&["the", "quick"]);
        let mut replay = Replay::new(&list, true);
        replay.apply(insert(0, 0, true, 't'));
        replay.apply(insert(0, 1, true, 'h'));
        replay.apply(insert(0, 2, true, 'e'));
        replay.apply(insert(0, 3, true, ' '));
        replay.apply(insert(1, 0, true, 'q'));

        assert_eq!(replay.input(0), "the ");
        assert_eq!(replay.input(1), "q");
        assert_eq!(replay.active(), 1);
    }

    #[test]
    fn a_commit_counts_only_while_its_word_is_the_active_one() {
        let list = targets(&["the"]);
        let mut timed = Replay::new(&list, true);
        timed.apply(insert(0, 0, true, 't'));
        timed.apply(insert(0, 1, true, 'h'));
        assert_eq!(timed.correct_word(), 2, "a prefix counts on a clock");

        let mut untimed = Replay::new(&list, false);
        untimed.apply(insert(0, 0, true, 't'));
        untimed.apply(insert(0, 1, true, 'h'));
        assert_eq!(untimed.correct_word(), 0, "and does not otherwise");
    }

    #[test]
    fn committing_a_prefix_takes_its_credit_back() {
        let list = targets(&["three"]);
        let mut replay = Replay::new(&list, true);
        replay.apply(insert(0, 0, true, 't'));
        replay.apply(insert(0, 1, true, 'h'));
        assert_eq!(replay.correct_word(), 2);
        replay.apply(insert(0, 2, false, ' '));
        assert_eq!(replay.correct_word(), 0, "'th ' is not a prefix");
    }

    #[test]
    fn a_backspace_out_of_a_committed_word_makes_it_unfinished_again() {
        let list = targets(&["cat", "dog"]);
        let mut replay = Replay::new(&list, true);
        replay.apply(insert(0, 0, true, 'c'));
        replay.apply(insert(0, 1, true, 'a'));
        replay.apply(insert(0, 2, true, 't'));
        replay.apply(insert(0, 3, true, ' '));
        replay.apply(insert(1, 0, true, 'd'));
        replay.apply(insert(1, 1, true, 'o'));
        replay.apply(insert(1, 2, true, 'g'));
        replay.apply(insert(1, 3, true, ' '));
        assert_eq!(replay.correct_word(), 8);

        replay.apply(Event::Delete { word: 1, index: 3 });
        assert_eq!(replay.active(), 1);
        assert_eq!(
            replay.correct_word(),
            7,
            "'dog' is a prefix again, and 'cat ' is finished"
        );
    }

    #[test]
    fn words_left_behind_the_active_one_leave_the_score() {
        let list = targets(&["cat", "dog"]);
        let mut replay = Replay::new(&list, true);
        for (index, ch) in "cat ".chars().enumerate() {
            replay.apply(insert(0, index, true, ch));
        }
        replay.apply(insert(1, 0, true, 'd'));
        assert_eq!(replay.active(), 1);
        assert_eq!(replay.correct_word(), 5, "'cat ' and a prefix of 'dog '");

        // Backspace the 'd' away and then step back into the finished word.
        replay.apply(Event::Delete { word: 1, index: 0 });
        replay.apply(Event::Delete { word: 0, index: 3 });
        assert_eq!(replay.active(), 0);
        assert_eq!(
            replay.correct_word(),
            3,
            "word 1 is past the active one, so only 'cat' counts"
        );
    }

    #[test]
    fn a_skipped_word_is_empty_and_contributes_nothing() {
        let list = targets(&["a", "b"]);
        let mut replay = Replay::new(&list, true);
        replay.apply(Event::Skip { word: 0 });
        assert_eq!(replay.input(0), "");
        assert_eq!(replay.active(), 1);
        assert_eq!(replay.correct_word(), 0);
    }

    #[test]
    fn a_word_with_no_target_is_scored_against_what_was_typed() {
        let mut replay = Replay::new(&[], true);
        replay.apply(insert(0, 0, true, 'a'));
        assert_eq!(replay.target(0), "a");
        assert_eq!(replay.correct_word(), 1);
    }

    #[test]
    fn words_are_reported_in_order() {
        let list = targets(&["a", "b", "c"]);
        let mut replay = Replay::new(&list, true);
        replay.apply(insert(2, 0, true, 'c'));
        replay.apply(insert(0, 0, true, 'a'));
        let seen: Vec<(usize, &str)> = replay.words().collect();
        assert_eq!(seen, [(0, "a"), (2, "c")]);
    }

    #[test]
    fn a_log_applied_end_to_end_replays_to_the_same_input() {
        let list = targets(&["the", "quick"]);
        let mut log = EventLog::new();
        typed(
            &mut log,
            &[
                insert(0, 0, true, 't'),
                insert(0, 1, true, 'h'),
                insert(0, 2, false, 'x'),
                Event::Delete { word: 0, index: 2 },
                insert(0, 2, true, 'e'),
                insert(0, 3, true, ' '),
                insert(1, 0, true, 'q'),
            ],
        );
        let mut replay = Replay::new(&list, true);
        for event in log.events() {
            replay.apply(event.event);
        }
        assert_eq!(replay.input(0), "the ");
        assert_eq!(replay.input(1), "q");
        assert_eq!(replay.correct_word(), 5);
    }
}
