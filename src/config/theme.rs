//! Themes.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// A named theme, serialisable as a bare string in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    #[default]
    Monkeytype,
    Gruvbox,
    Nord,
}

impl ThemeName {
    pub const ALL: [Self; 3] = [Self::Monkeytype, Self::Gruvbox, Self::Nord];

    /// The label shown in the settings screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Monkeytype => "monkeytype",
            Self::Gruvbox => "gruvbox",
            Self::Nord => "nord",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Resolves the name into concrete colors.
    pub fn resolve(self) -> Theme {
        match self {
            Self::Monkeytype => Theme {
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

    /// Style for chrome that should recede.
    pub fn chrome(self) -> Style {
        Style::default().fg(self.muted)
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
            assert_ne!(theme.background, theme.foreground, "{name:?} is unreadable");
            assert_ne!(theme.correct, theme.incorrect, "{name:?} hides errors");
        }
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
