//! Themes.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// A theme made of nothing but the terminal's own colours.
///
/// This is what `auto` is, and the whole point is that it contains **no colour
/// anybody chose**. Two kinds of entry, both of which the terminal resolves from its
/// own configuration:
///
/// - `Color::Reset` — the terminal's default foreground or background. Not "black" or
///   "white": whatever the user's profile says, which is the thing a theme named after
///   the terminal ought to respect.
/// - the sixteen palette entries — `Color::Red` is SGR 31, which is the terminal's own
///   idea of red. A user who has remapped their palette gets their red.
///
/// `auto` used to fall back to a bundled theme when the terminal did not answer the
/// colour query, and that fallback was the monkeytype theme — so on every terminal
/// that did not answer, `auto` *was* `monkeytype`, and nothing said so. A setting
/// called "take my terminal's colours" that quietly hands you someone else's is worse
/// than no setting, because it looks like it worked.
///
/// There is no fallback here. If the query answered, [`crate::terminal::palette::build`]
/// replaces these with the exact RGB values and mixes the in-between shades. If it did
/// not, every colour below is still the terminal's, and the two cases differ in
/// precision rather than in kind.
pub fn terminal_default() -> Theme {
    Theme {
        background: Color::Reset,
        surface: Color::Reset,
        foreground: Color::Reset,
        accent: Color::Yellow,
        correct: Color::Green,
        incorrect: Color::Red,
        extra: Color::Blue,
        // The terminal's own "bright black", which is the grey every palette agrees
        // is a grey — rather than a mix of two colours we would have had to guess.
        muted: Color::DarkGray,
    }
}

/// A named theme, serialisable as a bare string in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    /// Whatever the terminal says about itself, in preference to a guess.
    ///
    /// One setting, not two, and that is a change of mind. There used to be `auto`
    /// — pick a bundled theme by colour depth and background — *and* `terminal`,
    /// which asks the terminal for its own colours. They are the same intent
    /// ("use this machine's colours") reached two ways, and the picker showed both
    /// with an annotation on each: `auto (monkeytype)` and `terminal (no reply)`.
    /// Two rows saying one thing, in a list whose whole purpose is to be scanned.
    ///
    /// So `auto` asks first and guesses second. A terminal that answers — `OSC
    /// 10`, `OSC 11` and the sixteen ANSI entries — gives its exact colours; one
    /// that does not falls back to the bundled guess. Either way the row says
    /// `auto`, because that is the only thing the user chose and the only thing
    /// they can change.
    ///
    /// The default, because it is the only setting that is right on a machine
    /// nobody has looked at.
    #[default]
    #[serde(alias = "terminal")]
    Auto,
    Monkeytype,
    Gruvbox,
    Nord,
    Dracula,
    SolarizedDark,
    SolarizedLight,
    Catppuccin,
    TokyoNight,
    RosePine,
    Kanagawa,
    Everforest,
    GruvboxLight,
    OneHalfDark,
    OneHalfLight,
    Tomorrow,
    Material,
    Zenburn,
    BuiltinDark,
}

impl ThemeName {
    pub const ALL: [Self; 19] = [
        Self::Auto,
        Self::Monkeytype,
        Self::Gruvbox,
        Self::Nord,
        Self::Dracula,
        Self::SolarizedDark,
        Self::SolarizedLight,
        Self::Catppuccin,
        Self::TokyoNight,
        Self::RosePine,
        Self::Kanagawa,
        Self::Everforest,
        Self::GruvboxLight,
        Self::OneHalfDark,
        Self::OneHalfLight,
        Self::Tomorrow,
        Self::Material,
        Self::Zenburn,
        Self::BuiltinDark,
    ];

    /// The label shown in the settings screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Monkeytype => "monkeytype",
            Self::Gruvbox => "gruvbox",
            Self::Nord => "nord",
            Self::Dracula => "dracula",
            Self::SolarizedDark => "solarized dark",
            Self::SolarizedLight => "solarized light",
            Self::Catppuccin => "catppuccin",
            Self::TokyoNight => "tokyo night",
            Self::RosePine => "rose pine",
            Self::Kanagawa => "kanagawa",
            Self::Everforest => "everforest",
            Self::GruvboxLight => "gruvbox light",
            Self::OneHalfDark => "one half dark",
            Self::OneHalfLight => "one half light",
            Self::Tomorrow => "tomorrow",
            Self::Material => "material",
            Self::Zenburn => "zenburn",
            Self::BuiltinDark => "builtin dark",
        }
    }

    /// The theme `auto` settles on for a given terminal.
    ///
    /// The background is the only thing that can go badly wrong — a dark theme on
    /// a light terminal is unreadable — so that is what the choice is based on.
    /// Everything else is left to the user's taste, because there is no way to
    /// tell which of eighteen dark themes somebody would have chosen.
    pub fn for_terminal(self, terminal: &super::terminal::Terminal) -> Theme {
        match self {
            // Not a palette: a question. The terminal is asked what its colours are,
            // and where it does not know, the answer is "whatever you would have used
            // anyway" rather than somebody else's dark grey.
            Self::Auto => {
                let reported = crate::terminal::palette::reported();
                let mut theme = crate::terminal::palette::build(&reported, &terminal_default());
                terminal.adapt(&mut theme);
                theme
            }
            other => other.resolve(),
        }
    }

    /// The bundled theme `auto` falls back to when the terminal did not answer.
    ///
    /// The background is the only thing that can go badly wrong — a dark theme on
    /// a light terminal is unreadable — so that is what the choice is based on.
    /// Everything else is left to the user's taste, because there is no way to
    /// tell which of eighteen dark themes somebody would have chosen.
    pub fn auto_for(terminal: &super::terminal::Terminal) -> Self {
        if terminal.is_light() {
            Self::SolarizedLight
        } else {
            Self::Monkeytype
        }
    }

    /// Whether the theme is meant for a light terminal.
    ///
    /// The automatic setting resolves against whatever the terminal reported, so
    /// a light terminal never ends up with a dark theme — which is not a matter of
    /// taste but of being able to read the words.
    pub fn is_light(self) -> bool {
        match self {
            Self::SolarizedLight | Self::GruvboxLight | Self::OneHalfLight => true,
            // Decided by what the terminal said, then by what the fallback would
            // be. A theme called `auto` on a light terminal is a light theme, and
            // `COLORFGBG` says so even when OSC did not answer.
            Self::Auto => match crate::terminal::palette::reported().background {
                Some(background) => crate::terminal::palette::is_light(background),
                None => false,
            },
            _ => false,
        }
    }

    /// Whether a theme name is a real theme rather than the automatic setting.
    ///
    /// Used by the settings screen to label the current value: showing `auto` in
    /// the theme row is honest, but listing it among the palettes is not.
    pub fn name_is_explicit(self) -> bool {
        self != Self::Auto
    }

    /// Whether this theme is a question about the machine rather than a palette.
    ///
    /// Only `auto` is, and the settings screen's picker shows it apart from the
    /// colours for exactly that reason: it resolves to something else rather than
    /// to a palette of its own.
    pub fn is_automatic(self) -> bool {
        self == Self::Auto
    }

    /// Every theme a user can pick, `auto` aside.
    ///
    /// The settings screen's picker lists these rather than [`Self::ALL`], because
    /// `auto` is not a theme to try on — it is the absence of a choice.
    pub fn choices() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|name| !name.is_automatic())
    }

    /// The themes that are questions about the machine, for the picker to show
    /// above the palettes.
    pub fn automatic() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|name| name.is_automatic())
    }

    /// Steps through the themes in a direction.
    ///
    /// A direction rather than just a "next", because the settings screen has two
    /// arrows and they have to go opposite ways. An index that is not in the list
    /// starts from the beginning rather than panicking: a theme name read from a
    /// hand-edited config file can be anything.
    pub fn step(self, by: isize) -> Self {
        let len = Self::ALL.len() as isize;
        let index = Self::ALL
            .iter()
            .position(|t| *t == self)
            .map_or(0, |i| i as isize);
        Self::ALL[((index + by).rem_euclid(len)) as usize]
    }

    /// The next theme, wrapping round.
    pub fn next(self) -> Self {
        self.step(1)
    }

    /// Resolves the name into concrete colors.
    pub fn resolve(self) -> Theme {
        match self {
            // `auto` is resolved before it gets here, by `for_terminal`. This
            // arm exists only so the match is total, and it picks the same thing
            // `auto` picks for an unknown dark terminal.
            // `auto` never reaches here in normal use: `for_terminal` builds it
            // from the query. The arm exists so the match is total, and it returns
            // the same dark default `auto` falls back to.
            Self::Auto | Self::Monkeytype => Theme {
                background: Color::Rgb(20, 20, 20),
                surface: Color::Rgb(32, 32, 32),
                foreground: Color::Rgb(212, 212, 212),
                accent: Color::Rgb(255, 106, 61),
                correct: Color::Rgb(212, 212, 212),
                incorrect: Color::Rgb(232, 0, 0),
                extra: Color::Rgb(120, 200, 120),
                muted: Color::Rgb(100, 100, 100),
            },
            Self::Gruvbox => Theme {
                background: Color::Rgb(40, 40, 40),
                surface: Color::Rgb(60, 56, 54),
                foreground: Color::Rgb(235, 219, 178),
                accent: Color::Rgb(250, 189, 47),
                correct: Color::Rgb(184, 187, 38),
                incorrect: Color::Rgb(251, 73, 52),
                extra: Color::Rgb(142, 192, 124),
                muted: Color::Rgb(146, 131, 116),
            },
            Self::Nord => Theme {
                background: Color::Rgb(46, 52, 64),
                surface: Color::Rgb(59, 66, 82),
                foreground: Color::Rgb(216, 222, 233),
                accent: Color::Rgb(136, 192, 208),
                correct: Color::Rgb(163, 190, 140),
                incorrect: Color::Rgb(191, 97, 106),
                extra: Color::Rgb(129, 161, 193),
                muted: Color::Rgb(76, 86, 106),
            },
            Self::Dracula => Theme {
                background: Color::Rgb(40, 42, 54),
                surface: Color::Rgb(68, 71, 90),
                foreground: Color::Rgb(248, 248, 242),
                accent: Color::Rgb(189, 147, 249),
                correct: Color::Rgb(80, 250, 123),
                incorrect: Color::Rgb(255, 85, 85),
                extra: Color::Rgb(139, 233, 253),
                muted: Color::Rgb(98, 114, 164),
            },
            Self::SolarizedDark => Theme {
                background: Color::Rgb(0, 43, 54),
                surface: Color::Rgb(7, 54, 66),
                foreground: Color::Rgb(131, 148, 150),
                accent: Color::Rgb(38, 139, 210),
                correct: Color::Rgb(133, 153, 0),
                incorrect: Color::Rgb(220, 50, 47),
                extra: Color::Rgb(42, 161, 152),
                muted: Color::Rgb(88, 110, 117),
            },
            Self::SolarizedLight => Theme {
                background: Color::Rgb(253, 246, 227),
                surface: Color::Rgb(238, 232, 213),
                foreground: Color::Rgb(101, 123, 131),
                accent: Color::Rgb(38, 139, 210),
                correct: Color::Rgb(133, 153, 0),
                incorrect: Color::Rgb(220, 50, 47),
                extra: Color::Rgb(42, 161, 152),
                // A muted colour on a light background has to be *darker* than
                // the foreground, not lighter: "recede" means less contrast
                // against the background, and in a light theme that is down.
                muted: Color::Rgb(147, 161, 161),
            },
            Self::Catppuccin => Theme {
                background: Color::Rgb(30, 30, 46),
                surface: Color::Rgb(49, 50, 68),
                foreground: Color::Rgb(205, 214, 244),
                accent: Color::Rgb(137, 180, 250),
                correct: Color::Rgb(166, 227, 161),
                incorrect: Color::Rgb(243, 139, 168),
                extra: Color::Rgb(148, 226, 213),
                muted: Color::Rgb(108, 112, 134),
            },
            Self::TokyoNight => Theme {
                background: Color::Rgb(26, 27, 38),
                surface: Color::Rgb(36, 40, 59),
                foreground: Color::Rgb(192, 202, 245),
                accent: Color::Rgb(122, 162, 247),
                correct: Color::Rgb(158, 206, 106),
                incorrect: Color::Rgb(247, 118, 142),
                extra: Color::Rgb(125, 207, 255),
                muted: Color::Rgb(86, 95, 137),
            },
            Self::RosePine => Theme {
                background: Color::Rgb(25, 23, 36),
                surface: Color::Rgb(31, 29, 46),
                foreground: Color::Rgb(224, 222, 244),
                accent: Color::Rgb(196, 167, 231),
                correct: Color::Rgb(156, 207, 216),
                incorrect: Color::Rgb(235, 111, 146),
                extra: Color::Rgb(196, 167, 231),
                muted: Color::Rgb(110, 106, 134),
            },
            Self::Kanagawa => Theme {
                background: Color::Rgb(42, 42, 55),
                surface: Color::Rgb(54, 54, 70),
                foreground: Color::Rgb(220, 215, 186),
                accent: Color::Rgb(127, 160, 193),
                correct: Color::Rgb(152, 187, 108),
                incorrect: Color::Rgb(232, 36, 36),
                extra: Color::Rgb(136, 187, 179),
                muted: Color::Rgb(114, 113, 105),
            },
            Self::Everforest => Theme {
                background: Color::Rgb(45, 53, 59),
                surface: Color::Rgb(56, 66, 73),
                foreground: Color::Rgb(211, 198, 170),
                accent: Color::Rgb(167, 192, 128),
                correct: Color::Rgb(167, 192, 128),
                incorrect: Color::Rgb(230, 126, 128),
                extra: Color::Rgb(127, 187, 179),
                muted: Color::Rgb(102, 109, 110),
            },
            Self::GruvboxLight => Theme {
                background: Color::Rgb(251, 241, 199),
                surface: Color::Rgb(235, 219, 178),
                foreground: Color::Rgb(60, 56, 54),
                accent: Color::Rgb(175, 58, 3),
                correct: Color::Rgb(121, 116, 14),
                incorrect: Color::Rgb(157, 0, 6),
                extra: Color::Rgb(7, 102, 120),
                muted: Color::Rgb(124, 111, 100),
            },
            Self::OneHalfDark => Theme {
                background: Color::Rgb(40, 44, 52),
                surface: Color::Rgb(56, 61, 70),
                foreground: Color::Rgb(220, 223, 228),
                accent: Color::Rgb(97, 175, 239),
                correct: Color::Rgb(152, 195, 121),
                incorrect: Color::Rgb(224, 108, 117),
                extra: Color::Rgb(86, 182, 194),
                muted: Color::Rgb(92, 99, 112),
            },
            Self::OneHalfLight => Theme {
                background: Color::Rgb(250, 250, 250),
                surface: Color::Rgb(238, 238, 238),
                foreground: Color::Rgb(40, 44, 52),
                accent: Color::Rgb(64, 120, 242),
                correct: Color::Rgb(80, 161, 79),
                incorrect: Color::Rgb(203, 66, 66),
                extra: Color::Rgb(0, 143, 143),
                muted: Color::Rgb(140, 145, 150),
            },
            Self::Tomorrow => Theme {
                background: Color::Rgb(38, 38, 38),
                surface: Color::Rgb(50, 50, 50),
                foreground: Color::Rgb(204, 204, 204),
                accent: Color::Rgb(153, 153, 255),
                correct: Color::Rgb(180, 255, 180),
                incorrect: Color::Rgb(255, 119, 119),
                extra: Color::Rgb(164, 210, 255),
                muted: Color::Rgb(120, 120, 120),
            },
            Self::Material => Theme {
                background: Color::Rgb(38, 43, 54),
                surface: Color::Rgb(51, 58, 72),
                foreground: Color::Rgb(207, 216, 226),
                accent: Color::Rgb(130, 170, 255),
                correct: Color::Rgb(195, 232, 141),
                incorrect: Color::Rgb(240, 113, 120),
                extra: Color::Rgb(86, 182, 194),
                muted: Color::Rgb(105, 112, 128),
            },
            Self::Zenburn => Theme {
                background: Color::Rgb(59, 46, 46),
                surface: Color::Rgb(73, 61, 58),
                foreground: Color::Rgb(220, 216, 195),
                accent: Color::Rgb(240, 198, 116),
                correct: Color::Rgb(167, 192, 128),
                incorrect: Color::Rgb(255, 87, 62),
                extra: Color::Rgb(125, 207, 255),
                muted: Color::Rgb(146, 131, 116),
            },
            Self::BuiltinDark => Theme {
                // The terminal's own colours, left alone. Every slot is
                // `Color::Reset` except the two that have to be told apart, so
                // this theme follows whatever palette the terminal is configured
                // with instead of fighting it.
                background: Color::Reset,
                surface: Color::Rgb(45, 45, 45),
                foreground: Color::Reset,
                accent: Color::Cyan,
                correct: Color::Green,
                incorrect: Color::Red,
                extra: Color::Blue,
                muted: Color::DarkGray,
            },
        }
    }
}

/// The colors a screen needs. Screens compose these into `Style`s via the helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub foreground: Color,
    /// Titles, the active selection, the accent word.
    pub accent: Color,
    /// Correct characters.
    pub correct: Color,
    /// Incorrect characters.
    pub incorrect: Color,
    /// Correct characters marked extra.
    pub extra: Color,
    /// Untyped characters, punctuation, and other chrome.
    pub muted: Color,
}

impl Theme {
    /// One slot by name, for a test that is about which slot a colour is in.
    ///
    /// Not a lookup a caller should want: a `match` on a string that fails to compile
    /// when a slot is renamed is a better trade than a `get` that returns `None` and a
    /// panic three assertions later.
    pub fn slot(self, name: &str) -> Color {
        match name {
            "background" => self.background,
            "surface" => self.surface,
            "foreground" => self.foreground,
            "accent" => self.accent,
            "correct" => self.correct,
            "incorrect" => self.incorrect,
            "extra" => self.extra,
            "muted" => self.muted,
            other => panic!("no theme slot called {other:?}"),
        }
    }

    /// Base style for the whole frame.
    pub fn base(self) -> Style {
        Style::default().fg(self.foreground).bg(self.background)
    }

    /// Style for the caret block.
    pub fn caret(self) -> Style {
        Style::default()
            .fg(self.background)
            .bg(self.foreground)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for accent chrome: titles, the active selection.
    pub fn heading(self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// The plain text colour, for a character that has not been typed yet.
    ///
    /// The site gives an untyped letter no state class at all, so it comes out in
    /// `--text-color`; using the muted colour here instead made untouched text look
    /// disabled, which is a different thing.
    pub fn muted(&self) -> Style {
        Style::default().fg(self.foreground)
    }

    /// Style for chrome that should recede.
    pub fn chrome(self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Every colour the theme defines, for checking a whole theme at once.
    pub fn all_colors(&self) -> [Color; 8] {
        [
            self.background,
            self.surface,
            self.foreground,
            self.accent,
            self.correct,
            self.incorrect,
            self.extra,
            self.muted,
        ]
    }

    /// Style for something that went right.
    pub fn correct_style(self) -> Style {
        Style::default().fg(self.correct)
    }

    /// Style for a live number, e.g. the wpm counter.
    pub fn value(self) -> Style {
        Style::default()
            .fg(self.foreground)
            .add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_resolves_and_is_readable() {
        for name in ThemeName::ALL {
            let theme = name.resolve();
            assert_ne!(theme.correct, theme.incorrect, "{name:?} hides errors");
            // The one exception, and it is an exception on purpose: `builtin`
            // hands the background and foreground back to the terminal, so the
            // only thing this crate can be wrong about is the error pair, which
            // the line above already checked.
            if name != ThemeName::BuiltinDark {
                assert_ne!(theme.background, theme.foreground, "{name:?} is unreadable");
            }
        }
    }

    /// Muted means "less contrast against the background", which in a dark theme
    /// means lighter and in a light theme darker. A muted colour that is lighter
    /// than the foreground on a light background is the mistake that makes a light
    /// theme unusable, so it is checked rather than assumed.
    #[test]
    fn a_muted_colour_recedes_on_either_background() {
        for name in ThemeName::ALL {
            let theme = name.resolve();
            let (bg, fg, muted) = (theme.background, theme.foreground, theme.muted);
            if name == ThemeName::BuiltinDark {
                continue;
            }
            // "Recede" means *less contrast against the background*, and which
            // direction that is flips with the theme: on a dark background the
            // muted text goes darker, on a light one it goes lighter. The one
            // statement that holds for both is that it ends up nearer the
            // background than the foreground is — and a rule written for dark
            // themes alone would have rejected every light one here.
            assert!(
                (luminance(muted) - luminance(bg)).abs() < (luminance(fg) - luminance(bg)).abs(),
                "{name:?}: muted is not lower contrast against the background than the foreground"
            );
            // And it must still be visible against the background. 0.03 is about
            // 8 of 255 — low, because "recede" is supposed to be low, but low
            // enough that the text is not simply the background colour.
            assert!(
                (luminance(muted) - luminance(bg)).abs() > 0.03,
                "{name:?}: muted is invisible against its background"
            );
        }
    }

    /// A theme with a light background must be marked as one, or the automatic
    /// setting can never pick it for a light terminal.
    #[test]
    fn the_light_themes_are_the_ones_marked_light() {
        let light: Vec<ThemeName> = ThemeName::ALL
            .iter()
            .copied()
            .filter(|t| t.is_light())
            .collect();
        assert!(light.len() >= 3, "{light:?}");
        for name in light {
            let theme = name.resolve();
            assert!(
                luminance(theme.background) > luminance(theme.foreground),
                "{name:?} is marked light but its background is darker than its text"
            );
        }
    }

    /// Relative luminance, the same weighting the eye uses.
    fn luminance(color: Color) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        match color {
            Color::Rgb(r, g, b) => 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b),
            // The named colours a terminal actually shows, for the themes that
            // hand the terminal its own palette.
            Color::Black => 0.0,
            Color::White => 1.0,
            Color::Red => 0.2126,
            Color::Green => 0.7152,
            Color::Blue => 0.0722,
            Color::Cyan => 0.7874,
            Color::Magenta => 0.2848,
            Color::Yellow => 0.9278,
            Color::Gray | Color::DarkGray => 0.2158,
            _ => 0.5,
        }
    }

    /// `auto` is the terminal's own colours, and the same set of them whatever the
    /// terminal looks like.
    ///
    /// It used to *pick* a theme — solarized light for a light terminal, monkeytype
    /// for a dark one or an unknown one — which is what made `auto` and `monkeytype`
    /// the same setting on every terminal that would not answer the colour query. The
    /// guess also had the wrong failure: a dark theme on a light terminal is
    /// unreadable, and the guess for "light terminal" came from `COLORFGBG`, which a
    /// terminal that does not answer the colour query usually does not set either.
    ///
    /// There is no picking now. Every colour is either `Color::Reset`, which is the
    /// terminal's own default, or one of the sixteen palette entries, which is the
    /// terminal's own idea of that colour. A dark terminal gets its dark and a light
    /// terminal gets its light, and nothing had to guess which was which.
    #[test]
    fn auto_is_the_terminals_own_colours_and_never_a_bundled_theme() {
        use crate::config::terminal::Terminal;
        // True colour, so nothing is rounded and the colours can be compared
        // directly; the rounding is a separate test.
        let truecolor = Some("truecolor".to_owned());
        let light = Terminal {
            color_term: truecolor.clone(),
            color_fgbg: Some((0, 15)),
            ..Terminal::default()
        };
        let dark = Terminal {
            color_term: truecolor.clone(),
            color_fgbg: Some((15, 0)),
            ..Terminal::default()
        };
        let unknown = Terminal {
            color_term: truecolor,
            ..Terminal::default()
        };

        for (label, terminal) in [("light", &light), ("dark", &dark), ("unknown", &unknown)] {
            let theme = ThemeName::Auto.for_terminal(terminal);
            assert_eq!(
                theme.background,
                Color::Reset,
                "{label}: auto chose a background instead of the terminal's"
            );
            assert_eq!(
                theme.foreground,
                Color::Reset,
                "{label}: auto chose a foreground"
            );
            assert_eq!(theme.surface, Color::Reset, "{label}: auto chose a surface");
            for bundled in ThemeName::ALL {
                assert_ne!(
                    theme,
                    bundled.resolve(),
                    "{label}: auto resolved to the {bundled:?} theme"
                );
            }
        }
    }

    /// The colours `auto` cannot default are the terminal's palette *entries*, not
    /// anybody's RGB — a user who has remapped their red gets their red.
    ///
    /// Asserted as a *pair* of spellings rather than one, because a terminal with
    /// few colours is handed the same palette entry in indexed form: `Color::Yellow`
    /// on a true-colour terminal and `Color::Indexed(3)` on a sixteen-colour one are
    /// the same request answered twice, and which spelling comes back depends on the
    /// terminal rather than on the theme.
    #[test]
    fn the_colours_auto_cannot_default_are_palette_entries() {
        use crate::config::terminal::Terminal;
        let on_truecolor = ThemeName::Auto.for_terminal(&Terminal {
            color_term: Some("truecolor".to_owned()),
            ..Terminal::default()
        });
        let on_sixteen = ThemeName::Auto.for_terminal(&Terminal::default());
        // (name, named, indexed) — the ANSI index of each, which is the same number
        // for both spellings because the named colours *are* the palette entries.
        let wanted = [
            ("accent", Color::Yellow, 3u8),
            ("correct", Color::Green, 2),
            ("incorrect", Color::Red, 1),
            ("extra", Color::Blue, 4),
            ("muted", Color::DarkGray, 8),
        ];
        for (name, named, index) in wanted {
            assert_eq!(
                on_truecolor.slot(name),
                named,
                "{name} on a true-colour terminal"
            );
            assert_eq!(
                on_sixteen.slot(name),
                Color::Indexed(index),
                "{name} on a sixteen-colour terminal is not the terminal's own entry"
            );
        }
        // And the three that are the terminal's default stay `Reset` either way, which
        // is the part that makes the theme defer rather than choose.
        for terminal in [
            Terminal {
                color_term: Some("truecolor".to_owned()),
                ..Terminal::default()
            },
            Terminal::default(),
        ] {
            let theme = ThemeName::Auto.for_terminal(&terminal);
            for name in ["background", "surface", "foreground"] {
                assert_eq!(theme.slot(name), Color::Reset, "{name} was rounded");
            }
        }
    }

    /// An explicit theme is never overridden, however wrong the guess would have
    /// been: the user asked for it.
    #[test]
    fn an_explicit_theme_is_left_alone() {
        use crate::config::terminal::Terminal;
        let light = Terminal {
            color_fgbg: Some((0, 15)),
            ..Terminal::default()
        };
        for name in ThemeName::choices() {
            assert_eq!(ThemeName::name_is_explicit(name), name != ThemeName::Auto);
            let _ = light;
        }
        let truecolor = Terminal {
            color_term: Some("truecolor".to_owned()),
            color_fgbg: Some((0, 15)),
            ..Terminal::default()
        };
        assert_eq!(
            ThemeName::Nord.for_terminal(&truecolor),
            ThemeName::Nord.resolve(),
            "nord was overridden by auto's rule"
        );
        let _ = light;
    }

    /// An eight-colour terminal gets the eight entries, and still gets the terminal's
    /// own background.
    ///
    /// `Reset` used to be rounded here, to black, which is the nearest thing to a
    /// colour with no RGB — so on a terminal with few colours `auto` replaced "your
    /// background" with index 0 and stopped deferring. `Reset` is now left alone: it
    /// is the absence of a colour, not a colour, and there is nothing to round it to.
    #[test]
    fn auto_adapts_the_colours_to_the_terminals_depth() {
        use crate::config::terminal::Terminal;
        let depth8 = Terminal {
            term: Some("xterm".to_owned()),
            ..Terminal::default()
        };
        let theme = ThemeName::Auto.for_terminal(&depth8);
        for color in theme.all_colors() {
            let within = match color {
                Color::Reset => true,
                Color::Indexed(i) => i < 16,
                _ => false,
            };
            assert!(within, "{color:?} is beyond an eight-colour terminal");
        }
        assert_eq!(
            theme.background,
            Color::Reset,
            "auto lost the terminal's background"
        );
        assert_eq!(
            theme.foreground,
            Color::Reset,
            "auto lost the terminal's foreground"
        );
    }

    #[test]
    fn the_theme_list_offers_everything_but_auto_itself() {
        // `auto` is what is already configured, not something to cycle to.
        assert!(!ThemeName::choices().any(|t| t == ThemeName::Auto));
        assert_eq!(ThemeName::choices().count(), ThemeName::ALL.len() - 1);
    }

    #[test]
    fn next_wraps_around() {
        let last = *ThemeName::ALL.last().expect("non-empty");
        assert_eq!(last.next(), ThemeName::ALL[0]);
    }

    #[test]
    fn name_serialises_as_a_bare_string() {
        // In a config file the theme is `theme = "gruvbox"`, not a bare value.
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Wrapper {
            theme: ThemeName,
        }

        let text = toml::to_string(&Wrapper {
            theme: ThemeName::Gruvbox,
        })
        .expect("serialises");
        assert_eq!(text.trim(), "theme = \"gruvbox\"");

        let parsed: Wrapper = toml::from_str("theme = \"gruvbox\"").expect("valid name");
        assert_eq!(parsed.theme, ThemeName::Gruvbox);
        assert!(toml::from_str::<Wrapper>("theme = \"nope\"").is_err());
    }
}
