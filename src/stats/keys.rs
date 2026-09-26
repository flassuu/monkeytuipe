//! Keystrokes: how many, how fast, how regular, and which keys got hit.
//!
//! [`Inputs`] is the per-keystroke accuracy counter — the one the website's
//! `acc` comes from — and lives here because it is a count of keys pressed and
//! nothing else. The rest is ported from `getKeypressSpacing` and the letter
//! table in `frontend/src/ts/test/test-logic.ts`.
//!
//! ## What a terminal cannot see
//!
//! `keyDuration` and `keyOverlap` are in the API payload, and neither can be
//! computed here. Both are built from `keyup` events, and a terminal in its
//! ordinary mode does not report key releases at all — there is no sequence a
//! terminal sends for "I let go of E". crossterm can report them on Windows,
//! where the console API supplies them, and on Unix it cannot.
//!
//! Inventing a plausible duration would be worse than leaving it out: it would
//! look like a measurement and would be one. So [`KeyStats`] carries no
//! duration, and the payload sent to monkeytype leaves those two fields out
//! rather than filling them with a guess. The same goes for the *identity* of a
//! key: [`key_spacing`] is exact, because the gap between two keystrokes needs
//! no key names, but [`per_key`] groups by the character that came out, which on
//! a QWERTY layout is the key that was pressed and on any other layout is not.

/// Per-keystroke accuracy, counted exactly as the website counts it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Inputs {
    pub correct: u32,
    pub incorrect: u32,
}

impl Inputs {
    pub fn record(&mut self, correct: bool) {
        if correct {
            self.correct += 1;
        } else {
            self.incorrect += 1;
        }
    }

    /// Characters pressed, right and wrong.
    pub fn total(&self) -> u32 {
        self.correct + self.incorrect
    }

    /// Accuracy as a percentage. No keystrokes means `0`, not `NaN`.
    pub fn accuracy(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        f64::from(self.correct) / f64::from(total) * 100.0
    }
}

use super::event_log::EventLog;
use super::numbers::{consistency, round_to2};
use super::replay::Replay;

/// The gap in milliseconds between each pair of consecutive keypresses.
///
/// One entry per keypress after the first, so a test of *n* keystrokes has
/// *n - 1* gaps. A backspace and a skipped word are keypresses and are counted;
/// the series is about the hands, not about characters.
///
/// A keystroke recorded before the clock started is clamped to zero, which keeps
/// `start_to_first + sum(spacing) + last_to_end` equal to the test duration.
pub fn key_spacing(log: &EventLog) -> Vec<f64> {
    let mut spacing = Vec::new();
    let mut last: Option<f64> = None;
    for timed in log.events() {
        if let Some(previous) = last {
            spacing.push(timed.ms - previous);
        }
        last = Some(timed.ms.max(0.0));
    }
    spacing
}

/// The rhythm of the typing, as a consistency score.
///
/// Built from `key_spacing` with its **last** gap dropped, which is what the
/// website does: the final gap runs from the last keystroke to the end of the
/// test, which includes however long the typist sat looking at the result, and
/// that pause is not part of how they type.
///
/// [`consistency`] already returns `0` for a mean of `0`, so a test of one
/// keystroke — no gaps at all — scores `0` rather than dividing by nothing.
pub fn key_consistency(spacing: &[f64]) -> f64 {
    let scored = match spacing.len() {
        0 => &[],
        _ => &spacing[..spacing.len() - 1],
    };
    consistency(scored)
}

/// What one character on the keyboard was used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyUse {
    /// The character, as it came out of the keyboard.
    pub key: char,
    /// How many times it appears in the finished input.
    pub pressed: u32,
    /// How many of those were the right character in that position.
    ///
    /// `pressed - correct` is the number of times this key stood in for a
    /// different one, which is what the red cells in the site's letter table
    /// mean.
    pub correct: u32,
}

impl KeyUse {
    /// Times pressed wrongly.
    pub fn wrong(&self) -> u32 {
        self.pressed - self.correct
    }
}

/// The letter table: every character that appeared, most used first.
///
/// Counted from each word's *final* input rather than from the keystrokes, so a
/// character typed and then taken back does not appear — which is what makes
/// this a table of the words produced rather than of the fingers involved.
pub fn per_key(log: &EventLog, targets: &[String], is_timed: bool) -> Vec<KeyUse> {
    let mut replay = Replay::new(targets, is_timed);
    for timed in log.events() {
        replay.apply(timed.event);
    }

    // A char -> (pressed, correct), kept sorted by count so the table reads the
    // way the site's does: the keys you actually use, most-used first.
    let mut table: std::collections::BTreeMap<char, (u32, u32)> = std::collections::BTreeMap::new();
    for (index, input) in replay.words() {
        if input.is_empty() {
            continue;
        }
        let target: Vec<char> = replay.target(index).chars().collect();
        for (position, ch) in input.chars().enumerate() {
            let slot = table.entry(ch).or_default();
            slot.0 += 1;
            if target.get(position) == Some(&ch) {
                slot.1 += 1;
            }
        }
    }

    let mut uses: Vec<KeyUse> = table
        .into_iter()
        .map(|(key, (pressed, correct))| KeyUse {
            key,
            pressed,
            correct,
        })
        .collect();
    uses.sort_by(|a, b| b.pressed.cmp(&a.pressed).then(a.key.cmp(&b.key)));
    uses
}

/// The key figures of a test, gathered together.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyStats {
    /// Gaps between keystrokes, in milliseconds.
    pub spacing: Vec<f64>,
    /// Consistency of the typing rhythm.
    pub consistency: f64,
    /// The letter table.
    pub keys: Vec<KeyUse>,
}

impl KeyStats {
    /// The mean gap between keystrokes, in milliseconds.
    ///
    /// This is the one number that answers "how fast are my hands", as opposed
    /// to how fast the words came out: it is unaffected by word length.
    pub fn mean_spacing(&self) -> f64 {
        round_to2(super::numbers::mean(&self.spacing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Event;

    fn log_of(times: &[f64]) -> EventLog {
        let mut log = EventLog::new();
        for (i, ms) in times.iter().copied().enumerate() {
            log.push(
                ms,
                Event::Insert {
                    word: 0,
                    index: i,
                    correct: true,
                    ch: 'a',
                },
            );
        }
        log
    }

    fn targets(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| format!("{t} ")).collect()
    }

    #[test]
    fn spacing_is_the_gap_between_keypresses() {
        let log = log_of(&[0.0, 100.0, 250.0, 400.0]);
        assert_eq!(key_spacing(&log), [100.0, 150.0, 150.0]);
    }

    #[test]
    fn one_keystroke_has_no_gaps() {
        assert!(key_spacing(&log_of(&[0.0])).is_empty());
        assert!(key_spacing(&EventLog::new()).is_empty());
    }

    #[test]
    fn a_backspace_and_a_skip_are_keystrokes_too() {
        let mut log = EventLog::new();
        log.push(
            0.0,
            Event::Insert {
                word: 0,
                index: 0,
                correct: true,
                ch: 'a',
            },
        );
        log.push(80.0, Event::Delete { word: 0, index: 0 });
        log.push(200.0, Event::Skip { word: 1 });
        assert_eq!(key_spacing(&log), [80.0, 120.0]);
    }

    #[test]
    fn a_keystroke_before_the_clock_is_clamped_to_zero() {
        // The clamp is on the *stored* time, not on the gap it produces: a stray
        // keypress 5ms before the start is remembered as being at 0, so the next
        // gap is measured from 0 rather than from -5.
        let log = log_of(&[-5.0, 100.0]);
        assert_eq!(key_spacing(&log), [100.0]);
    }

    #[test]
    fn the_rhythm_drops_the_gap_that_includes_the_pause_at_the_end() {
        // Two even gaps and one long one: the long one is the wait before the
        // test ended, and including it would make a steady typist look erratic.
        let log = log_of(&[0.0, 100.0, 200.0, 5000.0]);
        let spacing = key_spacing(&log);
        assert_eq!(spacing, [100.0, 100.0, 4800.0]);
        assert_eq!(key_consistency(&spacing), 100.0);
    }

    #[test]
    fn an_uneven_rhythm_scores_below_a_steady_one() {
        let steady = key_spacing(&log_of(&[0.0, 100.0, 200.0, 300.0, 400.0]));
        let uneven = key_spacing(&log_of(&[0.0, 50.0, 400.0, 450.0, 900.0]));
        assert!(key_consistency(&uneven) < key_consistency(&steady));
        assert_eq!(key_consistency(&steady), 100.0);
    }

    #[test]
    fn a_rhythm_of_one_gap_has_nothing_to_be_consistent_about() {
        // The last gap is always dropped, so two keystrokes leave an empty
        // series and score zero rather than 100.
        assert_eq!(key_consistency(&key_spacing(&log_of(&[0.0, 100.0]))), 0.0);
        assert_eq!(key_consistency(&[]), 0.0);
    }

    #[test]
    fn the_letter_table_counts_the_words_produced() {
        let list = targets(&["cat", "dog"]);
        let mut log = EventLog::new();
        let mut ms = 0.0;
        for (word, text) in ["cat ", "dog "].iter().enumerate() {
            for (position, ch) in text.chars().enumerate() {
                log.push(
                    ms,
                    Event::Insert {
                        word,
                        index: position,
                        correct: true,
                        ch,
                    },
                );
                ms += 100.0;
            }
        }
        let keys = per_key(&log, &list, true);
        let all_correct: bool = keys.iter().all(|k| k.correct == k.pressed);
        assert!(all_correct, "nothing was mistyped: {keys:?}");
        // 'd', 'o', 'g' and 'c', 'a', 't' twice each, the space three times.
        let space = keys.iter().find(|k| k.key == ' ').expect("a space");
        assert_eq!(space.pressed, 2);
        assert_eq!(keys.len(), 7, "six letters and a space: {keys:?}");
    }

    #[test]
    fn a_wrong_character_is_counted_as_a_wrong_use_of_that_key() {
        let list = targets(&["cat"]);
        let mut log = EventLog::new();
        for (position, ch) in "xat ".chars().enumerate() {
            log.push(
                position as f64 * 100.0,
                Event::Insert {
                    word: 0,
                    index: position,
                    correct: ch == 'a' || ch == 't' || ch == ' ',
                    ch,
                },
            );
        }
        let keys = per_key(&log, &list, true);
        let x = keys.iter().find(|k| k.key == 'x').expect("the x");
        assert_eq!(x.pressed, 1);
        assert_eq!(x.correct, 0);
        assert_eq!(x.wrong(), 1);
    }

    #[test]
    fn a_character_typed_and_taken_back_does_not_appear() {
        let list = targets(&["cat"]);
        let mut log = EventLog::new();
        log.push(
            0.0,
            Event::Insert {
                word: 0,
                index: 0,
                correct: false,
                ch: 'z',
            },
        );
        log.push(100.0, Event::Delete { word: 0, index: 0 });
        for (position, ch) in "cat ".chars().enumerate() {
            log.push(
                200.0 + position as f64 * 100.0,
                Event::Insert {
                    word: 0,
                    index: position,
                    correct: true,
                    ch,
                },
            );
        }
        let keys = per_key(&log, &list, true);
        assert!(
            keys.iter().all(|k| k.key != 'z'),
            "the table is of words produced, not of fingers: {keys:?}"
        );
    }

    #[test]
    fn keys_are_ordered_most_used_first() {
        let list = targets(&["aa", "b"]);
        let mut log = EventLog::new();
        let mut ms = 0.0;
        for (word, text) in ["aa ", "b "].iter().enumerate() {
            for (position, ch) in text.chars().enumerate() {
                log.push(
                    ms,
                    Event::Insert {
                        word,
                        index: position,
                        correct: true,
                        ch,
                    },
                );
                ms += 100.0;
            }
        }
        let keys = per_key(&log, &list, true);
        assert_eq!(keys[0].key, ' ', "the space is used most");
        for pair in keys.windows(2) {
            assert!(pair[0].pressed >= pair[1].pressed, "out of order: {keys:?}");
        }
    }

    #[test]
    fn the_mean_gap_is_a_hand_speed_not_a_word_speed() {
        let log = log_of(&[0.0, 100.0, 200.0, 300.0]);
        let stats = KeyStats {
            spacing: key_spacing(&log),
            consistency: 0.0,
            keys: Vec::new(),
        };
        assert_eq!(stats.mean_spacing(), 100.0);
    }
}
