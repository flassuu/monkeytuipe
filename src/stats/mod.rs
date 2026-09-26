//! Test scoring.
//!
//! Everything here is a faithful port of monkeytype's own arithmetic, so that a
//! result computed in the terminal matches the one the website would compute.
//! Nothing in this module knows about ratatui or the terminal.

pub mod chars;
pub mod numbers;

pub use chars::{count_chars, CharCounts};
pub use numbers::{calculate_wpm, kogasa, mean, round_to2, std_dev};
