//! The live chart: wpm, burst and errors, one bucket per second.
//!
//! Ported from `frontend/src/ts/test/events/stats.ts` — `getTimerBoundaries`,
//! `countPerInterval`, `getBurstHistory`, `getErrorCountHistory` and
//! `getWpmHistory`.
//!
//! The three series are not three views of one number, and confusing them is
//! the classic way to get a chart that looks plausible and is wrong:
//!
//! - **wpm** is *cumulative*. It divides everything typed up to second *i* by
//!   *i*, so it is the average so far, not the speed of second *i*. The old
//!   client drew per-second numbers here and its "wpm" line was really a burst
//!   line, which is why its curve did not behave like the website's.
//! - **burst** is the speed of one second alone.
//! - **err** is how many keystrokes in that second were wrong, and feeds the
//!   little red ticks under the burst line rather than an axis of its own.
//!
//! The cumulative series needs the *word* view of the test, not just the
//! keystroke view: `correct_word` comes from [`count_chars`] over each word's
//! reconstructed input, exactly as the website's `countCharsForWordIndex` does.
//! That is why this module replays the log instead of reading a running total.

use std::collections::BTreeMap;
use std::ops::Range;

use super::chars::count_chars;
use super::event_log::{Event, EventLog};
use super::numbers::{calculate_wpm, js_round, round_to2};

/// The longest chart worth building, in seconds.
///
/// Nothing can reach this in practice — a test runs out of words long before —
/// but `end_ms` comes from a wall clock, and an unfiltered clock should not be
/// able to allocate.
const MAX_BUCKETS: u32 = 3600;

/// The three series of a chart, one entry per whole second of the test.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    /// Cumulative words per minute: the average up to and including that second.
    pub wpm: Vec<f64>,
    /// Words per minute within that second alone.
    pub burst: Vec<f64>,
    /// Keystrokes entered wrongly within that second.
    pub err: Vec<u32>,
}

impl Chart {
    fn with_capacity(seconds: usize) -> Self {
        Self {
            wpm: Vec::with_capacity(seconds),
            burst: Vec::with_capacity(seconds),
            err: Vec::with_capacity(seconds),
        }
    }

    /// Whether the test is too short to have a bucket yet.
    pub fn is_empty(&self) -> bool {
        self.wpm.is_empty()
    }

    /// How many seconds the chart spans.
    pub fn len(&self) -> usize {
        self.wpm.len()
    }

    /// The last cumulative figure, which is the wpm a finished test reports.
    ///
    /// Falls back to the mean of the burst series when there is no bucket, so a
    /// test under a second long still shows a number.
    pub fn final_wpm(&self) -> f64 {
        match self.wpm.last() {
            Some(value) => *value,
            None => js_round(super::numbers::mean(&self.burst)),
        }
    }

    /// True steady speed: `stddev(burst) / mean(burst)`, mapped by `kogasa`.
    ///
    /// This is the *consistency* figure, and it is built from burst rather than
    /// wpm — a cumulative curve is smooth by construction and would always
    /// score well.
    pub fn consistency(&self) -> f64 {
        super::numbers::consistency(&self.burst)
    }

    /// Whether any keystroke was wrong, for the single red tick under the chart.
    pub fn has_errors(&self) -> bool {
        self.err.iter().any(|count| *count > 0)
    }

    /// The seconds one terminal column stands for.
    ///
    /// One second per column while the test fits on screen. Past that a column
    /// covers a run of seconds, because a test longer than the terminal is wide
    /// has to be compressed rather than clipped — silently dropping the tail
    /// would be worse than a coarser picture of it.
    fn column_span(&self, column: u16, columns: u16) -> Range<usize> {
        let len = self.len();
        if len == 0 || columns == 0 {
            return 0..0;
        }
        let per_column = len.div_ceil(columns as usize);
        let start = column as usize * per_column;
        start..(start + per_column).min(len)
    }

    /// The burst figure for a column: the fastest second it covers.
    ///
    /// The fastest, not the average. Smoothing a stall away is the one thing a
    /// chart must not do.
    pub fn burst_at(&self, column: u16, columns: u16) -> f64 {
        self.column_span(column, columns)
            .filter_map(|i| self.burst.get(i).copied())
            .fold(0.0, f64::max)
    }

    /// The cumulative wpm for a column: the last value it covers.
    ///
    /// Cumulative means the last, where the average so far is.
    pub fn wpm_at(&self, column: u16, columns: u16) -> f64 {
        self.column_span(column, columns)
            .filter_map(|i| self.wpm.get(i).copied())
            .next_back()
            .unwrap_or(0.0)
    }

    /// The wrong keystrokes in a column.
    pub fn err_at(&self, column: u16, columns: u16) -> u32 {
        self.column_span(column, columns)
            .filter_map(|i| self.err.get(i).copied())
            .sum()
    }
}

/// Everything the chart needs beyond the log itself.
#[derive(Debug, Clone, Copy)]
pub struct ChartContext<'a> {
    /// Every target word, each *including* the character that ends it — the
    /// website calls this `textWithCommit`, and the trailing space is part of
    /// the target.
    pub targets: &'a [String],
    /// A test that runs on a clock rather than to a fixed length.
    ///
    /// Two things hinge on it, and both come straight from the original: the
    /// word being typed counts towards wpm as it grows, and the chart does not
    /// need a fractional trailing bucket. See [`crate::engine::Mode::is_timed`].
    pub is_timed: bool,
    /// How far the test reached — its final duration once it is over, or the
    /// elapsed time so far while it is still running.
    pub end_ms: f64,
}

/// Builds the chart for a test that has so far reached `ctx.end_ms`.
pub fn build(log: &EventLog, ctx: &ChartContext<'_>) -> Chart {
    let boundaries = boundaries(ctx.end_ms, ctx.is_timed);
    let events = log.events();
    let mut replay = Replay::new(ctx);
    let mut chart = Chart::with_capacity(boundaries.len());

    // A single cursor walks the log once. The events it consumes inside a
    // bucket are both that second's keystrokes and everything the cumulative
    // series needs, so the two views cannot drift apart.
    let mut next = 0;
    let mut previous_boundary = 0.0;

    for &boundary in &boundaries {
        let mut typed = 0u32;
        let mut wrong = 0u32;
        while let Some(timed) = events.get(next) {
            if timed.ms > boundary {
                break;
            }
            next += 1;
            if let Event::Insert { correct, .. } = timed.event {
                typed += 1;
                if !correct {
                    wrong += 1;
                }
            }
            replay.apply(timed.event);
        }

        let seconds = (boundary - previous_boundary) / 1000.0;
        previous_boundary = boundary;

        chart
            .burst
            .push(js_round(calculate_wpm(f64::from(typed), seconds)));
        chart.err.push(wrong);
        chart.wpm.push(js_round(calculate_wpm(
            f64::from(replay.correct_word()),
            boundary / 1000.0,
        )));
    }

    chart
}

/// The second boundaries of the chart: `[1000, 2000, …, floor(end) * 1000]`.
///
/// A test that is not on a clock and stopped part-way through a second gets one
/// extra, short, final bucket so its last moments are not thrown away. The
/// website adds that only when the remainder rounds up to half a second, which
/// is reproduced here.
fn boundaries(end_ms: f64, is_timed: bool) -> Vec<f64> {
    if !end_ms.is_finite() || end_ms <= 0.0 {
        return Vec::new();
    }
    let ticks = ((end_ms / 1000.0).floor() as u32).min(MAX_BUCKETS);
    let mut out: Vec<f64> = (1..=ticks).map(|i| f64::from(i) * 1000.0).collect();

    let seconds = round_to2(end_ms / 1000.0);
    if !is_timed && js_round(seconds % 1.0) >= 0.5 {
        out.push(end_ms);
    }
    out
}

/// Replays an [`EventLog`] far enough to score the test as it stood at any one
/// moment.
struct Replay<'a> {
    /// What has been entered in each word, reconstructed from insert/delete.
    inputs: BTreeMap<usize, String>,
    /// The word the typist is in, which is the one allowed to be half-typed.
    active: usize,
    targets: &'a [String],
    /// Whether a half-typed word counts — true only on a clock.
    credit_partial: bool,
}

impl<'a> Replay<'a> {
    fn new(ctx: &'a ChartContext<'_>) -> Self {
        Self {
            inputs: BTreeMap::new(),
            active: 0,
            targets: ctx.targets,
            credit_partial: ctx.is_timed,
        }
    }

    fn apply(&mut self, event: Event) {
        let word = event.word();
        match event {
            Event::Insert { ch, .. } => {
                self.inputs.entry(word).or_default().push(ch);
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
                self.active = word;
            }
            Event::Skip { .. } => {
                self.inputs.insert(word, String::new());
                self.active = word + 1;
            }
        }
    }

    /// `correct_word` for one word, as it stands right now.
    fn word_counts(&self, word: usize) -> u32 {
        let input = self.inputs.get(&word).map(String::as_str).unwrap_or("");
        // A word with no target in the list cannot be scored; treat what was
        // typed as the target, which is what the website does when the word is
        // missing from `targetWords`.
        let target = self.targets.get(word).map(String::as_str).unwrap_or(input);
        let credit = self.credit_partial && word == self.active;
        count_chars(input, target, credit).correct_word
    }

    /// The cumulative `correct_word` the wpm series divides by time.
    ///
    /// Words *after* the active one are excluded, which is what the website's
    /// `break` on the active word does: once you backspace across a commit, the
    /// word you left behind is no longer part of the score.
    fn correct_word(&self) -> u32 {
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

    /// A test in progress: a log, the targets it was typed against, and a way to
    /// ask what the chart looked like at a given moment.
    struct Scripted {
        log: EventLog,
        targets: Vec<String>,
        /// What has been entered in each word, so a second call to
        /// [`Scripted::type_word`] continues rather than restarts.
        typed: Vec<String>,
    }

    impl Scripted {
        /// Every word carries the character that ends it, as `textWithCommit`
        /// does, so the trailing space is part of the target.
        fn new(texts: &[&str]) -> Self {
            Self {
                log: EventLog::new(),
                targets: texts.iter().map(|t| format!("{t} ")).collect(),
                typed: vec![String::new(); texts.len()],
            }
        }

        /// Types `text` into `word`, `step` milliseconds apart, judging each
        /// character against the target by position exactly as the engine does.
        ///
        /// A bucket takes everything up to *and including* its second, as
        /// `countPerInterval` does, so a keystroke landing exactly on 1000ms
        /// counts towards the first second. The timings below dodge that.
        fn type_word(&mut self, word: usize, at: f64, step: f64, text: &str) {
            let target: Vec<char> = self.targets[word].chars().collect();
            for (offset, ch) in text.chars().enumerate() {
                let index = self.typed[word].chars().count();
                self.log.push(
                    at + step * offset as f64,
                    Event::Insert {
                        word,
                        index,
                        correct: target.get(index) == Some(&ch),
                        ch,
                    },
                );
                self.typed[word].push(ch);
            }
        }

        /// Takes back the last character of a word.
        fn backspace(&mut self, word: usize, at: f64) {
            let index = self.typed[word].chars().count().saturating_sub(1);
            self.typed[word].pop();
            self.log.push(at, Event::Delete { word, index });
        }

        fn chart(&self, is_timed: bool, end_ms: f64) -> Chart {
            build(
                &self.log,
                &ChartContext {
                    targets: &self.targets,
                    is_timed,
                    end_ms,
                },
            )
        }
    }

    #[test]
    fn there_is_no_bucket_before_the_first_second() {
        let mut test = Scripted::new(&["cat"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        let chart = test.chart(true, 640.0);
        assert!(chart.is_empty());
        assert_eq!(chart.len(), 0);
    }

    #[test]
    fn a_bucket_is_a_whole_second_on_an_ideal_grid() {
        // Keystrokes 300ms apart: the buckets must land on 1s, 2s and 3s,
        // whatever the events' own timings were.
        let mut test = Scripted::new(&["cat"]);
        test.type_word(0, 0.0, 300.0, "cat ");
        assert_eq!(test.chart(true, 3500.0).len(), 3);
    }

    #[test]
    fn burst_is_the_second_alone_and_wpm_is_the_average_so_far() {
        let mut test = Scripted::new(&["cat", "dogs", "birds"]);
        // 4 characters in the first second, 5 in the second, 6 in the third.
        test.type_word(0, 0.0, 200.0, "cat ");
        test.type_word(1, 1100.0, 150.0, "dogs ");
        test.type_word(2, 2200.0, 120.0, "birds ");
        let chart = test.chart(true, 3000.0);

        // chars/5/1s*60: 4→48, 5→60, 6→72.
        assert_eq!(chart.burst, [48.0, 60.0, 72.0]);
        // Cumulative: 4 over 1s→48, 9 over 2s→54, 15 over 3s→60.
        assert_eq!(chart.wpm, [48.0, 54.0, 60.0]);
        assert_eq!(chart.final_wpm(), 60.0);
    }

    #[test]
    fn the_two_series_agree_only_in_the_first_second() {
        // By construction the average over one second *is* that second, so the
        // only place they can be confused is after the first bucket — which is
        // exactly where the old client drew burst numbers as wpm.
        let mut test = Scripted::new(&["cat", "dogs", "birds"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        test.type_word(1, 1100.0, 150.0, "dogs ");
        test.type_word(2, 2200.0, 120.0, "birds ");
        let chart = test.chart(true, 3000.0);
        assert_eq!(chart.wpm[0], chart.burst[0]);
        assert_ne!(chart.wpm[1], chart.burst[1]);
        assert_ne!(chart.wpm[2], chart.burst[2]);
    }

    #[test]
    fn a_stall_leaves_an_empty_second_rather_than_being_ignored() {
        let mut test = Scripted::new(&["cat", "dog"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        // The typist walks away and comes back five seconds later.
        test.log.push(6000.0, Event::Skip { word: 1 });
        let chart = test.chart(true, 6000.0);

        assert_eq!(chart.len(), 6);
        assert_eq!(chart.burst[0], 48.0);
        for second in 1..=5 {
            assert_eq!(chart.burst[second], 0.0, "second {} is empty", second + 1);
        }
        // Time passing with nothing typed drags the average down, never up.
        assert!(
            chart.wpm.windows(2).all(|w| w[1] <= w[0]),
            "cumulative wpm must not rise while nothing is typed: {:?}",
            chart.wpm
        );
        assert_eq!(chart.wpm[5], 8.0, "4 characters over 6 seconds is 8 wpm");
    }

    #[test]
    fn the_half_typed_word_counts_only_on_a_clock() {
        // Two characters of a six-character target, then the clock runs out.
        let mut test = Scripted::new(&["three"]);
        test.type_word(0, 0.0, 200.0, "th");

        assert_eq!(
            test.chart(true, 1000.0).wpm[0],
            24.0,
            "2 characters in a second is 24 wpm, prefix included"
        );
        assert_eq!(
            test.chart(false, 1000.0).wpm[0],
            0.0,
            "a word-count test scores nothing until the word is finished"
        );
        assert_eq!(
            test.chart(false, 1000.0).burst[0],
            24.0,
            "burst counts keystrokes either way"
        );
    }

    #[test]
    fn committing_a_half_typed_word_takes_its_credit_back() {
        // "th" is credited while it is the word being typed and loses that
        // credit the moment it is committed, which is what makes a timed wpm
        // curve dip when you give up on a word.
        let mut test = Scripted::new(&["three"]);
        test.type_word(0, 0.0, 200.0, "th ");
        assert_eq!(
            test.chart(true, 1000.0).wpm[0],
            0.0,
            "'th ' is not a prefix of 'three '"
        );
    }

    #[test]
    fn a_correct_half_typed_word_is_credited_as_it_grows() {
        // The same word typed properly but not finished: the curve rises with
        // every keystroke instead of jumping at the commit.
        let mut test = Scripted::new(&["three"]);
        test.type_word(0, 0.0, 200.0, "th");
        let after_two = test.chart(true, 1000.0);
        test.type_word(0, 400.0, 200.0, "re");
        let after_four = test.chart(true, 1000.0);
        assert_eq!(after_two.wpm[0], 24.0);
        assert_eq!(after_four.wpm[0], 48.0);
    }

    #[test]
    fn a_skipped_word_is_not_a_keystroke() {
        let mut test = Scripted::new(&["a", "b"]);
        test.type_word(0, 0.0, 0.0, "a");
        test.log.push(100.0, Event::Skip { word: 1 });
        let chart = test.chart(true, 1000.0);
        assert_eq!(chart.burst[0], 12.0, "one keystroke in the first second");
        assert_eq!(chart.wpm[0], 0.0, "'a' alone is not a prefix of 'a '");
    }

    #[test]
    fn a_backspace_is_not_a_keystroke_but_the_error_sticks() {
        let mut test = Scripted::new(&["cat"]);
        test.type_word(0, 0.0, 100.0, "catx");
        test.backspace(0, 400.0);
        test.type_word(0, 500.0, 0.0, " ");
        let chart = test.chart(true, 1000.0);

        // Five keystrokes: c, a, t, x, and the space that took x's place.
        assert_eq!(chart.burst[0], 60.0, "the backspace is not a keystroke");
        assert_eq!(
            chart.err[0], 1,
            "the 'x' happened, even though it was undone"
        );
        assert_eq!(chart.wpm[0], 48.0, "'cat ' is four correct characters");
        assert!(chart.has_errors());
    }

    #[test]
    fn a_fully_backed_out_word_leaves_the_score() {
        // Type "cat ", start "dog" with one character, then take it back: the
        // last character is withdrawn, so the average drops below where it was.
        let mut test = Scripted::new(&["cat", "dog"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        test.type_word(1, 1100.0, 200.0, "d");
        assert_eq!(
            test.chart(true, 2000.0).wpm[1],
            30.0,
            "4 finished plus 1 half-typed, over 2 seconds"
        );

        test.backspace(1, 1400.0);
        let chart = test.chart(true, 2000.0);
        assert_eq!(
            chart.burst[1], 12.0,
            "one keystroke, and the backspace is not one"
        );
        assert_eq!(
            chart.wpm[1], 24.0,
            "only 'cat ' is left, so 4 characters over 2 seconds"
        );
    }

    #[test]
    fn a_words_left_behind_stop_counting_once_you_come_back() {
        // Type two words, then backspace across the commit into the first. The
        // website breaks its tally at the active word, and so must this.
        let mut test = Scripted::new(&["cat", "dog"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        test.type_word(1, 1100.0, 200.0, "dog ");
        assert_eq!(
            test.chart(true, 2000.0).wpm[1],
            48.0,
            "8 correct characters over 2 seconds"
        );

        // The trailing space goes, leaving the first word active again.
        test.backspace(1, 1900.0);
        assert_eq!(
            test.chart(true, 2000.0).wpm[1],
            42.0,
            "'dog' is a prefix again, so 7 characters over 2 seconds"
        );
    }

    #[test]
    fn only_a_test_off_the_clock_gets_a_short_final_bucket() {
        let mut test = Scripted::new(&["cat"]);
        test.type_word(0, 0.0, 200.0, "cat ");

        // 1.5 seconds: the fraction rounds up, so a words test keeps a bucket
        // for the half second it really ran.
        let untimed = test.chart(false, 1500.0);
        assert_eq!(untimed.len(), 2);
        assert_eq!(untimed.burst[1], 0.0, "the tail half second was empty");

        // A timed test does not: its seconds are the clock's seconds.
        assert_eq!(test.chart(true, 1500.0).len(), 1);
    }

    #[test]
    fn a_fraction_under_half_a_second_is_not_worth_a_bucket() {
        let mut test = Scripted::new(&["cat"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        assert_eq!(test.chart(false, 1400.0).len(), 1);
    }

    #[test]
    fn consistency_reads_the_burst_not_the_cumulative_curve() {
        // A cumulative curve is smooth by construction and would always score
        // well, so consistency has to be built from the burst series.
        let mut test = Scripted::new(&["cat", "bat"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        test.type_word(1, 1100.0, 200.0, "bat ");
        let chart = test.chart(true, 2000.0);

        assert_eq!(chart.burst, [48.0, 48.0], "a flat series of speed");
        assert_eq!(chart.wpm, [48.0, 48.0]);
        assert_eq!(chart.consistency(), 100.0);
    }

    #[test]
    fn consistency_penalises_an_uneven_test() {
        // 4 characters then 6: mean 60, deviation 12, cov 0.2.
        let mut test = Scripted::new(&["cat", "birds"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        test.type_word(1, 1100.0, 180.0, "birds ");
        let chart = test.chart(true, 2000.0);

        assert_eq!(chart.burst, [48.0, 72.0]);
        // kogasa(0.2) is 80.0002, rounded to two decimals.
        assert_eq!(chart.consistency(), 80.0);
    }

    #[test]
    fn a_chart_of_a_very_long_test_is_capped_rather_than_allocated() {
        let mut test = Scripted::new(&["cat"]);
        test.type_word(0, 0.0, 200.0, "cat ");
        let chart = test.chart(true, 10.0 * 60.0 * 60.0 * 1000.0);
        assert_eq!(chart.len(), MAX_BUCKETS as usize);
    }

    #[test]
    fn a_word_with_no_target_is_scored_against_what_was_typed() {
        // No targets at all: a word is its own target, so 'a' is correct.
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
        let chart = build(
            &log,
            &ChartContext {
                targets: &[],
                is_timed: true,
                end_ms: 1000.0,
            },
        );
        assert_eq!(chart.wpm[0], 12.0, "1 character in a second is 12 wpm");
    }
}
