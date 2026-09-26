//! A finished test, scored.
//!
//! Every figure here is the website's own arithmetic, so that a number this
//! client shows and a number monkeytype stores are the same number. Where the
//! two are computed from *different* inputs that is deliberate and noted: the
//! headline wpm counts `correct_word` while accuracy counts keystrokes, and
//! mixing them up is the classic way to end up with a result that disagrees with
//! the site in a way nobody can find.
//!
//! The two figures a terminal genuinely cannot produce are absent rather than
//! approximated — see [`crate::stats::keys`].

use super::chart::{self, Chart};
use super::event_log::EventLog;
use super::keys::{self, KeyStats};
use super::numbers::{calculate_wpm, js_round, kogasa, mean, round_to2, std_dev};
use super::{CharCounts, Inputs};

/// Everything a finished test is reported as.
#[derive(Debug, Clone, PartialEq)]
pub struct TestResult {
    /// Corrected words per minute, the headline figure.
    pub wpm: f64,
    /// Words per minute before credit for partial words, i.e. every character
    /// that came out right whatever word it landed in.
    pub raw_wpm: f64,
    /// Per-keystroke accuracy as a percentage.
    pub accuracy: f64,
    /// How even the burst series was, as a percentage.
    pub consistency: f64,
    /// The same measure over the cumulative wpm curve.
    ///
    /// Always flattering — a cumulative average is smooth by construction — which
    /// is why the site shows it next to [`Self::consistency`] rather than
    /// instead of it.
    pub wpm_consistency: f64,
    /// Keystrokes right and wrong.
    pub inputs: Inputs,
    /// Characters, counted after the fact.
    pub chars: CharCounts,
    /// How long the test took, in seconds.
    pub duration_secs: f64,
    /// The three chart series.
    pub chart: Chart,
    /// The key figures.
    pub keys: KeyStats,
}

impl TestResult {
    /// Corrected words per minute.
    pub fn wpm(&self) -> f64 {
        self.wpm
    }
}

/// The inputs a finished test is scored from.
///
/// The three things a result needs and cannot work out for itself: the log, the
/// targets it was typed against, and how long it ran.
pub struct Scoring<'a> {
    pub log: &'a EventLog,
    /// Each target including the character that ends it.
    pub targets: &'a [String],
    /// Whether the test ran against a clock, which decides whether the word
    /// being typed counts.
    pub is_timed: bool,
    /// The test's duration, in seconds.
    pub duration_secs: f64,
    /// The engine's own counters, which are authoritative for the characters.
    pub chars: CharCounts,
    pub inputs: Inputs,
}

/// Scores a finished test.
///
/// `chars` and `inputs` come from the engine rather than from the log, because
/// the engine is where they are maintained and the two are counted differently
/// on purpose — see [`crate::engine`]. Everything else is derived here.
pub fn score(input: Scoring<'_>) -> TestResult {
    let duration = input.duration_secs;
    let chars = input.chars;

    // `getChars(eventLog, true).correctWord`, the same figure the chart's last
    // point and the live counter use.
    let correct_word = f64::from(chars.correct_word);
    // Raw counts every character that came out right, whatever word it landed
    // in, plus the wrong and extra ones. Note it is *not* the denominator of
    // accuracy: accuracy is per keystroke.
    let raw_characters = f64::from(chars.all_correct + chars.incorrect + chars.extra);

    let chart = chart::build(
        input.log,
        &chart::ChartContext {
            targets: input.targets,
            is_timed: input.is_timed,
            end_ms: duration * 1000.0,
        },
    );
    let spacing = keys::key_spacing(input.log);
    let key_consistency = keys::key_consistency(&spacing);
    let key_table = keys::per_key(input.log, input.targets, input.is_timed);

    TestResult {
        wpm: round_to2(calculate_wpm(correct_word, duration)),
        raw_wpm: round_to2(calculate_wpm(raw_characters, duration)),
        accuracy: round_to2(input.inputs.accuracy()),
        consistency: round_to2_or_zero(kogasa_of(&chart.burst)),
        wpm_consistency: round_to2_or_zero(kogasa_of(&chart.wpm)),
        inputs: input.inputs,
        chars,
        duration_secs: duration,
        keys: KeyStats {
            spacing,
            consistency: key_consistency,
            keys: key_table,
        },
        chart,
    }
}

/// `kogasa(stddev / mean)`, or `0` where the website substitutes `0` for a `NaN`.
///
/// An empty series has a mean of `0`, and `0/0` is not a consistency. The site
/// checks with `!consistency || isNaN(consistency)` — which also catches a real
/// zero, so a test that genuinely scores zero consistency and a test that cannot
/// be scored come out the same. That is the site's behaviour, kept as it is:
/// inventing a different value here would make this client disagree with the
/// leaderboard about which of the two it is.
fn kogasa_of(series: &[f64]) -> f64 {
    let value = kogasa(std_dev(series) / mean(series));
    if value == 0.0 || value.is_nan() {
        return 0.0;
    }
    value
}

fn round_to2_or_zero(value: f64) -> f64 {
    let rounded = round_to2(value);
    if rounded == 0.0 || rounded.is_nan() {
        0.0
    } else {
        rounded
    }
}

/// Rounds a wpm for display, the way the counters do: no decimals.
pub fn display_wpm(wpm: f64) -> f64 {
    js_round(wpm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Event;

    /// A finished test: the words typed, and how long they took.
    fn log_of(times: &[f64], text: &str) -> EventLog {
        let mut log = EventLog::new();
        let mut word = 0usize;
        let mut index = 0usize;
        for (i, ch) in text.chars().enumerate() {
            if ch == ' ' {
                word += 1;
                index = 0;
            } else {
                index += 1;
            }
            log.push(
                times.get(i).copied().unwrap_or(0.0),
                Event::Insert {
                    word,
                    index,
                    correct: true,
                    ch,
                },
            );
        }
        log
    }

    fn targets(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| format!("{t} ")).collect()
    }

    /// Scores a test from a word list and the text that was typed.
    fn score_text(targets: &[String], text: &str, duration: f64, is_timed: bool) -> TestResult {
        let log = log_of(&[], text);
        let chars = crate::stats::count_chars(text, &targets.join(""), is_timed);
        let inputs = Inputs {
            correct: text.chars().filter(|c| *c != ' ').count() as u32,
            incorrect: 0,
        };
        score(Scoring {
            log: &log,
            targets,
            is_timed,
            duration_secs: duration,
            chars,
            inputs,
        })
    }

    #[test]
    fn wpm_counts_correct_characters_over_the_duration() {
        let list = targets(&["cat", "dog"]);
        // 'cat dog ' is eight characters, commit spaces included.
        let result = score_text(&list, "cat dog ", 8.0, true);
        // 8 / 5 / (8/60) = 12
        assert!((result.wpm - 12.0).abs() < 1e-9, "got {}", result.wpm);
        assert_eq!(result.wpm, round_to2(result.wpm), "two decimals");
    }

    #[test]
    fn raw_wpm_counts_everything_that_came_out() {
        let list = targets(&["cat", "dog"]);
        let result = score_text(&list, "cat dog ", 8.0, true);
        // Nothing was wrong, so raw and corrected agree.
        assert_eq!(result.raw_wpm, result.wpm);

        // A word that was committed early: the characters are right, the word is
        // not, so raw is higher than wpm.
        let log = log_of(&[], "cat do ");
        let chars = crate::stats::count_chars("cat do ", "cat dog ", true);
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 8.0,
            chars,
            inputs: Inputs {
                correct: 6,
                incorrect: 1,
            },
        });
        assert!(
            result.raw_wpm > result.wpm,
            "raw {} should beat corrected {}",
            result.raw_wpm,
            result.wpm
        );
    }

    #[test]
    fn accuracy_is_per_keystroke_and_wpm_is_per_character() {
        let list = targets(&["cat"]);
        let log = log_of(&[], "xat ");
        let chars = crate::stats::count_chars("xat ", "cat ", false);
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 10.0,
            chars,
            inputs: Inputs {
                correct: 3,
                incorrect: 1,
            },
        });
        assert_eq!(result.accuracy, 75.0);
        // The word is not correct, so no character of it is credited to wpm —
        // which is the whole reason the two measures exist separately.
        assert_eq!(result.wpm, 0.0);
    }

    #[test]
    fn consistency_of_a_flat_test_is_a_hundred() {
        let list = targets(&["cat", "dog", "sun", "cow"]);
        let log = log_of(&[], "cat dog sun cow ");
        let chars = crate::stats::count_chars("cat dog sun cow ", &list.join(""), true);
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 1.0,
            chars,
            inputs: Inputs {
                correct: 15,
                incorrect: 0,
            },
        });
        assert_eq!(result.consistency, 100.0, "four identical seconds");
    }

    #[test]
    fn a_test_too_short_to_score_reports_zero_rather_than_nan() {
        let list = targets(&["cat"]);
        let log = EventLog::new();
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 0.4,
            chars: CharCounts::default(),
            inputs: Inputs::default(),
        });
        assert_eq!(result.wpm, 0.0);
        assert_eq!(result.accuracy, 0.0);
        assert_eq!(result.consistency, 0.0);
        assert_eq!(result.wpm_consistency, 0.0);
        for value in [
            result.wpm,
            result.raw_wpm,
            result.accuracy,
            result.consistency,
            result.wpm_consistency,
        ] {
            assert!(!value.is_nan(), "a NaN leaked into the result");
        }
    }

    #[test]
    fn wpm_consistency_reads_the_cumulative_curve() {
        let list = targets(&["cat", "dog", "sun", "cow"]);
        let log = log_of(&[], "cat dog sun cow ");
        let chars = crate::stats::count_chars("cat dog sun cow ", &list.join(""), true);
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 1.0,
            chars,
            inputs: Inputs {
                correct: 15,
                incorrect: 0,
            },
        });
        // The cumulative curve of a one-second test is a single point, which has
        // no spread — so this scores 100 whatever the burst did. Which is exactly
        // why it is the lesser of the two figures.
        assert_eq!(result.wpm_consistency, 100.0);
    }

    #[test]
    fn the_key_figures_come_along_with_the_result() {
        let list = targets(&["cat", "dog"]);
        let log = log_of(&[0.0, 100.0, 200.0, 300.0], "cat dog ");
        let chars = crate::stats::count_chars("cat dog ", &list.join(""), true);
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 0.4,
            chars,
            inputs: Inputs {
                correct: 8,
                incorrect: 0,
            },
        });
        assert_eq!(result.keys.spacing.len(), 7, "eight keystrokes, seven gaps");
        assert!(!result.keys.keys.is_empty());
    }

    #[test]
    fn the_duration_is_kept_verbatim() {
        let list = targets(&["cat"]);
        let log = log_of(&[], "cat ");
        let result = score(Scoring {
            log: &log,
            targets: &list,
            is_timed: true,
            duration_secs: 12.345,
            chars: crate::stats::count_chars("cat ", "cat ", true),
            inputs: Inputs {
                correct: 4,
                incorrect: 0,
            },
        });
        assert_eq!(result.duration_secs, 12.345);
    }
}
