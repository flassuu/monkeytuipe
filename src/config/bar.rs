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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// What kind of test: time, words, quote, zen or custom.
    Mode,
    /// How long: seconds for a timed test, words for a word-count or passage one.
    Length,
    /// Quote length, which replaces [`Field::Length`] in quote mode.
    QuoteLength,
    Punctuation,
    Numbers,
    Difficulty,
    /// Free text: the passage to type.
    CustomText,
    Language,
    Blind,
}

impl Field {
    /// The bar's own label, as the website writes it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mode => "",
            Self::Length => "",
            Self::QuoteLength => "quote",
            Self::Punctuation => "punctuation",
            Self::Numbers => "numbers",
            Self::Difficulty => "difficulty",
            Self::CustomText => "custom",
            Self::Language => "language",
            Self::Blind => "blind",
        }
    }

    /// Whether this field has a value of its own to show, or only a name.
    ///
    /// The mode and the length are shown as the value alone — `time 30s`, not
    /// `mode: time` — because on the website they are the first two buttons and
    /// labelling them would only add noise.
    pub fn is_value_only(self) -> bool {
        matches!(self, Self::Mode | Self::Length)
    }
}

/// The modes, in the website's order.
pub const MODES: [Mode; 5] = [
    Mode::Time,
    Mode::Words,
    Mode::Quote,
    Mode::Zen,
    Mode::Custom,
];

/// Test lengths, in seconds, in the website's order plus two that a terminal
/// test can actually be run at: a 15-second test over a 200-word list is 20 words
/// and a 5-minute one is a chart you can read.
pub const TIMES: [u32; 6] = [15, 30, 60, 120, 180, 300];

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
    /// The website's label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Long => "long",
            Self::Thicc => "thicc",
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

/// Where the bar is: which field is selected, and whether it has the keyboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bar {
    selected: usize,
    focused: bool,
}

impl Bar {
    /// A bar that is on screen but not selected, so nothing on it reacts to a key.
    pub fn idle() -> Self {
        Self::default()
    }

    /// A bar with the keyboard, on the first field.
    pub fn focused() -> Self {
        Self {
            selected: 0,
            focused: true,
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The index of the selected field within [`Self::fields`].
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Moves the selection, clamping rather than wrapping.
    ///
    /// Clamping rather than wrapping is the point: the bar has an order, and a
    /// selection that jumps from the last field back to the first is a selection
    /// you have lost.
    pub fn move_selection(&mut self, by: isize, len: usize) {
        if len == 0 {
            return;
        }
        let last = len as isize - 1;
        let next = (self.selected as isize + by).clamp(0, last);
        self.selected = next as usize;
    }

    /// Selects a field by index, ignoring anything out of range.
    pub fn select(&mut self, index: usize) {
        self.selected = index;
    }

    pub fn focus(&mut self) {
        self.focused = true;
    }

    pub fn blur(&mut self) {
        self.focused = false;
    }

    /// The fields that apply to the current mode, in the website's order.
    ///
    /// This is the rule that makes the bar look like the site: the mode comes
    /// first, then whatever stands in for a length, and only then the modifiers.
    /// Punctuation, numbers, difficulty and language are all dropped in the modes
    /// where they would have nothing to act on, rather than being shown disabled —
    /// the site greys them out, but a bar is too short for a greyed-out control to
    /// read as anything but a mistake.
    pub fn fields(mode: Mode) -> Vec<Field> {
        let mut fields = vec![Field::Mode];
        match mode {
            Mode::Time | Mode::Words => fields.push(Field::Length),
            Mode::Quote => fields.push(Field::QuoteLength),
            Mode::Custom => fields.push(Field::CustomText),
            Mode::Zen => {}
        }
        match mode {
            Mode::Time | Mode::Words => {
                fields.extend([
                    Field::Punctuation,
                    Field::Numbers,
                    Field::Difficulty,
                    Field::Language,
                ]);
            }
            // A passage is already punctuated and is not made of the generator's
            // words, and zen has no target to be right or wrong about.
            Mode::Quote | Mode::Custom | Mode::Zen => {}
        }
        fields.push(Field::Blind);
        fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_test_offers_everything_the_generator_can_do() {
        let fields = Bar::fields(Mode::Words);
        assert_eq!(
            fields,
            [
                Field::Mode,
                Field::Length,
                Field::Punctuation,
                Field::Numbers,
                Field::Difficulty,
                Field::Language,
                Field::Blind
            ]
        );
    }

    /// The site's rule: no punctuation or language buttons in quote mode.
    #[test]
    fn a_quote_offers_only_its_length() {
        assert_eq!(
            Bar::fields(Mode::Quote),
            [Field::Mode, Field::QuoteLength, Field::Blind]
        );
    }

    #[test]
    fn a_zen_test_offers_almost_nothing() {
        // No length, because zen does not end, and nothing to modify, because
        // there is no target.
        assert_eq!(Bar::fields(Mode::Zen), [Field::Mode, Field::Blind]);
    }

    #[test]
    fn a_custom_test_offers_its_text_and_nothing_else() {
        assert_eq!(
            Bar::fields(Mode::Custom),
            [Field::Mode, Field::CustomText, Field::Blind]
        );
    }

    #[test]
    fn the_mode_is_always_the_first_field() {
        for mode in MODES {
            assert_eq!(
                Bar::fields(mode).first(),
                Some(&Field::Mode),
                "{mode:?} does not start with the mode"
            );
        }
    }

    #[test]
    fn blind_is_always_the_last_field() {
        for mode in MODES {
            assert_eq!(
                Bar::fields(mode).last(),
                Some(&Field::Blind),
                "{mode:?} does not end with blind"
            );
        }
    }

    /// The selection must stay inside the bar as the mode changes the bar's
    /// shape, or changing mode leaves nothing selected.
    #[test]
    fn a_selection_never_points_past_the_end() {
        let mut bar = Bar::focused();
        for _ in 0..20 {
            bar.move_selection(1, Bar::fields(Mode::Time).len());
        }
        assert_eq!(bar.selected(), Bar::fields(Mode::Time).len() - 1);
    }

    #[test]
    fn the_selection_stops_at_both_ends() {
        let mut bar = Bar::focused();
        for _ in 0..5 {
            bar.move_selection(-1, 7);
        }
        assert_eq!(bar.selected(), 0, "it went off the left edge");
    }

    #[test]
    fn an_empty_bar_cannot_be_moved() {
        let mut bar = Bar::focused();
        bar.move_selection(1, 0);
        assert_eq!(bar.selected(), 0);
    }

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
