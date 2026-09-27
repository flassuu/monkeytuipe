//! The chrome every centred window in the app shares.
//!
//! There are two of them — the command line and the settings menu — and they were
//! drawn separately, which is how two windows that are supposed to look like one
//! mechanism end up looking like two. Everything they have in common is here: the
//! box, the title, the field, the list, the hint, and the rule about a window that
//! is wider than the screen.
//!
//! The shape is the site's modal, which is a centred box on an opaque backdrop
//! with a title in its top border. Opaque matters: a box drawn *over* the words
//! with the words still legible through it is harder to read than one without,
//! and a user reading a command is looking at the command.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config::theme::Theme;

/// How far above the middle a window sits.
///
/// Slightly above rather than on it, which is where the eye goes first and where
/// the site's modals are. On a tall terminal the difference is a line or two, and
/// it is the difference between a window that is obviously in front and one that
/// looks like it is floating in the middle of the page.
const LIFT: f32 = 0.45;

/// A window's frame, positioned and cleared.
pub struct Chrome {
    /// Where to draw the contents.
    pub inner: Rect,
}

impl Chrome {
    /// Places a window of `width` x `height` in `area` and clears what is under
    /// it.
    ///
    /// Returns `None` when the window cannot be placed at all — no width, or a
    /// height the screen does not have. A window that cannot be drawn is not drawn
    /// at all, because a half-drawn box is a box the user cannot act on.
    pub fn place(
        area: Rect,
        width: u16,
        height: u16,
        title: &str,
        theme: Theme,
        frame: &mut Frame,
    ) -> Option<Self> {
        if width == 0 || height == 0 || width > area.width || height > area.height {
            return None;
        }
        let x = area.x + (area.width - width) / 2;
        let y = area.y + ((area.height - height) as f32 * LIFT) as u16;
        let rect = Rect::new(x, y, width, height);

        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .style(Style::default().fg(theme.foreground).bg(theme.surface))
            .title(format!(" {title} "));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        Some(Self { inner })
    }

    /// Draws lines into the window, clipped to what is left of it.
    pub fn draw(&self, lines: Vec<Line<'static>>, frame: &mut Frame) {
        frame.render_widget(Paragraph::new(lines), self.inner);
    }
}

/// The field at the top of a window: what has been typed, with a block caret on the
/// next cell.
///
/// The caret is a filled block rather than a `|` because the field is one column
/// of a box and a thin bar reads as a border. It blinks on a real terminal and
/// does not here, which is a limitation and not a decision: a blinking caret on a
/// frame-by-frame test backend never looks like anything.
pub fn field(text: &str, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(
        text.to_owned(),
        Style::default().fg(theme.foreground),
    )];
    spans.push(Span::styled("█", Style::default().fg(theme.accent)));
    Line::from(spans)
}

/// One row of a list, highlighted or not.
///
/// Highlighted means the accent colour and bold, with a marker in front — the
/// same rule the top bar uses for its active option, and for the same reason: a
/// background fill on a selected row is fine inside a box, but a marker plus a
/// colour is findable on a monochrome terminal, which a fill is not.
pub fn row(label: &str, selected: bool, theme: Theme) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.foreground)
    };
    Line::from(vec![
        Span::raw(if selected { "▸ " } else { "  " }),
        Span::styled(label.to_owned(), style),
    ])
}

/// A hint line, which is chrome and recedes.
pub fn hint(text: &str, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(
        text.to_owned(),
        Style::default().fg(theme.muted),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn theme() -> Theme {
        crate::config::theme::ThemeName::Gruvbox.resolve()
    }

    /// A window and the buffer it drew into, at a given size.
    fn drawn(
        width: u16,
        height: u16,
        win_w: u16,
        win_h: u16,
    ) -> (ratatui::buffer::Buffer, u16, u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a backend");
        let mut at = (0, 0);
        terminal
            .draw(|frame| {
                let area = frame.area();
                if let Some(chrome) = Chrome::place(area, win_w, win_h, "title", theme(), frame) {
                    chrome.draw(vec![field("hi", theme())], frame);
                    at = (chrome.inner.x, chrome.inner.y);
                }
            })
            .expect("draw");
        (terminal.backend().buffer().clone(), at.0, at.1)
    }

    #[test]
    fn a_window_is_centred_and_sits_slightly_above_the_middle() {
        let (buffer, x, y) = drawn(80, 24, 40, 8);
        // `x` and `y` are the *inner* rectangle, which is one column and one row
        // inside the border: a 40-wide box at column 20 has its content at 21.
        assert_eq!(x, 21, "the window is not centred");
        // Above the middle: the middle would be row 8 for a 24-row screen, and
        // `LIFT` is under a half.
        assert!(y < 9, "the window is not above the middle: {y}");
        // And the title is in the top border, which is the row above the content.
        let mut top = String::new();
        for column in 0..80 {
            top.push_str(buffer[(column, y - 1)].symbol());
        }
        assert!(top.contains("title"), "{top:?}");
        // The border is there too, so the box is a box and not a paragraph.
        assert!(top.contains('╭') || top.contains('┌'), "{top:?}");
    }

    /// A window that cannot be placed is not placed. A half-drawn box is a box the
    /// user cannot act on, and a box clipped by the screen edge is worse than no
    /// box.
    #[test]
    fn a_window_that_does_not_fit_is_not_drawn() {
        let (buffer, x, _) = drawn(80, 24, 100, 8);
        assert_eq!(x, 0, "an over-wide window was placed");
        // Nothing was drawn: the whole screen is blank.
        let mut all = String::new();
        for row in 0..24 {
            for column in 0..80 {
                all.push_str(buffer[(column, row)].symbol());
            }
        }
        assert!(all.trim().is_empty(), "{all:?}");
    }

    /// The window is opaque. A box with the words still legible through it is
    /// harder to read than one without.
    #[test]
    fn a_window_clears_what_is_under_it() {
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).expect("a backend");
        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.render_widget(
                    Paragraph::new("the quick brown fox jumps over the lazy dog and keeps going"),
                    area,
                );
                let _ = Chrome::place(area, 20, 4, "t", theme(), frame);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        // Inside the window's rows, over the word area, nothing of the sentence
        // survives — the `Clear` widget left blanks.
        let mut inside = String::new();
        for row in 1..5 {
            for column in 10..30 {
                inside.push_str(buffer[(column, row)].symbol());
            }
        }
        assert!(
            !inside.contains("quick") && !inside.contains("brown"),
            "the words are legible through the window: {inside:?}"
        );
    }

    #[test]
    fn a_selected_row_is_marked_and_an_unselected_one_is_not() {
        let theme = theme();
        let on = row("thing", true, theme);
        let off = row("thing", false, theme);
        assert_eq!(on.spans[0].content, "▸ ", "no marker on the selected row");
        assert_eq!(off.spans[0].content, "  ");
        assert_ne!(on.spans[1].style, off.spans[1].style);
    }

    #[test]
    fn the_field_ends_in_a_block_caret() {
        let line = field("hi", theme());
        assert_eq!(line.spans[0].content, "hi");
        assert_eq!(line.spans.last().expect("a caret").content, "█");
    }
}
