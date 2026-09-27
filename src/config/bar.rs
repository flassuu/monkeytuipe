//! The settings bar: the row of choices along the top of the typing screen.
//!
//! The website puts every test setting in one strip above the words, and changes
//! the test the moment you click one. This is the same thing, built out of keys
//! instead of a mouse: the bar is always on screen, one field is selected, the
//! arrows move between fields and change them, and the test regenerates as you
//! go so you can see what you picked before you type it.
//!
//! ## Where the choices come from
//!
//! The lists here are the website's, in the website's order — the same order
//! matters, because a bar where `30s` comes after `120s` is a bar nobody can
//! find their way around. Anything that *is* user data rather than a fixed
//! choice (the language, the custom text) is deliberately not here: those are
//! free text and a list of several hundred entries, and they belong on the
//! settings screen where there is room to type them.
//!
//! ## Why a model and not just drawing
//!
//! Which fields are live depends on the mode — punctuation and numbers are
//! greyed out in quote, zen and custom, exactly as on the site — and that is a
//! rule, not a drawing. Keeping it in a `Vec<Field>` that both the screen and
//! the app walk means "is this field available" has exactly one answer.

use serde::{Deserialize, Serialize};

use super::{Difficulty, Mode};

/// Steps `by` places around a list of `len` options, starting at `current`.
///
/// Wrapping rather than clamping, because a *value* is a ring: a difficulty that
/// stopped at "master" would make the bottom of the list unreachable by pressing
/// down, and one that stopped at "normal" would make the top unreachable. The
/// selection between fields does clamp — that one has a left and a right.
pub fn step(current: Option<usize>, by: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let from = current.unwrap_or(0) as isize;
    (from + by).rem_euclid(len as isize) as usize
}

/// "on" or "off", the way the website writes a toggle's value.
pub fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_owned()
}

/// One selectable field of the bar.
///
/// These are the website's own fields, in the order its three cards hold them:
/// the toggles on the left, the mode in the middle, and whatever the mode's
/// length is called on the right. `TimeCustom` and `WordsCustom` are the wrench
/// button rather than a preset — it opens the input window, which is a different
/// action from stepping to the next preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Punctuation,
    Numbers,
    Mode,
    /// One of the preset durations.
    Time,
    /// The wrench: set a duration of any length.
    TimeCustom,
    /// One of the preset word counts.
    Words,
    /// The wrench: set a word count of any size.
    WordsCustom,
    QuoteLength,
    /// A passage of the user's own.
    CustomText,
}

impl Field {
    /// Whether picking this field needs the input window rather than a step.
    ///
    /// A preset cycles; a wrench opens a dialog. Treating them as the same thing
    /// is how a "custom" button ends up cycling to nothing.
    pub fn needs_input(self) -> bool {
        matches!(
            self,
            Self::TimeCustom | Self::WordsCustom | Self::CustomText
        )
    }
}

/// The word-list sizes, in the order the settings browser's columns go.
///
/// Re-exported from [`crate::words::variants`] rather than duplicated, so the
/// browser's columns and the picker cannot drift apart.
pub use crate::words::variants::SIZES as variants_sizes;

/// The modes, in the website's order.
pub const MODES: [Mode; 5] = [
    Mode::Time,
    Mode::Words,
    Mode::Quote,
    Mode::Zen,
    Mode::Custom,
];

/// Test lengths, in seconds. The website's four, exactly: anything else is what
/// the wrench is for, and a longer list would make "custom" mean something
/// different here than it does there.
pub const TIMES: [u32; 4] = [15, 30, 60, 120];

/// Test lengths, in words, in the website's order.
pub const WORD_COUNTS: [u32; 4] = [10, 25, 50, 100];

/// The difficulties, in the website's order.
pub const DIFFICULTIES: [Difficulty; 3] =
    [Difficulty::Normal, Difficulty::Expert, Difficulty::Master];

/// The quote lengths, in the website's order. `Thicc` is their word for the
/// longest bucket and it is worth keeping: it is what the button says, and
/// calling it "very long" would make this bar disagree with the one it copies.
pub const QUOTE_LENGTHS: [QuoteLength; 5] = [
    QuoteLength::All,
    QuoteLength::Short,
    QuoteLength::Medium,
    QuoteLength::Long,
    QuoteLength::Thicc,
];

/// How long a quote is allowed to be.
///
/// The website buckets quotes by character count into four groups and lets the
/// user pick a bucket, so these are the same four.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteLength {
    #[default]
    All,
    Short,
    Medium,
    Long,
    Thicc,
}

impl QuoteLength {
    /// The website's label, and the config spelling.
    ///
    /// Fixed, whatever the interface language — the website's own words, and what
    /// a config file says. The translated name is [`Self::key`], which keeps
    /// "thicc" in every language, because the website's word for the longest
    /// bucket is the joke and translating it loses it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Long => "long",
            Self::Thicc => "thicc",
        }
    }

    /// The name as the interface shows it.
    pub fn key(self) -> crate::i18n::Key {
        match self {
            Self::All => crate::i18n::Key::QuoteAll,
            Self::Short => crate::i18n::Key::QuoteShort,
            Self::Medium => crate::i18n::Key::QuoteMedium,
            Self::Long => crate::i18n::Key::QuoteLong,
            Self::Thicc => crate::i18n::Key::QuoteThicc,
        }
    }

    /// The character-count range this bucket covers, inclusive.
    ///
    /// The bounds are the website's, from the `groups` array at the top of every
    /// quote file. `All` is the whole range rather than a span, so a length test
    /// does not have to special-case it.
    pub fn bounds(self) -> (usize, usize) {
        match self {
            Self::All => (0, usize::MAX),
            Self::Short => (0, 100),
            Self::Medium => (101, 300),
            Self::Long => (301, 600),
            Self::Thicc => (601, usize::MAX),
        }
    }

    /// Whether a quote of this many characters is in the bucket.
    pub fn accepts(self, length: usize) -> bool {
        let (low, high) = self.bounds();
        length >= low && length <= high
    }
}

impl Mode {
    /// The button label on the website.
    pub fn bar_label(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Words => "words",
            Self::Quote => "quote",
            Self::Zen => "zen",
            Self::Custom => "custom",
        }
    }

    /// Whether a length applies, and what it counts.
    ///
    /// A timed test counts seconds, a word-count test counts words, a passage
    /// counts its own length and so has nothing to choose, and zen is endless.
    pub fn length_unit(self) -> Option<LengthUnit> {
        match self {
            Self::Time => Some(LengthUnit::Seconds),
            Self::Words | Self::Custom => Some(LengthUnit::Words),
            Self::Quote | Self::Zen => None,
        }
    }
}

/// What the bar's length field is counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Seconds,
    Words,
}

impl LengthUnit {
    /// Renders a length the way the website writes it.
    pub fn render(self, value: u32) -> String {
        match self {
            Self::Seconds => format!("{value}s"),
            Self::Words => format!("{value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_lengths_are_the_websites_buckets() {
        // The bounds are shared between every quote file, so a quote of 250
        // characters is "medium" on this client and on the site.
        assert!(QuoteLength::Short.accepts(64));
        assert!(!QuoteLength::Short.accepts(101));
        assert!(QuoteLength::Medium.accepts(101));
        assert!(!QuoteLength::Medium.accepts(301));
        assert!(QuoteLength::Long.accepts(300 + 1));
        assert!(QuoteLength::Thicc.accepts(601));
        assert!(QuoteLength::All.accepts(10_000));
    }

    #[test]
    fn the_buckets_do_not_overlap_or_leave_gaps() {
        let lengths = [0usize, 1, 100, 101, 300, 301, 600, 601, 5_000];
        for length in lengths {
            let matching: Vec<QuoteLength> = QUOTE_LENGTHS
                .iter()
                .copied()
                .filter(|q| *q != QuoteLength::All && q.accepts(length))
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "{length} characters matched {matching:?}"
            );
        }
    }

    #[test]
    fn the_longest_bucket_is_called_thicc() {
        // It is the website's word. Replacing it would make this bar disagree
        // with the one it is copying, and the user is looking at both.
        assert_eq!(QuoteLength::Thicc.as_str(), "thicc");
    }

    #[test]
    fn only_a_timed_test_counts_seconds() {
        assert_eq!(Mode::Time.length_unit(), Some(LengthUnit::Seconds));
        assert_eq!(Mode::Words.length_unit(), Some(LengthUnit::Words));
        assert_eq!(Mode::Quote.length_unit(), None);
        assert_eq!(Mode::Zen.length_unit(), None);
    }

    /// A value is a ring, so the last entry reaches the first by pressing on.
    #[test]
    fn a_value_cycles_in_both_directions() {
        assert_eq!(step(Some(0), -1, 5), 4, "up from the first is the last");
        assert_eq!(step(Some(4), 1, 5), 0, "down from the last is the first");
        assert_eq!(step(Some(2), 1, 5), 3);
        assert_eq!(step(Some(2), -1, 5), 1);
        // A step bigger than the list still lands on it.
        assert_eq!(step(Some(0), 7, 5), 2);
        assert_eq!(step(Some(0), -7, 5), 3);
    }

    /// A value not in the list starts from the beginning rather than panicking.
    #[test]
    fn a_value_the_list_does_not_have_starts_at_the_first() {
        assert_eq!(step(None, 1, 5), 1);
        assert_eq!(step(None, 0, 5), 0);
    }

    #[test]
    fn an_empty_list_cannot_be_stepped_through() {
        assert_eq!(step(Some(0), 1, 0), 0);
    }

    #[test]
    fn a_toggle_reads_the_way_the_website_writes_it() {
        assert_eq!(on_off(true), "on");
        assert_eq!(on_off(false), "off");
    }

    #[test]
    fn lengths_are_written_the_way_the_website_writes_them() {
        assert_eq!(LengthUnit::Seconds.render(30), "30s");
        assert_eq!(LengthUnit::Words.render(25), "25");
    }
}
