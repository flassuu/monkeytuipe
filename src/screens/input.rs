//! The one input window: commands, durations, word counts, passages, keys.
//!
//! The site has a command line and, separately, a modal for a custom duration, a
//! modal for a custom word count, a modal for custom text and a field for the
//! ApeKey — five places to type something. Here they are one window, because a
//! terminal has one keyboard and a user does not think of "setting a duration" and
//! "running a command" as different activities.
//!
//! What it accepts depends on why it opened, and that is [`Reason`]:
//!
//! - opened for a **command**, anything typed is matched against the command list
//!   and running it is done by filtering;
//! - opened for a **number**, digits go to the field and a plausible duration is
//!   accepted the way the site accepts one — `1h30m`, `90`, `1.5`;
//! - opened for **text**, everything goes to the field.
//!
//! The number parsing is the site's, from `CustomTestDurationModal.parseInput`:
//! every run of digits with an optional `h`/`m`/`s` suffix is a term, and the
//! terms are added up. That is why `1h30m` works and why `-5` is a subtraction
//! rather than a duration.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config::bar::Field;
use crate::config::theme::Theme;
use crate::screens::commands;

/// Why the window is open, which decides what it accepts and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Run a command.
    Command,
    /// Set a length. The field is `Time`, `Words` or their wrench.
    Length(Field),
    /// Set a passage.
    Text,
    /// Set the ApeKey.
    ApeKey,
}

impl Reason {
    /// The heading, which says what is being asked for.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Command => "commands",
            Self::Length(field) => match field {
                Field::Time | Field::TimeCustom => "duration",
                _ => "words",
            },
            Self::Text => "custom text",
            Self::ApeKey => "ape key",
        }
    }

    /// A line under the field saying what is accepted.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Command => "type to search · ↑↓ move · enter run · esc close",
            // Zero is not offered: a word count of zero and a duration of zero
            // both mean an endless test, and endless is what zen is for.
            Self::Length(_) => "seconds, or 1h30m · h hours m minutes",
            Self::Text => "one passage per line · the first line is the test",
            Self::ApeKey => "from monkeytype account settings",
        }
    }

    /// Whether the text is read as a number, a command or a passage.
    fn kind(&self) -> Kind {
        match self {
            Self::Command => Kind::Command,
            Self::Length(_) => Kind::Number,
            Self::Text | Self::ApeKey => Kind::Text,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Command,
    Number,
    Text,
}

/// An open window: why it is open, what has been typed, and which match is
/// highlighted.
///
/// The state lives here rather than in the app so that "what does this window
/// show" has one answer: the matches are a function of the text, so caching them
/// on the app would be a second answer waiting to disagree with the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    reason: Reason,
    text: String,
    /// Which of the matches is highlighted. Only meaningful for a command window;
    /// a number or a text window has no list to move through.
    cursor: usize,
}

impl Window {
    /// Opens a window, empty.
    pub fn new(reason: Reason) -> Self {
        Self {
            reason,
            text: String::new(),
            cursor: 0,
        }
    }

    /// Opens a window with something already in it, which is what an edit does.
    pub fn prefilled(reason: Reason, text: impl Into<String>) -> Self {
        let mut window = Self::new(reason);
        window.text = text.into();
        window
    }

    /// Why it is open.
    pub fn reason(&self) -> &Reason {
        &self.reason
    }

    /// What has been typed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The matches for what has been typed.
    ///
    /// Empty for a window that is not searching, which is what makes the list
    /// simply absent rather than present and empty.
    pub fn matches(&self) -> Vec<commands::Match> {
        match self.reason.kind() {
            Kind::Command => commands::filter(&self.text),
            _ => Vec::new(),
        }
    }

    /// The highlighted match, if there is one.
    pub fn selected(&self) -> Option<commands::Match> {
        self.matches().get(self.cursor).copied()
    }

    /// Takes a keystroke, returning what came out of the window if it was one.
    ///
    /// Returning `None` means the key was not for the window and the caller
    /// should carry on as if it had not happened. That is how `ctrl+c` still
    /// quits with the window open.
    pub fn key(&mut self, code: crossterm::event::KeyCode) -> Option<Outcome> {
        use crossterm::event::KeyCode as K;
        match code {
            // Escape closes without acting, as it does in every modal the site
            // has. The command list is opened *by* escape, so the second one
            // closes it — which is how a palette is expected to behave.
            K::Esc => Some(Outcome::Cancelled),
            K::Enter => Some(self.outcome()),
            K::Backspace => {
                self.text.pop();
                self.cursor = 0;
                None
            }
            K::Up => {
                self.move_cursor(-1);
                None
            }
            K::Down => {
                self.move_cursor(1);
                None
            }
            // Everything else printable is text. The arrow keys and enter are the
            // only keys with a job of their own, and a window that is filtering a
            // list has nothing to do with a function key.
            other => {
                if let K::Char(c) = other {
                    self.text.push(c);
                    self.cursor = 0;
                }
                None
            }
        }
    }

    /// Moves the highlight, clamped to the list.
    ///
    /// The highlight returns to the top when it runs off the bottom, so holding
    /// down cycles rather than sticking — the site's command list does the same.
    fn move_cursor(&mut self, by: isize) {
        let len = self.matches().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let last = len as isize - 1;
        let next = (self.cursor as isize + by).rem_euclid(last + 1);
        self.cursor = next as usize;
    }

    /// What the window would produce if it were submitted now.
    pub fn outcome(&self) -> Outcome {
        let text = self.text.trim();
        match self.reason {
            Reason::Command => match self.selected() {
                Some(found) => Outcome::Command(found.command),
                // Nothing matched. Submitting a search that found nothing is not
                // "run nothing" — it is a query that has not answered yet, so the
                // window stays open with the text in it.
                None => Outcome::Invalid(text.to_owned()),
            },
            Reason::Length(field) => match parse_duration(text) {
                // Zero is endless, which is not a length this client can honour,
                // so it is refused here rather than producing a test that never
                // ends and cannot be stopped.
                Some(value) if value > 0 => Outcome::Length(field, value),
                _ => Outcome::Invalid(text.to_owned()),
            },
            Reason::Text => Outcome::Text(self.text.clone()),
            Reason::ApeKey => Outcome::ApeKey(self.text.clone()),
        }
    }

    /// A live reading of the text, for the line under the field.
    ///
    /// A number window shows what it has understood as the user types, because
    /// `1h30m` is not obviously five thousand four hundred seconds and the site
    /// shows the same figure.
    pub fn preview(&self) -> Option<String> {
        match self.reason {
            Reason::Length(_) => {
                let text = self.text.trim();
                if text.is_empty() {
                    return None;
                }
                Some(match parse_duration(text) {
                    Some(0) => "0 — endless, which is what zen is for".to_owned(),
                    Some(seconds) => format!("{seconds} seconds"),
                    None => format!("{text:?} is not a duration"),
                })
            }
            Reason::ApeKey => None,
            Reason::Text => {
                let words = self.text.split_whitespace().count();
                (words > 0).then(|| format!("{words} words"))
            }
            Reason::Command => None,
        }
    }

    /// How many rows the window needs.
    pub fn height(&self) -> u16 {
        // Border, title row, the field, the hint, and one row per match up to a
        // cap, plus a row for the live reading when there is one.
        let list = self.matches().len().min(MAX_VISIBLE) as u16;
        let extra = u16::from(self.preview().is_some());
        (4 + list + extra).min(MAX_HEIGHT)
    }

    /// The width the window wants, given the room there is.
    ///
    /// Long enough for the widest command label or the field, and never wider
    /// than the screen with a margin, so it cannot be drawn off the edge.
    pub fn width(&self, available: u16) -> u16 {
        let longest_label = commands::COMMANDS
            .iter()
            .map(|c| c.display.chars().count())
            .max()
            .unwrap_or(20);
        // The hint sits inside the border, so it has to fit too. A clipped hint
        // is worse than none: it looks like the sentence simply ends.
        let wanted = (longest_label + 4).max(self.reason.hint().chars().count() + 2);
        // A margin either side, and never so narrow that nothing can be read.
        let room = available.saturating_sub(4).max(1);
        (wanted as u16).min(room)
    }
}

/// What came out of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing; the window was closed.
    Cancelled,
    /// A command was chosen, by index into the command list.
    Command(usize),
    /// A length was set.
    Length(Field, u32),
    /// A passage was set.
    Text(String),
    /// An ApeKey was set.
    ApeKey(String),
    /// The text could not be read. The window stays open with it.
    Invalid(String),
}

/// The most matches shown at once.
///
/// The list scrolls past this rather than growing, because a window that grows
/// with the query would push the field off the screen at exactly the moment the
/// user is typing.
const MAX_VISIBLE: usize = 8;

/// The tallest the window may get, so a long list cannot swallow the screen.
const MAX_HEIGHT: u16 = 14;

/// Reads a duration the way the site does.
///
/// A run of digits, optionally with an `h`, `m` or `s` suffix, is a term; the
/// terms are added up and the result floored. So `90` is ninety seconds, `1h30m`
/// is five thousand four hundred, and `m` on its own matches nothing and is zero.
pub fn parse_duration(input: &str) -> Option<u32> {
    let mut total: f64 = 0.0;
    let mut matched = false;
    let bytes: Vec<char> = input.to_lowercase().chars().collect();
    let mut at = 0usize;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let mut start = at;
        // A minus before the digits makes it a negative number, which is then
        // refused rather than quietly read as the digits on their own.
        if start > 0 && matches!(bytes[start - 1], '-' | '\u{2212}') {
            start -= 1;
        }
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        // A fractional part, which the site's regex allows and which `1.5`
        // needs.
        if at + 1 < bytes.len() && bytes[at] == '.' && bytes[at + 1].is_ascii_digit() {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
        }
        let number: String = bytes[start..at].iter().collect();
        // An optional unit, with whitespace allowed between the number and it,
        // because the site's regex allows `1 h`.
        let mut probe = at;
        while probe < bytes.len() && bytes[probe].is_whitespace() {
            probe += 1;
        }
        let unit = match bytes.get(probe) {
            Some('h') => 3600.0,
            Some('m') => 60.0,
            Some('s') => 1.0,
            _ => 1.0,
        };
        if matches!(bytes.get(probe), Some('h' | 'm' | 's')) {
            at = probe + 1;
        }
        let Ok(value) = number.parse::<f64>() else {
            continue;
        };
        if value < 0.0 {
            return None;
        }
        total += value * unit;
        matched = true;
    }
    if !matched || total < 0.0 || !total.is_finite() {
        return None;
    }
    // A duration past this is a number nobody means, and letting it wrap would
    // produce a test that silently lasts a second.
    if total > f64::from(u32::MAX) {
        return None;
    }
    u32::try_from(total.floor() as u64).ok()
}

/// Draws the window over whatever is on screen.
///
/// Centred, and cleared underneath, the way the site's modals are: a box drawn
/// on top of the words would leave them legible through it, and a half-legible
/// background is harder to read than an opaque one.
pub fn render(window: &Window, area: Rect, theme: Theme, frame: &mut Frame) {
    let width = window.width(area.width);
    if width == 0 || area.height == 0 {
        return;
    }
    let height = window.height().min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    // Slightly above the middle, which is where the eye goes first and where the
    // site puts its modals.
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width, height);

    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().fg(theme.foreground).bg(theme.surface))
        .title(format!(" {} ", window.reason.title()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(field_line(&window.text, theme));
    if let Some(preview) = window.preview() {
        lines.push(Line::from(Span::styled(
            preview,
            Style::default().fg(theme.muted),
        )));
    }
    for (offset, found) in window.matches().into_iter().take(MAX_VISIBLE).enumerate() {
        let selected = offset == window.cursor;
        lines.push(match_line(
            commands::display(found.command),
            selected,
            theme,
        ));
    }
    // The hint is the last thing, so a long list cannot push it off.
    if (lines.len() as u16) < inner.height {
        lines.push(Line::from(Span::styled(
            window.reason.hint(),
            Style::default().fg(theme.muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The field: what has been typed, with a block caret on the next cell.
fn field_line(text: &str, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(
        text.to_owned(),
        Style::default().fg(theme.foreground),
    )];
    spans.push(Span::styled("█", Style::default().fg(theme.accent)));
    Line::from(spans)
}

/// One line of the match list.
fn match_line(label: &str, selected: bool, theme: Theme) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(theme.background)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.foreground)
    };
    // A marker rather than a colour alone, so the highlight is still findable on
    // a monochrome terminal.
    Line::from(vec![
        Span::styled(if selected { "▸ " } else { "  " }, Style::default()),
        Span::styled(label.to_owned(), style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode as K;

    fn window(reason: Reason) -> Window {
        Window::new(reason)
    }

    /// Sends a string of keystrokes to the window.
    fn type_into(window: &mut Window, text: &str) {
        for c in text.chars() {
            window.key(K::Char(c));
        }
    }

    #[test]
    fn a_plain_number_is_seconds() {
        assert_eq!(parse_duration("90"), Some(90));
        assert_eq!(parse_duration("0"), Some(0));
        assert_eq!(parse_duration("42"), Some(42));
    }

    /// The site's own example, and the reason the parser is not a split on `:`.
    #[test]
    fn hours_and_minutes_add_up() {
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("30m"), Some(1800));
        assert_eq!(parse_duration("2m30s"), Some(150));
        assert_eq!(parse_duration("1h 30m"), Some(5400), "space is allowed");
    }

    #[test]
    fn a_fraction_floors() {
        assert_eq!(parse_duration("1.5"), Some(1));
        assert_eq!(parse_duration("1.5m"), Some(90));
    }

    #[test]
    fn nothing_to_read_is_nothing() {
        for input in ["", "abc", "m", "s", "-5", "1h-30m"] {
            assert_eq!(parse_duration(input), None, "{input:?} parsed");
        }
    }

    /// A minus is part of the number, not a separator. Reading `-5` as `5` would
    /// be a five-second test when the user asked for something impossible.
    #[test]
    fn a_negative_duration_is_refused_rather_than_flipped_to_positive() {
        assert_eq!(parse_duration("-5"), None);
        assert_eq!(parse_duration("-1h"), None);
        assert_eq!(parse_duration("1h-30m"), None);
    }

    /// A duration larger than a `u32` would wrap, and a test that silently lasts
    /// a second is worse than one that is refused.
    #[test]
    fn an_absurd_duration_is_refused_rather_than_wrapped() {
        assert_eq!(parse_duration("9999999999h"), None);
    }

    #[test]
    fn the_window_says_what_it_is_asking_for() {
        assert_eq!(Reason::Command.title(), "commands");
        assert_eq!(Reason::Length(Field::Time).title(), "duration");
        assert_eq!(Reason::Length(Field::Words).title(), "words");
        assert_eq!(Reason::Text.title(), "custom text");
        assert_eq!(Reason::ApeKey.title(), "ape key");
    }

    #[test]
    fn typing_goes_into_the_field() {
        let mut w = window(Reason::Length(Field::Time));
        type_into(&mut w, "1h30m");
        assert_eq!(w.text(), "1h30m");
        w.key(K::Backspace);
        assert_eq!(w.text(), "1h30", "one character should come off");
    }

    #[test]
    fn the_field_empties_one_character_at_a_time_and_stops() {
        let mut w = window(Reason::Text);
        type_into(&mut w, "ab");
        w.key(K::Backspace);
        w.key(K::Backspace);
        w.key(K::Backspace);
        assert_eq!(w.text(), "", "backspacing an empty field underflowed");
    }

    /// The figure under the field is what makes `1h30m` usable: nobody reading a
    /// raw `1h30m` knows it is five thousand four hundred seconds.
    #[test]
    fn a_duration_is_read_back_as_seconds_while_it_is_typed() {
        let mut w = window(Reason::Length(Field::Time));
        assert_eq!(w.preview(), None, "an empty field previews nothing");
        type_into(&mut w, "1h30m");
        assert_eq!(w.preview().as_deref(), Some("5400 seconds"));
    }

    /// And nonsense is said to be nonsense rather than left to be discovered.
    #[test]
    fn a_nonsense_duration_says_so() {
        let mut w = window(Reason::Length(Field::Time));
        type_into(&mut w, "soon");
        assert!(w.preview().unwrap().contains("not a duration"));
        assert_eq!(w.outcome(), Outcome::Invalid("soon".to_owned()));
    }

    /// Zero is endless, which is not a length this client can honour.
    #[test]
    fn zero_is_refused_because_endless_is_what_zen_is_for() {
        let mut w = window(Reason::Length(Field::Time));
        type_into(&mut w, "0");
        assert_eq!(w.outcome(), Outcome::Invalid("0".to_owned()));
        assert!(w.preview().unwrap().contains("endless"));
    }

    #[test]
    fn a_duration_window_returns_the_field_it_was_opened_for() {
        let mut w = window(Reason::Length(Field::WordsCustom));
        type_into(&mut w, "80");
        assert_eq!(w.outcome(), Outcome::Length(Field::WordsCustom, 80));
    }

    /// A command window is a search: submitting runs the highlighted match, not
    /// the literal text.
    #[test]
    fn a_command_window_runs_the_match_it_has_found() {
        let mut w = window(Reason::Command);
        type_into(&mut w, "punc");
        let found = w.selected().expect("a match");
        assert_eq!(w.outcome(), Outcome::Command(found.command));
    }

    /// Nothing found is not "run nothing" — the window stays open so the query
    /// can be corrected.
    #[test]
    fn a_search_that_found_nothing_is_not_a_submission() {
        let mut w = window(Reason::Command);
        type_into(&mut w, "qwertyuiop");
        assert!(w.matches().is_empty());
        assert_eq!(w.outcome(), Outcome::Invalid("qwertyuiop".to_owned()));
    }

    /// An empty command window matches everything, so the first thing highlighted
    /// is the first command — which is what makes the list browsable.
    #[test]
    fn an_empty_command_window_offers_the_whole_list() {
        let w = window(Reason::Command);
        assert_eq!(w.matches().len(), commands::COMMANDS.len());
        assert_eq!(w.selected().map(|m| m.command), Some(0));
    }

    /// The highlight moves and comes back round the other end, so holding down
    /// cycles through the whole list instead of sticking at the bottom — which is
    /// what the site's list does.
    #[test]
    fn the_highlight_moves_and_wraps() {
        let mut w = window(Reason::Command);
        w.key(K::Down);
        assert_eq!(w.selected().map(|m| m.command), Some(1));
        w.key(K::Up);
        assert_eq!(w.selected().map(|m| m.command), Some(0));
        w.key(K::Up);
        let last = commands::COMMANDS.len() - 1;
        assert_eq!(
            w.selected().map(|m| m.command),
            Some(last),
            "up from the first should wrap to the last"
        );
        w.key(K::Down);
        assert_eq!(w.selected().map(|m| m.command), Some(0), "and back again");
    }

    /// A key that is not the window's is left for someone else, so `ctrl+c` still
    /// quits with the window open.
    #[test]
    fn the_highlight_never_falls_off_the_list() {
        let mut w = window(Reason::Command);
        for _ in 0..(commands::COMMANDS.len() * 3) {
            w.key(K::Down);
            assert!(w.selected().is_some(), "the highlight fell off the list");
        }
    }

    /// Editing the query starts again at the top of the *new* list, because a
    /// highlight that was the best match for the old query is not the best match
    /// for this one.
    #[test]
    fn editing_the_query_resets_the_highlight() {
        let mut w = window(Reason::Command);
        w.key(K::Down);
        w.key(K::Down);
        assert_eq!(w.selected().map(|m| m.command), Some(2));
        type_into(&mut w, "q");
        assert_eq!(
            w.selected(),
            w.matches().first().copied(),
            "the highlight stayed where it was in a list that changed"
        );
    }

    #[test]
    fn escape_closes_without_acting() {
        let mut w = window(Reason::Length(Field::Time));
        type_into(&mut w, "90");
        assert_eq!(w.key(K::Esc), Some(Outcome::Cancelled));
    }

    #[test]
    fn enter_submits() {
        let mut w = window(Reason::Length(Field::Time));
        type_into(&mut w, "90");
        assert_eq!(w.key(K::Enter), Some(Outcome::Length(Field::Time, 90)));
    }

    /// A passage keeps the newlines, because the site lets a custom test be more
    /// than one line and only the first is typed.
    #[test]
    fn a_passage_window_keeps_every_line() {
        let mut w = window(Reason::Text);
        type_into(&mut w, "one\ntwo");
        assert_eq!(w.outcome(), Outcome::Text("one\ntwo".to_owned()));
    }

    #[test]
    fn a_passage_counts_its_words() {
        let mut w = window(Reason::Text);
        type_into(&mut w, "one two three");
        assert_eq!(w.preview().as_deref(), Some("3 words"));
    }

    /// A window is never wider than the screen, and never wider than its content
    /// needs, so it is the same shape on a wide terminal and a narrow one.
    #[test]
    fn the_window_fits_the_screen_it_is_on() {
        let w = window(Reason::Command);
        for available in [10u16, 20, 40, 80, 200] {
            let width = w.width(available);
            assert!(width <= available, "{width} > {available}");
            assert!(width > 0);
        }
    }

    /// The list is capped, because a window that grows with the query pushes the
    /// field off the screen at exactly the moment the user is typing.
    #[test]
    fn the_window_has_a_maximum_height() {
        let w = window(Reason::Command);
        assert!(w.height() <= MAX_HEIGHT, "{} rows", w.height());
        assert!(w.height() >= 4, "a window with no field in it");
    }

    /// A key that is not the window's is left for someone else.
    #[test]
    fn an_unrelated_key_is_not_the_windows() {
        let mut w = window(Reason::Length(Field::Time));
        assert_eq!(w.key(K::F(5)), None);
        assert_eq!(w.text(), "", "a function key was typed into the field");
    }
}
