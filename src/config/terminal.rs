//! Themes, and working out what the terminal behind them is actually like.
//!
//! A theme is a guess about colours the terminal has not told you, and a guess is
//! only as good as the information available. Some terminals say outright:
//!
//! - `COLORTERM=truecolor` or `24bit` means 24-bit colour is available, so every
//!   colour can be the one the theme author intended.
//! - `TERM` ending in `-256color` means 256 colours, which is enough for any
//!   theme but means the true-colour values have to be rounded.
//! - `COLORFGBG=15;0` — set by rxvt, urxvt and a few others — says the default
//!   foreground and background, which is the only way to find out whether the
//!   terminal has a **light** background. That matters more than anything else
//!   here: a dark theme on a light terminal is unreadable, and no amount of
//!   colour rounding fixes it.
//!
//! When nothing is known the terminal is assumed to be dark, which is what a
//! terminal with no configuration at all nearly always is.
//!
//! ## What is not attempted
//!
//! Querying the terminal over OSC 11 for its background colour would be more
//! accurate, but it needs a reply and a timeout, and the terminal module is
//! synchronous inside a raw-mode alternate screen. A client that blocks for 100 ms
//! on startup to find out the background is a client that feels slow, so
//! [`Terminal`] reads the environment instead and the user can set `theme` by hand
//! when the guess is wrong.

use std::env;

use ratatui::style::Color;

use super::theme::Theme;

/// What the environment says about the terminal.
///
/// Every field is an `Option` because "the terminal said nothing" and "the
/// terminal said zero" are different answers: `COLORFGBG=0;0` is a terminal
/// reporting a black background, which is not the same as not reporting one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Terminal {
    /// `COLORTERM`.
    pub color_term: Option<String>,
    /// `TERM`.
    pub term: Option<String>,
    /// `COLORFGBG`, as `(foreground, background)` in terminal palette numbers.
    pub color_fgbg: Option<(u8, u8)>,
    /// `TERM_PROGRAM`, which is how the terminal emulator names itself.
    pub program: Option<String>,
}

impl Terminal {
    /// Reads what the environment says.
    ///
    /// `COLORFGBG` is read even when the other variables are missing, because
    /// the background is the thing worth knowing and an rxvt user will have it
    /// and nothing else.
    pub fn from_env() -> Self {
        let var = |name: &str| env::var(name).ok().map(|value| value.trim().to_owned());
        Self {
            color_term: var("COLORTERM"),
            term: var("TERM"),
            color_fgbg: var("COLORFGBG").as_deref().and_then(parse_color_fgbg),
            program: var("TERM_PROGRAM"),
        }
    }

    /// Whether the terminal takes 24-bit colour.
    pub fn supports_true_color(&self) -> bool {
        let value = self
            .color_term
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        value.contains("truecolor") || value.contains("24bit")
    }

    /// Whether the terminal has at least 256 colours.
    ///
    /// A terminal that says `COLORTERM=truecolor` obviously does; so does a bare
    /// `xterm-256color`. A terminal that says only `xterm` is assumed to be
    /// 8-colour, which is the safe direction to be wrong in: the theme degrades
    /// rather than emitting escape sequences the terminal will print literally.
    pub fn supports_256_colors(&self) -> bool {
        if self.supports_true_color() {
            return true;
        }
        let term = self.term.as_deref().unwrap_or_default();
        term.contains("256color") || term.contains("direct") || term == "alacritty"
    }

    /// Whether the terminal's background is light.
    ///
    /// Only the low half of the palette is light: colours 8..=15 are the bright
    /// variants, and a bright *background* among them is still dark enough for a
    /// light theme to be wrong. The `COLORFGBG` background is the **terminal's
    /// default background**, which is what a light-theme user has set.
    pub fn is_light(&self) -> bool {
        let Some((_, background)) = self.color_fgbg else {
            // Terminals that are known to ship a light default and say nothing
            // about it. macOS Terminal defaults to a light background, which is
            // the one case worth guessing at: everything else in the wild is
            // dark, and a wrong guess here is an unreadable screen.
            return self
                .program
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("Apple_Terminal"));
        };
        matches!(background, 7 | 11..=15)
    }

    /// How a theme's colours should be adjusted for this terminal.
    pub fn adapt(&self, theme: &mut Theme) {
        if self.supports_true_color() {
            return;
        }
        if self.supports_256_colors() {
            theme.round_to_256();
        } else {
            theme.round_to_16();
        }
    }
}

/// Parses `COLORFGBG`, which is `fg;bg` in terminal palette numbers.
///
/// Returns `None` for anything that is not two decimal numbers, rather than
/// guessing: a malformed value means the terminal is not one of the ones that
/// set it, and treating its first field as a colour would invert a light
/// terminal into a dark theme.
pub fn parse_color_fgbg(value: &str) -> Option<(u8, u8)> {
    let mut parts = value.split(';');
    let fg = parts.next()?.trim().parse::<u8>().ok()?;
    let bg = parts.next()?.trim().parse::<u8>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((fg, bg))
}

impl Theme {
    /// Rounds every colour to the 256-colour palette.
    pub fn round_to_256(&mut self) {
        self.round(nearest_256, nearest_256_keeping_hue, 0..=255);
    }

    /// Rounds every colour to the 16 basic ANSI colours.
    pub fn round_to_16(&mut self) {
        self.round(nearest_16, nearest_16_keeping_hue, 0..=15);
    }

    /// Rounds every colour, keeping the hue of the ones whose hue means something,
    /// and then making sure those are still different from each other.
    ///
    /// The coloured slots are read *before* anything is rounded and written after
    /// everything is, because the two passes must not feed each other: a hue
    /// rounded to a neutral grey first has no hue left to keep on the second pass,
    /// which is exactly the failure this is here to prevent.
    ///
    /// The distinctness repair is the last step and the one that matters. A small
    /// palette genuinely cannot hold every theme's hues — SolarizedDark's olive
    /// and its red are both nearest the same brown among eight colours — and
    /// rounding to the *nearest* entry in that case makes a correct character and
    /// a wrong one look the same. So when two slots land on the same colour, the
    /// second one is moved to the closest entry that is not taken. Being slightly
    /// the wrong hue is a much better failure than being indistinguishable, which
    /// is invisible rather than wrong.
    fn round(
        &mut self,
        plain: fn(Color) -> Color,
        keep_hue: fn(Color) -> Color,
        palette: std::ops::RangeInclusive<u8>,
    ) {
        let wanted = [self.accent, self.correct, self.incorrect, self.extra];
        for slot in self.slots_mut() {
            // `Reset` is not a colour to be rounded, it is the *absence* of one: the
            // terminal's own default. Rounding it would replace "whatever you use"
            // with a specific entry of the palette — and black, since that is the
            // nearest thing to a colour that has no RGB — which is the exact opposite
            // of what a slot set to `Reset` is asking for. The `auto` theme is built
            // out of `Reset`s, so this is what keeps it deferring to the terminal on
            // a terminal with sixteen colours.
            if !matches!(slot, Color::Reset) {
                *slot = plain(*slot);
            }
        }
        for (slot, source) in self.coloured_slots_mut().iter_mut().zip(wanted) {
            **slot = keep_hue(source);
        }

        // Wrong first, then right, then the rest: the error pair is the one that
        // has to survive, because a wrong keystroke is the one thing on screen
        // that the typist most needs to see.
        let order = [2usize, 1, 0, 3]; // incorrect, correct, accent, extra
        let mut taken: Vec<Color> = Vec::new();
        for index in order {
            let slots = self.coloured_slots_mut();
            let slot: &mut Color = slots[index];
            let current = *slot;
            if taken.contains(&current) {
                *slot = nearest_except(current, &taken, palette.clone());
            }
            taken.push(*slot);
        }
    }

    fn slots_mut(&mut self) -> [&mut Color; 8] {
        [
            &mut self.background,
            &mut self.surface,
            &mut self.foreground,
            &mut self.accent,
            &mut self.correct,
            &mut self.incorrect,
            &mut self.extra,
            &mut self.muted,
        ]
    }

    /// The slots whose colour *means* something, so they must not be rounded
    /// into each other or into a neutral grey.
    ///
    /// This is not a detail. The 256-colour palette's greyscale ramp is 24 steps
    /// of neutral, and the 16-colour palette has only two useful greys — so two
    /// different hues can both round to the same grey and a correct character and
    /// a wrong one become indistinguishable. A theme that has lost that
    /// distinction is worse than a theme with the wrong colour, because it looks
    /// like it is working.
    fn coloured_slots_mut(&mut self) -> [&mut Color; 4] {
        [
            &mut self.accent,
            &mut self.correct,
            &mut self.incorrect,
            &mut self.extra,
        ]
    }
}

/// The 16 basic ANSI colours, in the order the palette numbers 0..16 go.
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (170, 0, 0),
    (0, 170, 0),
    (170, 85, 0),
    (0, 0, 170),
    (170, 0, 170),
    (0, 170, 170),
    (170, 170, 170),
    (85, 85, 85),
    (255, 85, 85),
    (85, 255, 85),
    (255, 255, 85),
    (85, 85, 255),
    (255, 85, 255),
    (85, 255, 255),
    (255, 255, 255),
];

fn rgb(color: Color) -> (f64, f64, f64) {
    match color {
        Color::Rgb(r, g, b) => (f64::from(r), f64::from(g), f64::from(b)),
        // The sixteen named colours are the same sixteen palette entries under the
        // names ANSI gives them, so they get the same RGB. This has to be right or
        // rounding is nonsense for them: every named colour used to fall through to
        // the black arm below, so on a sixteen-colour terminal the `auto` theme —
        // which is built entirely from named colours, because "the terminal's own
        // red" *is* `Color::Red` — rounded its yellow to dark red and its green to
        // black. A theme whose whole claim is that the colours are the terminal's
        // came out as a different set of colours from the one it asked for.
        Color::Black => entry(0),
        Color::Red => entry(1),
        Color::Green => entry(2),
        Color::Yellow => entry(3),
        Color::Blue => entry(4),
        Color::Magenta => entry(5),
        Color::Cyan => entry(6),
        Color::Gray => entry(7),
        Color::DarkGray => entry(8),
        Color::LightRed => entry(9),
        Color::LightGreen => entry(10),
        Color::LightYellow => entry(11),
        Color::LightBlue => entry(12),
        Color::LightMagenta => entry(13),
        Color::LightCyan => entry(14),
        Color::White => entry(15),
        // The fixed palette, and then the 6x6x6 cube. Anything past 231 is the
        // greyscale ramp, which is the colour itself repeated, so the ramp is
        // handled by `nearest_256` rather than here.
        Color::Indexed(i) if i < 16 => {
            let (r, g, b) = ANSI16[i as usize];
            (f64::from(r), f64::from(g), f64::from(b))
        }
        Color::Indexed(i) if i < 232 => cube_rgb(i),
        Color::Indexed(i) => {
            let value = f64::from(8 + (i - 232) * 10);
            (value, value, value)
        }
        // `Reset` has no RGB of its own — it means "whatever the terminal uses" — and
        // black is the conservative answer, because rounding a terminal's own default
        // background into something else is the one thing that must not happen.
        _ => (0.0, 0.0, 0.0),
    }
}

/// One of the sixteen palette entries, as RGB.
fn entry(index: usize) -> (f64, f64, f64) {
    let (r, g, b) = ANSI16[index];
    (f64::from(r), f64::from(g), f64::from(b))
}

/// The closest colour in the 256-colour palette.
///
/// The palette is two halves: the 6x6x6 colour cube at 16..=231 and a 24-step
/// greyscale ramp at 232..=255. A grey's nearest entry is on the ramp, not in the
/// cube — the cube only holds two greys, 0 and 95, so rounding a theme's mid-grey
/// text to the cube gives it a visible tint. So both candidates are computed and
/// the nearer one wins, rather than reaching for the cube first.
pub fn nearest_256(color: Color) -> Color {
    let (r, g, b) = rgb(color);

    // The greyscale ramp: 24 steps of 10 from 8 to 238.
    let mean = (r + g + b) / 3.0;
    let level = ((mean - 8.0) / 10.0).round().clamp(0.0, 23.0);
    let ramp = 232 + level as u8;
    let ramp_value = 8.0 + level * 10.0;
    // The mean of the three channels already is the grey, so the error is the
    // difference itself, summed over the channels it applies to.
    let ramp_error = (mean - ramp_value).powi(2) * 3.0;

    let Some(entry) = cube(r, g, b) else {
        return Color::Indexed(ramp);
    };
    let (cr, cg, cb) = cube_rgb(entry);
    let cube_error = (r - cr).powi(2) + (g - cg).powi(2) + (b - cb).powi(2);
    if ramp_error < cube_error {
        Color::Indexed(ramp)
    } else {
        Color::Indexed(entry)
    }
}

/// The closest colour in the 256-colour palette, keeping the colour's hue.
///
/// For a colour whose hue carries meaning — a right character against a wrong one
/// — the greyscale ramp is not an option even when it is the nearest entry, which
/// it often is. This restricts the choice to the colour cube, so a green stays
/// green and a red stays red at the cost of a little accuracy in brightness.
pub fn nearest_256_keeping_hue(color: Color) -> Color {
    let (r, g, b) = rgb(color);
    match cube(r, g, b) {
        Some(entry) => Color::Indexed(entry),
        // Too far from the cube for a faithful match. Rather than fall back to a
        // neutral grey, which is the thing being avoided, take the cube entry
        // nearest by distance without the reachability test.
        None => Color::Indexed(nearest_cube_unbounded((r, g, b))),
    }
}

/// The closest colour in the 16 basic colours, keeping the colour's hue.
///
/// The basic palette is entirely made of hues plus three neutrals, so "keep the
/// hue" here means "do not choose black, bright black, silver or white".
pub fn nearest_16_keeping_hue(color: Color) -> Color {
    Color::Indexed(nearest_16_index(color, true) as u8)
}

/// The closest colour to `from` that is not one of `taken`.
///
/// The search is over the same palette [`nearest_256`] or [`nearest_16`] would
/// have used, so the result is still a colour the terminal can show. It is only
/// used when the nearest one is already spoken for.
fn nearest_except(from: Color, taken: &[Color], palette: std::ops::RangeInclusive<u8>) -> Color {
    let mut best: Option<(f64, Color)> = None;
    for index in palette {
        let candidate = Color::Indexed(index);
        if taken.contains(&candidate) {
            continue;
        }
        let distance = distance(from, candidate);
        let better = match best {
            Some((best_distance, _)) => distance < best_distance,
            None => true,
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    best.map_or(from, |(_, color)| color)
}

/// Squared distance between two palette colours.
fn distance(a: Color, b: Color) -> f64 {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    (ar - br).powi(2) + (ag - bg).powi(2) + (ab - bb).powi(2)
}

/// The nearest of the 16 basic colours.
pub fn nearest_16(color: Color) -> Color {
    Color::Indexed(nearest_16_index(color, false) as u8)
}

/// The index of the nearest of the 16 basic colours.
///
/// With `keep_hue`, the three neutral entries (0 black, 8 bright black, 7 silver,
/// 15 white) are only considered for a colour that is itself near-neutral.
fn nearest_16_index(color: Color, keep_hue: bool) -> usize {
    let (r, g, b) = rgb(color);
    let neutral = r.max(g).max(b) - r.min(g).min(b) < 40.0;
    let mut best = 0usize;
    let mut best_distance = f64::MAX;
    for (index, (fr, fg, fb)) in ANSI16.iter().enumerate() {
        if keep_hue && !neutral && is_neutral_index(index) {
            continue;
        }
        let dr = r - f64::from(*fr);
        let dg = g - f64::from(*fg);
        let db = b - f64::from(*fb);
        // Weighted to the eye's sensitivity, which is why a green test fails as
        // hard on a green background but a blue one is judged on brightness.
        let distance = 0.299 * dr * dr + 0.587 * dg * dg + 0.114 * db * db;
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best
}

/// Whether a basic-palette entry is one of its neutral greys.
fn is_neutral_index(index: usize) -> bool {
    matches!(index, 0 | 7 | 8 | 15)
}

/// The nearest cube entry to a colour, with no reachability test.
fn nearest_cube_unbounded((r, g, b): (f64, f64, f64)) -> u8 {
    const STEPS: [f64; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
    let quantise = |value: f64| -> usize {
        let mut best = 0usize;
        let mut best_distance = f64::MAX;
        for (index, step) in STEPS.iter().enumerate() {
            let distance = (value - step).abs();
            if distance < best_distance {
                best_distance = distance;
                best = index;
            }
        }
        best
    };
    16 + (quantise(r) * 36 + quantise(g) * 6 + quantise(b)) as u8
}

/// The 6x6x6 cube entry nearest to a colour, or `None` if it is nowhere near one.
///
/// The cube cannot represent a colour whose channels are all near 40 or all near
/// 200, so this returns `None` rather than an entry that would be visibly wrong.
fn cube(r: f64, g: f64, b: f64) -> Option<u8> {
    const STEPS: [f64; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
    let quantise = |value: f64| -> (usize, f64) {
        let mut best = 0usize;
        let mut best_distance = f64::MAX;
        for (index, step) in STEPS.iter().enumerate() {
            let distance = (value - step).abs();
            if distance < best_distance {
                best_distance = distance;
                best = index;
            }
        }
        (best, best_distance)
    };
    let (ri, rd) = quantise(r);
    let (gi, gd) = quantise(g);
    let (bi, bd) = quantise(b);
    // Beyond about 40 off on the worst channel, the colour is not in the cube.
    if rd.max(gd).max(bd) > 40.0 {
        return None;
    }
    // 16 is the first cube entry; the fixed palette occupies 0..16.
    Some(16 + (ri * 36 + gi * 6 + bi) as u8)
}

/// The RGB of a cube entry, whose layout is `16 + r*36 + g*6 + b`.
fn cube_rgb(entry: u8) -> (f64, f64, f64) {
    const STEPS: [f64; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
    let offset = (entry - 16) as usize;
    (
        STEPS[offset / 36],
        STEPS[(offset / 6) % 6],
        STEPS[offset % 6],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::theme::ThemeName;

    fn terminal(color_term: Option<&str>, term: Option<&str>, fgbg: Option<&str>) -> Terminal {
        Terminal {
            color_term: color_term.map(str::to_owned),
            term: term.map(str::to_owned),
            color_fgbg: fgbg.and_then(parse_color_fgbg),
            program: None,
        }
    }

    #[test]
    fn true_color_is_recognised_under_either_name() {
        for value in ["truecolor", "24bit", "TRUECOLOR", "24BIT"] {
            assert!(
                terminal(Some(value), None, None).supports_true_color(),
                "{value} was not recognised"
            );
        }
    }

    #[test]
    fn a_missing_colorterm_is_not_true_color() {
        assert!(!terminal(None, Some("xterm-256color"), None).supports_true_color());
    }

    #[test]
    fn the_256_colour_terminals_are_recognised() {
        for value in [
            "xterm-256color",
            "screen-256color",
            "alacritty",
            "tmux-256color",
        ] {
            assert!(
                terminal(None, Some(value), None).supports_256_colors(),
                "{value} was not recognised"
            );
        }
        // True colour implies 256, whatever TERM says.
        assert!(terminal(Some("truecolor"), Some("xterm"), None).supports_256_colors());
    }

    /// A terminal that says nothing must be assumed to be *not* capable, so the
    /// theme degrades rather than emitting sequences that get printed literally.
    #[test]
    fn an_unknown_terminal_is_assumed_to_have_eight_colours() {
        assert!(!terminal(None, Some("xterm"), None).supports_256_colors());
        assert!(!terminal(None, None, None).supports_256_colors());
    }

    /// The one fact worth detecting: a light terminal needs a light theme.
    #[test]
    fn a_light_background_is_detected_from_colorfgbg() {
        // The default palette: light foreground, dark background.
        assert!(!terminal(None, None, Some("15;0")).is_light());
        // Inverted: a light background.
        assert!(terminal(None, None, Some("0;15")).is_light());
        assert!(terminal(None, None, Some("0;7")).is_light());
        assert!(terminal(None, None, Some("0;11")).is_light());
        // Bright black is dark, despite the number.
        assert!(!terminal(None, None, Some("0;8")).is_light());
        // The low colours are dark.
        for bg in 0..=6 {
            assert!(
                !terminal(None, None, Some(&format!("0;{bg}"))).is_light(),
                "palette {bg} was read as light"
            );
        }
    }

    #[test]
    fn a_malformed_colorfgbg_is_ignored_rather_than_guessed() {
        for value in ["", "15", "15;0;0", "light;dark", "a;b", "15;-1"] {
            assert_eq!(parse_color_fgbg(value), None, "{value:?} was parsed");
        }
    }

    /// A terminal that says nothing is dark, because almost every one is.
    #[test]
    fn an_unknown_terminal_is_assumed_dark() {
        let mut t = Terminal {
            program: Some("iTerm.app".to_owned()),
            ..Terminal::default()
        };
        assert!(!t.is_light());
        // macOS Terminal is the one known light-by-default exception.
        t.program = Some("Apple_Terminal".to_owned());
        assert!(t.is_light());
    }

    #[test]
    fn a_truecolor_terminal_leaves_the_theme_alone() {
        let mut theme = ThemeName::Gruvbox.resolve();
        let before = theme;
        terminal(Some("truecolor"), None, None).adapt(&mut theme);
        assert_eq!(theme, before, "true-colour themes must not be rounded");
    }

    /// Rounding must produce colours the terminal can actually show.
    #[test]
    fn a_256_colour_terminal_gets_palette_colours_only() {
        let mut theme = ThemeName::Gruvbox.resolve();
        terminal(None, Some("xterm-256color"), None).adapt(&mut theme);
        for color in theme.all_colors() {
            assert!(
                matches!(color, Color::Indexed(_)),
                "{color:?} is not a palette colour"
            );
        }
    }

    #[test]
    fn an_eight_colour_terminal_gets_only_the_basic_sixteen() {
        let mut theme = ThemeName::Gruvbox.resolve();
        terminal(None, Some("xterm"), None).adapt(&mut theme);
        for color in theme.all_colors() {
            match color {
                Color::Indexed(i) => assert!(i < 16, "{i} is outside the basic 16"),
                other => panic!("{other:?} is not a palette colour"),
            }
        }
    }

    /// Rounding must not destroy readability: a theme that was legible has to
    /// stay legible.
    #[test]
    fn rounding_keeps_the_foreground_distinct_from_the_background() {
        for name in ThemeName::ALL {
            // `builtin` hands the background and foreground back to the terminal,
            // so there is nothing here for rounding to collapse.
            if name == crate::config::theme::ThemeName::BuiltinDark {
                continue;
            }
            let mut theme = name.resolve();
            for terminal in [
                terminal(None, Some("xterm-256color"), None),
                terminal(None, Some("xterm"), None),
            ] {
                let mut rounded = theme;
                terminal.adapt(&mut rounded);
                assert_ne!(
                    rounded.background, rounded.foreground,
                    "{name:?} became unreadable at {terminal:?}"
                );
                assert_ne!(
                    rounded.correct, rounded.incorrect,
                    "{name:?} lost its errors at {terminal:?}"
                );
                theme = rounded;
            }
        }
    }

    /// A grey's nearest palette entry is on the greyscale ramp, not in the
    /// colour cube: the cube only has two greys (0 and 95), so rounding to it
    /// tints neutral text.
    #[test]
    fn a_grey_goes_to_the_greyscale_ramp_and_not_the_cube() {
        for grey in [8u8, 30, 60, 100, 128, 180, 210, 238] {
            let rounded = nearest_256(Color::Rgb(grey, grey, grey));
            assert!(
                matches!(rounded, Color::Indexed(232..=255)),
                "grey {grey} became {rounded:?}"
            );
        }
    }

    #[test]
    fn a_colour_outside_the_cube_goes_to_the_greyscale_ramp() {
        // A saturated colour nothing in the cube can reach: 120 is more than 40
        // off the cube's 95, so the cube is not offered at all.
        let maroon = nearest_256(Color::Rgb(120, 10, 10));
        assert!(
            matches!(maroon, Color::Indexed(232..=255)),
            "{maroon:?} is in the cube and should not be"
        );
    }

    #[test]
    fn a_colour_in_the_cube_lands_on_it_exactly() {
        assert_eq!(nearest_256(Color::Rgb(0, 0, 0)), Color::Indexed(16));
        assert_eq!(nearest_256(Color::Rgb(255, 0, 0)), Color::Indexed(196));
        assert_eq!(nearest_256(Color::Rgb(255, 255, 255)), Color::Indexed(231));
        // 16 + r*36 + g*6 + b, so one step of red is 52.
        assert_eq!(nearest_256(Color::Rgb(95, 0, 0)), Color::Indexed(52));
    }

    #[test]
    fn rounding_an_unknown_colour_leaves_it_black() {
        // A `Reset` has no RGB value. Rounding it must not invent one.
        assert_eq!(nearest_256(Color::Reset), Color::Indexed(16));
        assert!(matches!(nearest_16(Color::Reset), Color::Indexed(_)));
    }
}
