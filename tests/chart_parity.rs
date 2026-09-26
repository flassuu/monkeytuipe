//! Differential test of the chart against the website's algorithm.
//!
//! `research/chart-oracle.js` is a transliteration of the three series in
//! `frontend/src/ts/test/events/stats.ts` (`getWpmHistory`, `getBurstHistory`,
//! `getErrorCountHistory` and the boundary grid from `getTimerBoundaries`),
//! driven by a simulated typist that mistypes, backspaces, skips words and
//! stalls. It was run once and its output committed to
//! `tests/data/chart_vectors.json`; this test replays those exact inputs and
//! compares all three series bucket for bucket.
//!
//! The vectors are committed rather than regenerated so CI needs no Node. When
//! the algorithm changes on purpose, regenerate and read the diff:
//!
//! ```text
//! node research/chart-oracle.js > tests/data/chart_vectors.json
//! ```

use monkeytuipe::engine::Mode;
use monkeytuipe::stats::chart::{build, ChartContext};
use monkeytuipe::stats::{Event, EventLog};

const VECTORS: &str = include_str!("data/chart_vectors.json");

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum RawEvent {
    Insert {
        ms: f64,
        word: usize,
        correct: bool,
        ch: char,
    },
    Delete {
        ms: f64,
        word: usize,
    },
    Skip {
        ms: f64,
        word: usize,
    },
}

impl RawEvent {
    fn ms(&self) -> f64 {
        match self {
            Self::Insert { ms, .. } | Self::Delete { ms, .. } | Self::Skip { ms, .. } => *ms,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct Case {
    name: String,
    mode: String,
    mode2: u32,
    /// Each target includes its own commit character, as `textWithCommit` does.
    targets: Vec<String>,
    events: Vec<RawEvent>,
    #[serde(rename = "endMs")]
    end_ms: f64,
}

#[derive(Debug, serde::Deserialize)]
struct Expected {
    wpm: Vec<f64>,
    burst: Vec<f64>,
    err: Vec<u32>,
}

#[derive(Debug, serde::Deserialize)]
struct Vector {
    case: Case,
    expected: Expected,
}

fn vectors() -> Vec<Vector> {
    serde_json::from_str(VECTORS).expect("the committed vectors parse")
}

fn mode(case: &Case) -> Mode {
    match case.mode.as_str() {
        "time" => Mode::Time,
        "words" => Mode::Words,
        "quote" => Mode::Quote,
        other => panic!("unknown mode {other}"),
    }
}

fn build_for(case: &Case) -> monkeytuipe::stats::Chart {
    let mut log = EventLog::new();
    for raw in &case.events {
        // The index is only ever needed to replay a mid-word replace, which the
        // engine cannot produce: characters are appended and a deletion is a
        // pop, so the chart's own replay never reads it.
        let event = match raw {
            RawEvent::Insert {
                word, correct, ch, ..
            } => Event::Insert {
                word: *word,
                index: 0,
                correct: *correct,
                ch: *ch,
            },
            RawEvent::Delete { word, .. } => Event::Delete {
                word: *word,
                index: 0,
            },
            RawEvent::Skip { word, .. } => Event::Skip { word: *word },
        };
        log.push(raw.ms(), event);
    }
    let mode = mode(case);
    let ctx = ChartContext {
        targets: &case.targets,
        is_timed: mode.is_timed(case.mode2),
        end_ms: case.end_ms,
    };
    build(&log, &ctx)
}

#[test]
fn every_series_matches_the_oracle() {
    let vectors = vectors();
    assert!(!vectors.is_empty(), "the corpus must not be empty");

    let mut checked = 0usize;
    for vector in &vectors {
        let chart = build_for(&vector.case);
        let expected = &vector.expected;

        assert_eq!(
            chart.wpm.len(),
            expected.wpm.len(),
            "{}: wpm bucket count",
            vector.case.name
        );
        assert_eq!(
            chart.burst.len(),
            expected.burst.len(),
            "{}: burst bucket count",
            vector.case.name
        );
        assert_eq!(
            chart.err.len(),
            expected.err.len(),
            "{}: err bucket count",
            vector.case.name
        );

        for i in 0..expected.wpm.len() {
            assert_eq!(
                chart.wpm[i],
                expected.wpm[i],
                "{}: wpm at second {}",
                vector.case.name,
                i + 1
            );
            assert_eq!(
                chart.burst[i],
                expected.burst[i],
                "{}: burst at second {}",
                vector.case.name,
                i + 1
            );
            assert_eq!(
                chart.err[i],
                expected.err[i],
                "{}: errors at second {}",
                vector.case.name,
                i + 1
            );
        }
        checked += expected.wpm.len();
    }
    assert!(
        checked > 1000,
        "the corpus must be wide, got {checked} buckets"
    );
}

#[test]
fn the_corpus_exercises_every_branch() {
    let vectors = vectors();

    let buckets: usize = vectors.iter().map(|v| v.expected.wpm.len()).sum();
    assert!(buckets > 1000, "buckets: {buckets}");

    let errors: u32 = vectors.iter().flat_map(|v| &v.expected.err).sum::<u32>();
    assert!(errors > 500, "errors: {errors}");

    let inserts = count(&vectors, |e| matches!(e, RawEvent::Insert { .. }));
    let deletes = count(&vectors, |e| matches!(e, RawEvent::Delete { .. }));
    let skips = count(&vectors, |e| matches!(e, RawEvent::Skip { .. }));
    assert!(inserts > 1000, "inserts: {inserts}");
    assert!(deletes > 100, "deletes: {deletes}");
    assert!(skips > 20, "skips: {skips}");

    // The fractional trailing bucket, which only a non-timed test can produce.
    let tails = vectors
        .iter()
        .filter(|v| {
            let timed = mode(&v.case).is_timed(v.case.mode2);
            let full = (v.case.end_ms / 1000.0).floor();
            !timed && full + 1.0 == v.expected.wpm.len() as f64
        })
        .count();
    assert!(tails > 5, "fractional trailing buckets: {tails}");

    // An empty second, which is what a stall in the middle of a test produces
    // and what makes the burst line dip.
    let flat = vectors
        .iter()
        .flat_map(|v| &v.expected.burst)
        .filter(|b| **b == 0.0)
        .count();
    assert!(flat > 5, "empty seconds: {flat}");

    // A cumulative curve that never equals the instantaneous one, or the
    // distinction between the two series has silently been lost.
    let differ = vectors
        .iter()
        .filter(|v| v.expected.wpm != v.expected.burst)
        .count();
    assert!(
        differ > 50,
        "cumulative differs from burst in {differ} cases"
    );
}

fn count(vectors: &[Vector], wanted: impl Fn(&RawEvent) -> bool) -> usize {
    vectors
        .iter()
        .flat_map(|v| &v.case.events)
        .filter(|e| wanted(e))
        .count()
}
