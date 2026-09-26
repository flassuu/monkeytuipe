//! Test scoring.
//!
//! Everything here is a faithful port of monkeytype's own arithmetic, so that a
//! result computed in the terminal matches the one the website would compute.
//! Nothing in this module knows about ratatui or the terminal, and nothing in it
//! depends on the typing engine — the handoff is [`EventLog`], a record of what
//! the typist did and when.

pub mod chars;
pub mod chart;
pub mod event_log;
pub mod numbers;

pub use chars::{count_chars, CharCounts};
pub use chart::{build as build_chart, Chart, ChartContext};
pub use event_log::{Event, EventLog, TimedEvent};
pub use numbers::{calculate_wpm, consistency, js_round, kogasa, mean, round_to2, std_dev};
