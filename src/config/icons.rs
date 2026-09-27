//! The bar's glyphs, and how to change them.
//!
//! The built-in set is a table of Nerd Font names and codepoints in
//! [`crate::screens::topbar::icon`], and every one of the eight is a fact that can be
//! checked — the codepoints were read out of the upstream cmap tables rather than
//! written from memory, after four of the first eight turned out to be wrong. But a
//! wrong glyph is not the only reason a user might want a different one: a terminal
//! whose font lacks them draws boxes, and a terminal configured to treat
//! ambiguous-width characters as wide — which every Nerd Font glyph is, they all live
//! in the Private Use Area — draws each of them **two** columns wide, which pushes the
//! bar's frame out of shape.
//!
//! So the glyphs are a setting, and a setting can be wrong for a machine in a way the
//! author cannot see. This is that escape hatch.
//!
//! # The fallback chain
//!
//! For each of the eight, in order:
//!
//! 1. this file's key, if it is present and not empty;
//! 2. the built-in glyph, if `enabled`;
//! 3. nothing.
//!
//! An empty string is a real answer — it means "no glyph for this one" — so
//! `punctuation = ""` removes the punctuation icon while leaving the other seven
//! alone. `enabled = false` removes all eight at once, which is the setting to reach
//! for on a terminal that renders them double-width.
//!
//! ```toml
//! [icons]
//! enabled = true
//! punctuation = "@"     # any single character; the built-in Nerd Font glyph otherwise
//! numbers = ""          # no icon for this one
//! zen = ""              # ditto
//! ```
//!
//! # Why the length buttons have none
//!
//! `15 30 60 120` is a row of numbers, and a glyph in front of each of them would be
//! five glyphs saying nothing. There is no key for them here and there is no built-in
//! glyph for them, on purpose.

use serde::{Deserialize, Serialize};

/// Which bar item a glyph belongs to.
///
/// A function rather than a field name, because the keys are the built-in Nerd Font
/// names — `punctuation`, `time`, `zen` — rather than this enum's variants, so that
/// renaming an internal type never silently changes a config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Item {
    Punctuation,
    Numbers,
    Time,
    Words,
    Quote,
    Zen,
    Custom,
    Other,
}

impl Item {
    /// All eight, in the order the bar shows them.
    pub const ALL: [Item; 8] = [
        Item::Punctuation,
        Item::Numbers,
        Item::Time,
        Item::Words,
        Item::Quote,
        Item::Zen,
        Item::Custom,
        Item::Other,
    ];

    /// The key this item has in `config.toml`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Punctuation => "punctuation",
            Self::Numbers => "numbers",
            Self::Time => "time",
            Self::Words => "words",
            Self::Quote => "quote",
            Self::Zen => "zen",
            Self::Custom => "custom",
            Self::Other => "other",
        }
    }

    /// The item a `config.toml` key names, or `None` if it names nothing.
    ///
    /// Checked against the real key list rather than parsed from the field name, so a
    /// key that does not exist is a rejected value rather than a silently ignored one.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.key() == key)
    }

    /// The item a test mode's button is.
    ///
    /// The five modes are five of the eight items, and the mapping is the same one the
    /// built-in table uses. It lives here rather than in the table so that a mode and
    /// a config key cannot come to mean different things: they are asked about in the
    /// same breath when a button is built.
    pub fn from_mode(mode: crate::config::Mode) -> Self {
        match mode {
            crate::config::Mode::Time => Self::Time,
            crate::config::Mode::Words => Self::Words,
            crate::config::Mode::Quote => Self::Quote,
            crate::config::Mode::Zen => Self::Zen,
            crate::config::Mode::Custom => Self::Custom,
        }
    }

    /// The Nerd Font **name** of this item's built-in glyph.
    ///
    /// A name and not a character, because a name is the thing that is written down
    /// and the thing a person would want to change. It used to return both, and only
    /// the character was ever read — so renaming an icon did nothing at all, with no
    /// error and no warning, because the name was a caption on a value rather than
    /// the value. The name is now the value: [`glyph`] resolves it.
    pub fn built_in(self) -> &'static str {
        match self {
            Self::Punctuation => "md-dog",
            Self::Numbers => "fa-hashtag",
            Self::Time => "md-clock-time-two",
            Self::Words => "fa-font",
            Self::Quote => "fa-quote_left",
            Self::Zen => "fa-mountain",
            Self::Custom => "fa-wrench",
            Self::Other => "fa-screwdriver_wrench",
        }
    }

    /// The character this item's built-in name resolves to.
    pub fn glyph(self) -> &'static str {
        resolve(self.built_in()).unwrap_or("")
    }
}

/// The character a Nerd Font name refers to.
///
/// The one place a name becomes a glyph, so `config.toml` and the built-in table go
/// through exactly the same lookup and cannot disagree. A name that is not in the
/// table resolves to nothing rather than to itself: a name is a name, and drawing the
/// eight letters `md-` because one of them was misspelled is worse than drawing
/// nothing.
///
/// The table is the one in [`crate::screens::topbar::icon`], where the codepoints live
/// beside the names they were read from.
pub fn resolve(name: &str) -> Option<&'static str> {
    use crate::screens::topbar::icon;
    icon::TABLE
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, glyph)| *glyph)
}

/// Turns whatever a config value says into a glyph.
///
/// A value that names a glyph is resolved through [`resolve`], so `zen = "fa-mountain"`
/// and `zen = "󰀁"` are the same request written two ways. A single pasted character
/// is taken as itself, and is *not* looked up: the user pasted it, so that is what they
/// meant, even if it happens to spell a name.
///
/// Anything else is **nothing**: a misspelled name, or a run of several characters.
/// Nothing, because the alternative is drawing the six letters `nope` where the user
/// asked for an icon, and a missing icon is a far smaller problem than a row of text
/// that was not asked for.
pub fn as_glyph(value: &str) -> &str {
    if value.chars().count() == 1 {
        return value;
    }
    resolve(value).unwrap_or("")
}

/// The user's glyphs, or the built-in ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Icons {
    /// Whether the built-in glyphs are used at all.
    ///
    /// `false` is the setting for a terminal that draws these two columns wide, which
    /// every Nerd Font glyph is a candidate for: they all live in the Private Use Area
    /// and the area is East Asian *ambiguous*, so a terminal set to treat ambiguous
    /// characters as wide renders each glyph twice as wide as the bar reserved for it
    /// and the frame comes out ragged.
    pub enabled: bool,
    /// A replacement for the built-in glyph, or `""` for none.
    pub punctuation: Option<String>,
    pub numbers: Option<String>,
    pub time: Option<String>,
    pub words: Option<String>,
    pub quote: Option<String>,
    pub zen: Option<String>,
    pub custom: Option<String>,
    pub other: Option<String>,
}

impl Icons {
    /// What to draw for an item: the config's key, the built-in glyph, or nothing.
    ///
    /// One function, so the whole chain is in one place and the bar cannot implement
    /// a different order of it. An override that is present but empty is an answer —
    /// "no glyph for this one" — which is why this is not
    /// `filter(|s| !s.is_empty())`.
    ///
    /// An override is put through [`as_glyph`], so it may be a Nerd Font name or a
    /// pasted character, and either way what the user wrote is what gets drawn.
    pub fn glyph(&self, item: Item) -> &str {
        let set = match item {
            Item::Punctuation => &self.punctuation,
            Item::Numbers => &self.numbers,
            Item::Time => &self.time,
            Item::Words => &self.words,
            Item::Quote => &self.quote,
            Item::Zen => &self.zen,
            Item::Custom => &self.custom,
            Item::Other => &self.other,
        };
        if let Some(glyph) = set {
            return as_glyph(glyph);
        }
        if self.enabled {
            item.glyph()
        } else {
            ""
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in set, which is what a config that says nothing gets.
    fn built_in() -> Icons {
        Icons {
            enabled: true,
            ..Icons::default()
        }
    }

    /// The whole fallback chain, in the order the module doc gives it.
    #[test]
    fn the_chain_is_config_key_then_built_in_then_nothing() {
        // 1. Nothing set: the built-in glyph.
        assert_eq!(built_in().glyph(Item::Zen), Item::Zen.glyph());

        // 2. A key set: that, whatever it is.
        let custom = Icons {
            enabled: true,
            zen: Some("R".to_owned()),
            ..Icons::default()
        };
        assert_eq!(custom.glyph(Item::Zen), "R");
        // And the others are untouched — one key is one item.
        assert_eq!(custom.glyph(Item::Time), Item::Time.glyph());

        // 3. `enabled = false`: nothing, even with a key set, because the key is the
        //    first fallback and not the only one.
        let off = Icons {
            enabled: false,
            zen: Some("R".to_owned()),
            ..Icons::default()
        };
        assert_eq!(off.glyph(Item::Zen), "R");
        assert_eq!(off.glyph(Item::Time), "");
    }

    /// A built-in name that is not in the table resolves to nothing, and this test is
    /// what says so.
    ///
    /// This is the trap that was just walked into. The name and the character used to
    /// be a pair and only the character was read, so changing `"md-dog"` to anything
    /// else changed nothing on screen — no error, no warning, no empty icon, just a
    /// different caption on the same glyph. Now the name *is* the value, so a name
    /// that does not resolve means no icon, which is visible, and this test fails.
    #[test]
    fn every_built_in_name_resolves_to_a_distinct_glyph() {
        let mut seen = std::collections::BTreeSet::new();
        for item in Item::ALL {
            let name = item.built_in();
            let glyph = item.glyph();
            assert!(
                !glyph.is_empty(),
                "{item:?} names {name:?}, which is not in the glyph table — \
                 the icon will be blank"
            );
            assert_eq!(glyph.chars().count(), 1, "{name:?} is not one character");
            assert!(
                seen.insert(glyph.to_owned()),
                "{item:?} and something else are both {name:?}"
            );
        }
    }

    /// A name in `config.toml` is resolved, so a user can write the same thing the
    /// built-in table says instead of hunting for a codepoint.
    #[test]
    fn a_config_value_can_be_a_name_or_a_character() {
        let by_name = Icons {
            enabled: true,
            zen: Some("fa-wrench".to_owned()),
            ..Icons::default()
        };
        assert_eq!(by_name.glyph(Item::Zen), Item::Custom.glyph());

        let by_character = Icons {
            enabled: true,
            zen: Some(Item::Custom.glyph().to_owned()),
            ..Icons::default()
        };
        assert_eq!(by_character.glyph(Item::Zen), Item::Custom.glyph());

        // And something that is neither a name nor one character draws nothing.
        // Drawing the letters would put six characters where the bar reserved one, and
        // push the cell — and the frame — out of shape.
        let neither = Icons {
            enabled: true,
            zen: Some("!!".to_owned()),
            ..Icons::default()
        };
        assert_eq!(neither.glyph(Item::Zen), "");
    }

    /// A pasted character is never looked up, even if it happens to spell a name — the
    /// user pasted it, so that is what they meant.
    #[test]
    fn a_pasted_character_is_taken_literally() {
        assert_eq!(as_glyph("R"), "R");
        assert_eq!(as_glyph("󰀁"), "󰀁");
        assert_eq!(
            as_glyph("fa-wrench"),
            resolve("fa-wrench").expect("it resolves")
        );
    }

    /// A misspelled name is no glyph rather than the letters of the name.
    #[test]
    fn a_name_that_does_not_resolve_is_no_icon() {
        assert_eq!(as_glyph("not-a-name"), "");
        assert_eq!(as_glyph("fa-wrenc"), "");
        assert_eq!(as_glyph("ab"), "");
    }

    /// An empty string is an answer — "no glyph for this one" — and not a way of
    /// saying "use the built-in".
    ///
    /// This is the one that a `filter(|s| !s.is_empty())` would silently get wrong,
    /// and it is the difference between a user removing one icon and a user failing
    /// to remove it.
    #[test]
    fn an_empty_string_means_no_icon_not_the_built_in_one() {
        let one_off = Icons {
            enabled: true,
            zen: Some(String::new()),
            ..Icons::default()
        };
        assert_eq!(one_off.glyph(Item::Zen), "");
        assert_eq!(one_off.glyph(Item::Time), Item::Time.glyph());
    }

    /// `enabled = false` with no keys is the setting for a terminal that draws these
    /// glyphs two columns wide, and it has to reach every one of the eight.
    #[test]
    fn enabled_false_empties_all_eight() {
        let off = Icons {
            enabled: false,
            ..Icons::default()
        };
        for item in Item::ALL {
            assert_eq!(off.glyph(item), "", "{item:?} survived");
        }
    }

    /// Every item's key is the one the documentation says it is, and no two share one.
    #[test]
    fn the_keys_are_the_documented_ones() {
        let want = [
            (Item::Punctuation, "punctuation"),
            (Item::Numbers, "numbers"),
            (Item::Time, "time"),
            (Item::Words, "words"),
            (Item::Quote, "quote"),
            (Item::Zen, "zen"),
            (Item::Custom, "custom"),
            (Item::Other, "other"),
        ];
        for (item, key) in want {
            assert_eq!(item.key(), key);
            assert_eq!(
                Item::from_key(key),
                Some(item),
                "{key} does not name {item:?}"
            );
        }
        assert_eq!(Item::from_key("nonsense"), None);
        let keys: std::collections::BTreeSet<&str> =
            Item::ALL.into_iter().map(|i| i.key()).collect();
        assert_eq!(keys.len(), Item::ALL.len(), "two items share a key");
    }

    /// A mode is one of the eight, and all five map somewhere different.
    #[test]
    fn the_five_modes_are_five_different_items() {
        use crate::config::Mode;
        let mapped: std::collections::BTreeSet<Item> = [
            Mode::Time,
            Mode::Words,
            Mode::Quote,
            Mode::Zen,
            Mode::Custom,
        ]
        .into_iter()
        .map(Item::from_mode)
        .collect();
        assert_eq!(mapped.len(), 5, "two modes share a glyph");
    }

    /// A config file that says nothing about icons gets the built-in ones, so an
    /// existing file is unchanged by this feature existing.
    #[test]
    fn a_config_without_an_icons_table_gets_the_built_in_ones() {
        let parsed: crate::config::Config = toml::from_str(
            r#"
            version = 1
            ape_key = ""
            api_url = "https://api.monkeytype.com"
            theme = "auto"
            submit_results = false
            ui_language = "en"

            [test]
            mode = "time"
            punctuation = false
            numbers = false
            difficulty = "normal"
            language = "english"
            time = 30
            words = 25
            quote_length = "all"
            blind = false
            custom_text = []
            "#,
        )
        .expect("a config with no icons table");
        assert!(parsed.icons.enabled, "the built-ins are off by default");
        assert_eq!(parsed.icons.glyph(Item::Zen), Item::Zen.glyph());
    }

    /// And a config that sets them is read, including the empty-string case that has
    /// to survive a round trip through the file.
    #[test]
    fn a_config_can_set_and_clear_them() {
        let parsed: crate::config::Config = toml::from_str(
            r#"
            version = 1
            ape_key = ""
            api_url = "https://api.monkeytype.com"
            theme = "auto"
            submit_results = false
            ui_language = "en"

            [icons]
            enabled = false
            zen = ""
            time = "*"

            [test]
            mode = "time"
            punctuation = false
            numbers = false
            difficulty = "normal"
            language = "english"
            time = 30
            words = 25
            quote_length = "all"
            blind = false
            custom_text = []
            "#,
        )
        .expect("a config with an icons table");
        assert!(!parsed.icons.enabled);
        assert_eq!(parsed.icons.glyph(Item::Time), "*", "a key is not read");
        assert_eq!(
            parsed.icons.glyph(Item::Zen),
            "",
            "an empty key is not read"
        );
        assert_eq!(
            parsed.icons.glyph(Item::Quote),
            "",
            "enabled=false is not read"
        );
    }

    /// A key that is not one of the eight is rejected rather than ignored.
    ///
    /// The config denies unknown fields, so a typo is an error at start-up with the
    /// key named — which is the difference between a user fixing a typo and a user
    /// wondering why their icon never changed.
    #[test]
    fn a_misspelled_key_is_an_error_and_not_a_silent_no_op() {
        let parsed = toml::from_str::<crate::config::Config>(
            r#"
            version = 1
            ape_key = ""
            api_url = "https://api.monkeytype.com"
            theme = "auto"
            submit_results = false
            ui_language = "en"

            [icons]
            zne = "R"

            [test]
            mode = "time"
            punctuation = false
            numbers = false
            difficulty = "normal"
            language = "english"
            time = 30
            words = 25
            quote_length = "all"
            blind = false
            custom_text = []
            "#,
        );
        let err = parsed.expect_err("a misspelled icon key should not parse");
        assert!(
            err.to_string().contains("zne"),
            "the error does not name the key: {err}"
        );
    }
}
