//! Asking the terminal what colours it is using.
//!
//! Every other theme in this app is a guess: some colours someone picked in 2023,
//! adjusted to whatever the terminal can do. A terminal that knows its own
//! foreground and background knows them exactly, and a great many of them will
//! answer a question if asked properly.
//!
//! ## The question
//!
//! Two escape sequences, both answered rather than acted upon:
//!
//! ```text
//! ESC ] 11 ; ? BEL     "what is your background colour?"
//! ESC ] 10 ; ? BEL     "what is your foreground colour?"
//! ```
//!
//! The answer is the same sequence with a colour in it:
//!
//! ```text
//! ESC ] 11 ; rgb:1e1e/1e1e/1e1e ESC \
//! ```
//!
//! `ESC \` is ST (string terminator); some terminals use `BEL` instead, and both
//! are accepted. The components are 1–4 hex digits and are *scaled*, not `0xRR`:
//! `rgb:1e1e/1e1e/1e1e` is 16-bit and means 30/255, and `rgb:ff/ff/ff` is
//! 8-bit and means white. A component of one digit is `0xf` scaled to 16 bits,
//! so `f` and `ffff` are the same colour. Getting that wrong is how a query comes
//! back as a near-black screen and nobody can work out why.
//!
//! The 16-colour palette is asked for as well, one query for all of it:
//!
//! ```text
//! ESC ] 4 ; 1 ; ? ; 2 ; ? ; … ; 15 ; ? BEL
//! ```
//!
//! which is worth doing because it is the difference between inventing an error
//! colour and using the terminal's.
//!
//! ## Why it is awkward, and how it is handled
//!
//! The reply arrives on standard input, asynchronously, from another process, and
//! a terminal that does not understand the query says nothing at all. So:
//!
//! - raw mode has to be on, or the reply is canonical input and the line
//!   discipline may eat the control characters;
//! - the read has to have a deadline, or a terminal that ignores the query hangs
//!   the app forever;
//! - whatever arrives has to be a terminal that understands OSC, because a
//!   `TERM` that is not one of those will never answer and every path above has
//!   to give up gracefully;
//! - and the whole thing happens *once*, before the event loop starts, because
//!   reading standard input afterwards races crossterm's own reader for the same
//!   bytes.
//!
//! Every one of those failures is a reason to fall back to a guess, and none of
//! them is worth reporting to the user: a theme that quietly looks like the rest
//! of the app is a better outcome than an error about escape sequences.

use std::io::Write;
use std::sync::OnceLock;

use ratatui::style::Color;

use crate::config::theme::Theme;

/// The colours the terminal reported, and nothing else.
///
/// Every field is optional because every one of them can be missing: a terminal
/// can answer the background and not the palette, or answer nothing at all. A
/// `Reported` with everything `None` is a perfectly good value and means "use
/// the fallback".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reported {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    /// The 16 ANSI palette entries, indexed 0–15. `None` where the terminal did
    /// not answer for that index.
    pub ansi: [Option<Color>; 16],
}

impl Reported {
    /// Whether the terminal told us anything at all.
    ///
    /// The theme falls back to a guess when this is false, and the test asserts
    /// it, because a query that silently returns nothing is the most likely way
    /// this feature breaks and the hardest to notice.
    pub fn is_empty(&self) -> bool {
        self.foreground.is_none()
            && self.background.is_none()
            && self.ansi.iter().all(Option::is_none)
    }

    /// One of the 16 palette entries.
    pub fn ansi(&self, index: u8) -> Option<Color> {
        self.ansi.get(index as usize).copied().flatten()
    }
}

/// The process-wide answer, filled in once at start-up.
static REPORTED: OnceLock<Reported> = OnceLock::new();

/// What the terminal said, if anyone has asked.
///
/// Before [`cache_now`] this is empty, which is also what a test sees — so a
/// theme resolved in a test is the fallback theme, deterministically, rather than
/// whatever colour the machine running the tests happens to use.
pub fn reported() -> Reported {
    REPORTED.get().copied().unwrap_or_default()
}

/// Asks the terminal and remembers the answer.
///
/// Never fails and never blocks for long: see the module docs. A terminal that
/// does not answer costs a few hundred milliseconds at start-up and nothing else.
pub fn cache_now() {
    let _ = REPORTED.set(query());
}

/// Asks the terminal, right now, without caching.
///
/// The public entry point is [`cache_now`]; this exists so the query can be
/// tested against a fake terminal without going through a global.
#[cfg(unix)]
pub fn query() -> Reported {
    if !terminal_understands_osc() {
        return Reported::default();
    }
    // Raw mode is what lets the control characters through. If it cannot be
    // enabled the reply would be mangled, so there is no point asking.
    if crossterm::terminal::enable_raw_mode().is_err() {
        return Reported::default();
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "{}", request());
    let _ = out.flush();

    let bytes = read_reply(DEADLINE);

    // Raw mode off before anything else, including a panic path: leaving a
    // terminal in raw mode is the one failure here that is worse than no theme.
    let _ = crossterm::terminal::disable_raw_mode();

    parse(&bytes)
}

#[cfg(not(unix))]
pub fn query() -> Reported {
    // Nothing here knows how to poll a file descriptor without libc, and a
    // blocking read on standard input would eat the user's next keystroke. The
    // honest answer is "I do not know", and the theme falls back.
    Reported::default()
}

/// How long to wait for the reply.
///
/// Long enough for a terminal over a slow ssh link, short enough that a
/// `TERM` which ignores the query does not hold up start-up. Two hundred
/// milliseconds is about the round trip for a local terminal and about the point
/// where a user on a slow link starts wondering.
#[cfg(unix)]
const DEADLINE: std::time::Duration = std::time::Duration::from_millis(200);

/// The query itself: the two colours, then the whole 16-colour palette.
fn request() -> String {
    let mut query = String::from("\x1b]11;?\x07\x1b]10;?\x07");
    query.push_str("\x1b]4;");
    for index in 0..16u8 {
        if index > 0 {
            query.push(';');
        }
        query.push_str(&format!("{index};?"));
    }
    query.push('\x07');
    query
}

/// Whether a `TERM` names a terminal that answers OSC colour queries.
///
/// A function of the string rather than of the environment, so the list can be
/// tested without setting and unsetting `TERM` around every case.
///
/// A colour-capable `TERM` is the signal, not a list of emulators: `xterm-256color`,
/// `screen`, `tmux-256color`, `alacritty`, `kitty`, `wezterm` and everything else
/// that advertises colour. The refusals matter more than the agreements — a
/// terminal that does not answer costs a read that times out, which is cheap,
/// whereas being too eager means asking a terminal that will print the query.
fn osc_support_for(term: &str) -> bool {
    let term = term.to_ascii_lowercase();
    if term.is_empty() || term == "dumb" {
        return false;
    }
    // A monochrome terminal has no colours to report, and `ansi` is a lie some
    // pagers tell.
    if term.contains("mono") || term == "ansi" {
        return false;
    }
    // `vt100` and friends are colourless by definition.
    if term.starts_with("vt") && !term.contains("color") {
        return false;
    }
    term.contains("color")
        || term.contains("256")
        || term.contains("true")
        || term.contains("direct")
        || [
            "screen",
            "tmux",
            "alacritty",
            "kitty",
            "wezterm",
            "foot",
            "contour",
            "rio",
            "ghostty",
            "xterm",
            "vte",
            "st",
            "eterm",
            "nsterm",
            "iterm",
            "iterm2",
            "hyper",
        ]
        .iter()
        .any(|name| term == *name)
}

/// Whether the terminal we are actually running in will answer.
#[cfg(unix)]
fn terminal_understands_osc() -> bool {
    std::env::var("TERM").is_ok_and(|term| osc_support_for(&term))
}

/// Reads from standard input until the deadline, discarding anything that is not
/// part of an answer.
///
/// The reply is not a line and not a fixed length, so this reads in small chunks
/// and stops on the deadline rather than on a terminator. A chunk boundary can
/// land inside an escape sequence, so the bytes are accumulated and parsed at the
/// end — the parser is what knows where a sequence begins.
#[cfg(unix)]
fn read_reply(deadline: std::time::Duration) -> Vec<u8> {
    use std::os::fd::AsRawFd;

    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let start = std::time::Instant::now();
    let mut got: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 256];

    while start.elapsed() < deadline {
        // A 20 ms slice of the remaining time, so the deadline is honoured
        // whether it is 200 ms or less.
        let left = deadline.saturating_sub(start.elapsed());
        // A 20 ms slice of what is left, so the deadline is honoured whether it is
        // 200 ms or less. `poll`'s timeout is in whole milliseconds, which is why
        // this is an integer and not a duration.
        let slice = libc::c_int::try_from(left.as_millis().clamp(1, 20)).unwrap_or(20);
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll_fd` is a single well-formed `pollfd` and the count is 1, so
        // the call cannot read past the end of the array. A negative timeout would
        // block forever, and the clamp above rules that out.
        let ready = unsafe { libc::poll(&mut poll_fd, 1, slice) };
        if ready <= 0 {
            // `ready == 0` is the slice expiring, which is normal and means "ask
            // again if there is time left". A negative is an error, and an error
            // means no reply is coming.
            if ready < 0 {
                break;
            }
            continue;
        }
        // SAFETY: reading into a 256-byte slice from a file descriptor, and the
        // slice is the length passed.
        let read =
            unsafe { libc::read(fd, buffer.as_mut_ptr().cast::<libc::c_void>(), buffer.len()) };
        if read <= 0 {
            break;
        }
        let Ok(read) = usize::try_from(read) else {
            break;
        };
        got.extend_from_slice(&buffer[..read]);
        // An answer is at most a few hundred bytes; more than that means we are
        // reading the user's keystrokes, which is the one thing this must not do.
        if got.len() > 8192 {
            break;
        }
    }
    got
}

/// Reads the answers out of whatever came back.
///
/// Pure, and the only part of this that is worth testing: everything above it is
/// a conversation with another process. A terminal that answers all three
/// questions produces something like
///
/// ```text
/// ESC]10;rgb:c7c7/c7c7/c7c7ESC\ ESC]11;rgb:1c1c/1c1c/1c1cESC\ ESC]4;0;rgb:0000/0000/0000ESC\ …
/// ```
///
/// and one that does not produce nothing at all. Both have to come out right.
pub fn parse(bytes: &[u8]) -> Reported {
    let mut out = Reported::default();
    let mut at = 0usize;
    while at < bytes.len() {
        // Every answer starts with ESC ].
        if bytes[at] != 0x1b || bytes.get(at + 1) != Some(&b']') {
            at += 1;
            continue;
        }
        let body_start = at + 2;
        // Find the terminator: ST (`ESC \`) or BEL.
        let mut end = body_start;
        while end < bytes.len() {
            if bytes[end] == 0x07 {
                break;
            }
            if bytes[end] == 0x1b && bytes.get(end + 1) == Some(&b'\\') {
                break;
            }
            end += 1;
        }
        if end >= bytes.len() {
            // A truncated sequence at the end of the buffer is a chunk boundary,
            // not a malformed answer; there is nothing to learn from it.
            break;
        }
        if let Some(reply) = parse_body(&bytes[body_start..end]) {
            out.apply(reply);
        }
        at = end + if bytes[end] == 0x07 { 1 } else { 2 };
    }
    out
}

/// One parsed answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    Foreground(Color),
    Background(Color),
    /// `(palette index, colour)`.
    Palette(u8, Color),
}

impl Reported {
    fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Foreground(color) => self.foreground = Some(color),
            Reply::Background(color) => self.background = Some(color),
            Reply::Palette(index, color) => {
                if let Some(slot) = self.ansi.get_mut(index as usize) {
                    *slot = Some(color);
                }
            }
        }
    }
}

/// Parses the inside of one `ESC ] …` sequence, without the introducer or the
/// terminator.
///
/// Three shapes, and they are told apart by *position* rather than by sniffing
/// each field — a field that looks like a number could be either, since
/// `4;1;rgb:ff/ff/ff` alternates an index with a colour and `rgb:1/2/3` does not
/// contain a bare number at all:
///
/// - `10;<colour>`
/// - `11;<colour>`
/// - `4;<index>;<colour>;<index>;<colour>…`
fn parse_body(body: &[u8]) -> Option<Reply> {
    let text = std::str::from_utf8(body).ok()?;
    let mut parts = text.split(';');
    let kind = parts.next()?;
    match kind {
        "10" | "11" => {
            let colour = parse_colour(parts.next()?)?;
            // Exactly one field after the kind. Anything more is a different
            // sequence that happens to start the same way.
            if parts.next().is_some() {
                return None;
            }
            Some(if kind == "10" {
                Reply::Foreground(colour)
            } else {
                Reply::Background(colour)
            })
        }
        "4" => {
            // Index, colour, index, colour. The first answer is the one to
            // return, because `parse` is called once per sequence and a terminal
            // that answers sixteen indices is answering sixteen sequences — or
            // one long one, depending on the terminal, and both are in the wild.
            let mut index = parse_index(parts.next()?)?;
            let mut colour = parse_colour(parts.next()?)?;
            let mut found = Reply::Palette(index, colour);
            while let (Some(next_index), Some(next_colour)) = (parts.next(), parts.next()) {
                let Some(next_index) = parse_index(next_index) else {
                    return Some(found);
                };
                let Some(next_colour) = parse_colour(next_colour) else {
                    return Some(found);
                };
                index = next_index;
                colour = next_colour;
                found = Reply::Palette(index, colour);
            }
            Some(found)
        }
        _ => None,
    }
}

/// A palette index: a decimal number in 0–15.
///
/// Bounded on purpose. There are sixteen ANSI colours, and a terminal that says
/// `4;200;…` is either wrong or answering a different sequence — either way it is
/// not a colour this theme has a slot for, and storing it would need an array
/// bigger than the terminal has.
fn parse_index(text: &str) -> Option<u8> {
    text.parse::<u8>().ok().filter(|index| *index < 16)
}

/// A colour in any of the forms a terminal might send.
///
/// The forms, and the trap in the middle of them:
///
/// - `rgb:RR/GG/BB` — 8-bit, no scaling;
/// - `rgb:RRRR/GGGG/BBBB` — 16-bit, **scaled down**, so `ffff` is 255 and not
///   65535. Reading the bytes literally gives a terminal that answers in 16-bit
///   a near-black screen;
/// - `rgb:R/G/B` — one digit, also scaled, and `f` really does mean white;
/// - `#RRGGBB` — the same as the 8-bit `rgb:`, which some terminals send instead;
/// - `RRGGBB` — bare hex, which a few send;
/// - a palette *name* — `red`, `blue`, and so on, for the terminals that only
///   support the named set.
fn parse_colour(text: &str) -> Option<Color> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("rgb:") {
        return parse_rgb_parts(hex);
    }
    if let Some(hex) = lower.strip_prefix('#') {
        return parse_hex(hex, 2);
    }
    if let Some(named) = named(&lower) {
        return Some(named);
    }
    // Bare hex of six digits is unambiguous enough to accept: a colour name is
    // never six hex characters.
    parse_hex(&lower, 2)
}

/// The three components of an `rgb:` value, however many digits each has.
fn parse_rgb_parts(hex: &str) -> Option<Color> {
    let mut parts = hex.split('/');
    let red = parse_component(parts.next()?)?;
    let green = parse_component(parts.next()?)?;
    let blue = parse_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(Color::Rgb(red, green, blue))
}

/// One component, scaled to 8 bits whatever its width.
///
/// This is the whole difficulty of the format, and getting it wrong turns a
/// terminal's `ffff` into a near-black screen. Two things have to be right:
///
/// - the scale is the *largest value the width can hold*, not the next power of
///   two. One hex digit is 4 bits, so its full scale is **15**, not 16 — which is
///   why `f` means white and not 15/16 of the way to it;
/// - the division is by that maximum, so `ffff` is 65535/65535 and `8000` is
///   32768/65535, which is half.
fn parse_component(text: &str) -> Option<u8> {
    if text.is_empty() || text.len() > 4 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(text, 16).ok()?;
    let full = (1u64 << (4 * text.len())) - 1;
    // Rounded, because `8` at one digit is 136.0 and `7` is 119.0 and neither is
    // an integer number of 255ths.
    let scaled = (u64::from(value) * 255 + full / 2) / full;
    u8::try_from(scaled.min(255)).ok()
}

/// Hex of a known width, meaning what it says.
fn parse_hex(text: &str, width: usize) -> Option<Color> {
    if text.len() != width * 3 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let component = |offset: usize| u8::from_str_radix(&text[offset..offset + 2], 16).ok();
    Some(Color::Rgb(component(0)?, component(2)?, component(4)?))
}

/// The sixteen colour names, in the order every terminal agrees on.
///
/// Which is not the order the numbers are in. This list is indexed by *name* in
/// the order of the original VGA scheme, which is what `color0`…`color15` mean
/// everywhere except that the middle six are named after dark and light greys
/// here and after the second set of primaries in the xterm naming. Both
/// namings are accepted, because terminals send both.
fn named(name: &str) -> Option<Color> {
    Some(match name {
        "black" => Color::Rgb(0, 0, 0),
        "red" => Color::Rgb(205, 49, 49),
        "green" => Color::Rgb(13, 188, 121),
        "yellow" => Color::Rgb(229, 229, 16),
        "blue" => Color::Rgb(36, 114, 200),
        "magenta" | "purple" => Color::Rgb(188, 63, 188),
        "cyan" => Color::Rgb(17, 168, 205),
        "white" | "grey" | "gray" => Color::Rgb(229, 229, 229),
        "brightblack" => Color::Rgb(102, 102, 102),
        "brightred" => Color::Rgb(241, 76, 76),
        "brightgreen" => Color::Rgb(35, 209, 139),
        "brightyellow" => Color::Rgb(245, 245, 67),
        "brightblue" => Color::Rgb(59, 142, 234),
        "brightmagenta" | "brightpurple" => Color::Rgb(214, 112, 214),
        "brightcyan" => Color::Rgb(41, 184, 219),
        "brightwhite" => Color::Rgb(255, 255, 255),
        // The other common naming for the same sixteen, index by index.
        "color0" => Color::Rgb(0, 0, 0),
        "color1" => Color::Rgb(205, 49, 49),
        "color2" => Color::Rgb(13, 188, 121),
        "color3" => Color::Rgb(229, 229, 16),
        "color4" => Color::Rgb(36, 114, 200),
        "color5" => Color::Rgb(188, 63, 188),
        "color6" => Color::Rgb(17, 168, 205),
        "color7" => Color::Rgb(229, 229, 229),
        "color8" => Color::Rgb(102, 102, 102),
        "color9" => Color::Rgb(241, 76, 76),
        "color10" => Color::Rgb(35, 209, 139),
        "color11" => Color::Rgb(245, 245, 67),
        "color12" => Color::Rgb(59, 142, 234),
        "color13" => Color::Rgb(214, 112, 214),
        "color14" => Color::Rgb(41, 184, 219),
        "color15" => Color::Rgb(255, 255, 255),
        _ => return None,
    })
}

/// A theme built out of what the terminal said.
///
/// The interesting part is what happens when the terminal answered *part* of the
/// question. The background and foreground are the two it nearly always knows, so
/// everything that is not one of the sixteen palette entries is derived from those
/// two by mixing: a surface is the background moved a little towards the
/// foreground, a muted colour is the foreground moved most of the way back to the
/// background. That way a terminal which answers two questions out of three still
/// gets a coherent theme, and the colours that come from the palette — the error
/// red, the accent — are the terminal's own rather than invented.
pub fn build(reported: &Reported, fallback: &Theme) -> Theme {
    let mut theme = *fallback;
    if let Some(background) = reported.background {
        theme.background = background;
    }
    if let Some(foreground) = reported.foreground {
        theme.foreground = foreground;
    }
    if !reported.is_empty() {
        // A surface that is *not* the background, so the bar's cards still read
        // as cards. How far to move is a guess, and a small one: the site's
        // `--sub-alt-color` is a few percent of the way to the text colour.
        theme.surface = mix(theme.background, theme.foreground, 0.07);
        // Untyped text is the text colour pulled most of the way back towards the
        // background, which is what "less important" looks like when the only two
        // colours you have are the text and the page.
        theme.muted = mix(theme.background, theme.foreground, 0.45);
    }

    // From here on the palette entries, if the terminal gave them. The numbers
    // are the ANSI indices every terminal agrees on: 1 red, 2 green, 3 yellow,
    // 4 blue, 5 magenta, 6 cyan, 8 bright black (the grey).
    if let Some(red) = reported.ansi(1) {
        theme.incorrect = red;
    }
    if let Some(green) = reported.ansi(2) {
        theme.correct = mix(theme.background, green, 0.85);
    }
    if let Some(yellow) = reported.ansi(3) {
        theme.accent = yellow;
    }
    if let Some(blue) = reported.ansi(4) {
        theme.extra = blue;
    }
    if let Some(cyan) = reported.ansi(6) {
        // Cyan is the one palette entry that is reliably readable on both a light
        // and a dark background, which makes it the safest accent when the
        // terminal gave a yellow that will not contrast.
        if !is_light(theme.background) {
            theme.accent = cyan;
        }
    }
    theme
}

/// Whether a background is light, by brightness.
///
/// The threshold is a sum of the three channels rather than a mean or a luma
/// calculation, because this is not a case where the distinction needs to be
/// subtle: a theme built for a light page needs a dark accent and vice versa, and
/// getting that backwards makes the app unreadable rather than slightly off.
///
/// Public because a theme named "terminal" still has to know whether it is a
/// light theme, and the background it is judging is the one the terminal
/// reported.
pub fn is_light(background: Color) -> bool {
    match background {
        Color::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b) > 380,
        // Anything not truecolor has no components to judge, and assuming light
        // would put a dark accent on what is most likely a dark page.
        _ => false,
    }
}

/// Mixes two colours, `amount` of the way from `from` to `to`.
///
/// Only defined for RGB. A colour that came from a named palette entry and a
/// colour from `COLORFGBG` are both RGB by the time they get here, but an
/// indexed or reset colour in either position has no components to mix, and
/// guessing a component for `Color::Reset` would produce a colour nobody chose.
fn mix(from: Color, to: Color, amount: f32) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (from, to) else {
        return to;
    };
    let step = |a: u8, b: u8| -> u8 {
        let value = f32::from(a) + (f32::from(b) - f32::from(a)) * amount;
        value.round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(step(r0, r1), step(g0, g1), step(b0, b1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal that answers everything, in the shape it really answers in:
    /// 16-bit components, `ST` terminators, and the sixteen palette entries
    /// sixteen times. This is xterm's own default palette, so the colours it
    /// produces are checkable against a known answer rather than against itself.
    const FULL: &[u8] = concat!(
        "\x1b]10;rgb:c7c7/c7c7/c7c7\x1b\\",
        "\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\",
        "\x1b]4;0;rgb:0000/0000/0000\x1b\\",
        "\x1b]4;1;rgb:cdcd/0000/0000\x1b\\",
        "\x1b]4;2;rgb:0000/cdcd/0000\x1b\\",
        "\x1b]4;3;rgb:cdcd/cdcd/0000\x1b\\",
        "\x1b]4;4;rgb:0000/0000/eeee\x1b\\",
        "\x1b]4;6;rgb:0000/cdcd/cdcd\x1b\\",
        "\x1b]4;7;rgb:e5e5/e5e5/e5e5\x1b\\",
        "\x1b]4;8;rgb:7f7f/7f7f/7f7f\x1b\\",
    )
    .as_bytes();

    #[test]
    fn a_full_answer_is_read() {
        let got = parse(FULL);
        assert_eq!(got.foreground, Some(Color::Rgb(199, 199, 199)));
        assert_eq!(got.background, Some(Color::Rgb(28, 28, 28)));
        // xterm's default palette, in 8-bit terms.
        assert_eq!(got.ansi(0), Some(Color::Rgb(0, 0, 0)), "black");
        assert_eq!(got.ansi(1), Some(Color::Rgb(205, 0, 0)), "red");
        assert_eq!(got.ansi(2), Some(Color::Rgb(0, 205, 0)), "green");
        assert_eq!(got.ansi(3), Some(Color::Rgb(205, 205, 0)), "yellow");
        assert_eq!(got.ansi(4), Some(Color::Rgb(0, 0, 238)), "blue");
        assert_eq!(got.ansi(6), Some(Color::Rgb(0, 205, 205)), "cyan");
        assert_eq!(got.ansi(7), Some(Color::Rgb(229, 229, 229)), "white");
        assert_eq!(got.ansi(8), Some(Color::Rgb(127, 127, 127)), "bright black");
        assert!(!got.is_empty());
    }

    /// The 16-bit form is scaled, not read literally. Getting this wrong turns a
    /// terminal's `ffff` into near-black, which is the single most likely way for
    /// this feature to look broken.
    #[test]
    fn a_sixteen_bit_component_is_scaled_to_eight() {
        assert_eq!(
            parse(b"\x1b]11;rgb:ffff/ffff/ffff\x07").background,
            Some(Color::Rgb(255, 255, 255))
        );
        assert_eq!(
            parse(b"\x1b]11;rgb:0000/0000/0000\x07").background,
            Some(Color::Rgb(0, 0, 0))
        );
        // Half of full is half of 255, not 32767.
        assert_eq!(
            parse(b"\x1b]11;rgb:8000/8000/8000\x07").background,
            Some(Color::Rgb(128, 128, 128))
        );
    }

    /// One digit is also a fraction of full, not a nibble.
    #[test]
    fn a_one_digit_component_is_full_scale() {
        assert_eq!(
            parse(b"\x1b]11;rgb:f/f/f\x07").background,
            Some(Color::Rgb(255, 255, 255))
        );
        assert_eq!(
            parse(b"\x1b]11;rgb:8/8/8\x07").background,
            Some(Color::Rgb(136, 136, 136))
        );
        assert_eq!(
            parse(b"\x1b]11;rgb:0/0/0\x07").background,
            Some(Color::Rgb(0, 0, 0))
        );
    }

    #[test]
    fn the_eight_bit_form_is_read_as_is() {
        assert_eq!(
            parse(b"\x1b]10;rgb:ff/80/00\x07").foreground,
            Some(Color::Rgb(255, 128, 0))
        );
    }

    /// Some terminals answer with `#rrggbb` or with bare hex.
    #[test]
    fn the_other_colour_spellings_are_read() {
        assert_eq!(
            parse(b"\x1b]10;#ff8000\x07").foreground,
            Some(Color::Rgb(255, 128, 0))
        );
        assert_eq!(
            parse(b"\x1b]10;ff8000\x07").foreground,
            Some(Color::Rgb(255, 128, 0))
        );
    }

    /// And with a name, for the terminals that only know the sixteen.
    #[test]
    fn a_named_colour_is_read() {
        assert_eq!(
            parse(b"\x1b]11;black\x07").background,
            Some(Color::Rgb(0, 0, 0))
        );
        assert_eq!(
            parse(b"\x1b]4;1;red\x07").ansi(1),
            Some(Color::Rgb(205, 49, 49)),
            "a named colour is the xterm default, not the exact terminal's"
        );
    }

    /// The width decides the precision, as the xterm specification says: four hex
    /// digits is a 16-bit component *whatever the value looks like*. A terminal
    /// that sends `0102` meaning eight-bit is sending a value this cannot tell
    /// from 258, and reading it as eight-bit would be reading the specification
    /// wrong rather than accommodating a quirk.
    #[test]
    fn four_digits_is_sixteen_bit_and_two_is_eight() {
        assert_eq!(
            parse(b"\x1b]11;rgb:1000/2000/3000\x07").background,
            Some(Color::Rgb(16, 32, 48)),
            "a 4-digit component was read as 8-bit"
        );
        assert_eq!(
            parse(b"\x1b]11;rgb:01/02/03\x07").background,
            Some(Color::Rgb(1, 2, 3)),
            "a 2-digit component was not read as 8-bit"
        );
    }

    /// `BEL` and `ST` are both terminators, and terminals disagree.
    #[test]
    fn both_terminators_are_accepted() {
        let bel = parse(b"\x1b]11;rgb:01/02/03\x07");
        let st = parse(b"\x1b]11;rgb:01/02/03\x1b\\");
        assert_eq!(bel, st);
        assert_eq!(bel.background, Some(Color::Rgb(1, 2, 3)));
    }

    /// A terminal that ignores the query says nothing, and that is not an error.
    #[test]
    fn silence_gives_an_empty_answer() {
        assert!(parse(b"").is_empty());
        assert!(parse(b"hello\r\n").is_empty());
        assert_eq!(parse(b""), Reported::default());
    }

    /// Whatever else is on standard input — a keypress, a mouse report, a
    /// terminal's unsolicited Device Attributes reply — must not derail the parse.
    #[test]
    fn unrelated_bytes_are_skipped() {
        let mut bytes = b"\x1b[?1;2c".to_vec(); // a DA reply
        bytes.extend_from_slice(FULL);
        bytes.extend_from_slice(b"\x1b[<0;10;20M"); // a mouse report
        let got = parse(&bytes);
        assert_eq!(got.background, Some(Color::Rgb(28, 28, 28)), "{got:?}");
        assert_eq!(got.ansi(1), Some(Color::Rgb(205, 0, 0)), "{got:?}");
        assert_eq!(got.ansi(8), Some(Color::Rgb(127, 127, 127)), "{got:?}");
    }

    /// A read that stopped mid-sequence is a chunk boundary, not a bad answer.
    #[test]
    fn a_truncated_sequence_is_ignored_rather_than_guessed_at() {
        let got = parse(b"\x1b]11;rgb:1c1c");
        assert!(got.is_empty(), "{got:?}");
    }

    /// Nonsense in a colour position is not a colour.
    #[test]
    fn nonsense_is_refused() {
        for body in [
            "rgb:",
            "rgb:zz/zz/zz",
            "rgb:1/2",
            "rgb:1/2/3/4",
            "rgb:12345/1/1",
            "chartreuse",
            "",
        ] {
            assert!(
                parse(format!("\x1b]11;{body}\x07").as_bytes())
                    .background
                    .is_none(),
                "{body:?} parsed as a colour"
            );
        }
    }

    /// A palette index past 15 is ignored rather than panicking. There is no
    /// sixteenth-and-a-half colour, but a terminal that says 200 is not an error
    /// worth crashing over.
    #[test]
    fn a_palette_index_outside_the_sixteen_is_ignored() {
        let got = parse(b"\x1b]4;200;rgb:ff/ff/ff\x07");
        assert!(got.ansi.iter().all(Option::is_none), "{got:?}");
    }

    /// The whole point: a theme that looks like the terminal it is running in.
    #[test]
    fn a_theme_is_built_from_what_the_terminal_said() {
        let got = parse(FULL);
        let theme = build(&got, &fallback());
        assert_eq!(theme.background, Color::Rgb(28, 28, 28));
        assert_eq!(theme.foreground, Color::Rgb(199, 199, 199));
        assert_eq!(theme.incorrect, Color::Rgb(205, 0, 0), "the terminal's red");
        assert_eq!(theme.extra, Color::Rgb(0, 0, 238), "the terminal's blue");
        // A surface is not the background, or the bar's cards stop being cards.
        assert_ne!(
            theme.surface, theme.background,
            "the surface is the background"
        );
        // Muted text is dimmer than the text and brighter than the page.
        assert!(dimmer(theme.muted, theme.foreground, theme.background));
    }

    /// Two answers out of three still makes a coherent theme, because everything
    /// that is not a palette entry comes from the background and foreground.
    #[test]
    fn a_partial_answer_still_makes_a_theme() {
        let got = parse(b"\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]11;rgb:0000/0000/0000\x07");
        let theme = build(&got, &fallback());
        assert_eq!(theme.background, Color::Rgb(0, 0, 0));
        assert_eq!(theme.foreground, Color::Rgb(255, 255, 255));
        assert_ne!(theme.muted, theme.foreground);
        assert_ne!(theme.surface, theme.background);
    }

    /// And an empty answer leaves the fallback exactly as it was, rather than
    /// producing a theme of default blacks on a light terminal.
    #[test]
    fn an_empty_answer_leaves_the_fallback_alone() {
        let fallback = fallback();
        assert_eq!(build(&Reported::default(), &fallback), fallback);
    }

    /// The fallback itself: a plain dark theme, and the premise the tests above
    /// rely on.
    fn fallback() -> Theme {
        Theme {
            background: Color::Rgb(20, 20, 20),
            surface: Color::Rgb(32, 32, 32),
            foreground: Color::Rgb(212, 212, 212),
            accent: Color::Rgb(255, 106, 61),
            correct: Color::Rgb(212, 212, 212),
            incorrect: Color::Rgb(232, 0, 0),
            extra: Color::Rgb(255, 106, 61),
            muted: Color::Rgb(100, 100, 100),
        }
    }

    /// Whether `a` is between `b` and `c` in brightness.
    fn dimmer(a: Color, b: Color, c: Color) -> bool {
        let Color::Rgb(ar, ag, ab) = a else {
            return false;
        };
        let Color::Rgb(br, bg, bb) = b else {
            return false;
        };
        let Color::Rgb(cr, cg, cb) = c else {
            return false;
        };
        let lum = |r: u8, g: u8, b: u8| f32::from(r) + f32::from(g) + f32::from(b);
        lum(ar, ag, ab) < lum(br, bg, bb) && lum(ar, ag, ab) > lum(cr, cg, cb)
    }

    /// The query asks for the background first. Some terminals answer in the
    /// order they like rather than the order they were asked, which is why the
    /// parser does not care — but the *first* thing asked should be the one most
    /// likely to be answered, and the background is the one every terminal knows.
    #[test]
    fn the_query_asks_for_the_colours_that_every_terminal_knows() {
        let query = request();
        assert!(query.starts_with("\x1b]11;?"), "{query:?}");
        assert!(query.contains("\x1b]10;?"), "{query:?}");
        assert!(query.contains("\x1b]4;"), "the palette is not asked for");
        // All sixteen indices in one query, which is the form that works.
        for index in 0..16 {
            assert!(
                query.contains(&format!("{index};?")),
                "index {index} is missing"
            );
        }
    }

    /// A terminal that cannot answer is not asked, and a `TERM` that is not a
    /// terminal is not believed.
    #[test]
    fn a_terminal_that_cannot_answer_is_not_asked() {
        for term in ["dumb", "", "vt100", "ansi", "xterm-mono"] {
            assert!(!osc_support_for(term), "{term:?} was believed to answer");
        }
        for term in [
            "xterm-256color",
            "screen-256color",
            "tmux-256color",
            "alacritty",
            "kitty",
            "wezterm",
            "foot",
            "ghostty",
            "vte-256color",
        ] {
            assert!(osc_support_for(term), "{term:?} was not believed");
        }
    }
}
