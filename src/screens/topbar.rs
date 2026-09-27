//! The top bar: the website's settings strip, rebuilt out of keys.
//!
//! The site lays it out as three cards in a `grid-cols-[1fr_auto_1fr]`, so the
//! three groups sit next to each other with a gap and the whole cluster is
//! centred. The two `1fr` columns absorb the leftover width, which is why the
//! left group is right-aligned against the mode group rather than pinned to the
//! edge of the page. That is reproduced here exactly: three cards, a gap, centred.
//!
//! ```text
//! ┌ punctuation numbers ┐┌ time words quote zen custom ┐┌ 15 30 60 120 ⚒ ┐
//! ```
//!
//! ## What is in each card, per mode
//!
//! | mode | left | centre | right |
//! |---|---|---|---|
//! | `time` | punc, numbers | modes | `15 30 60 120 ⚒` |
//! | `words` | punc, numbers | modes | `10 25 50 100 ⚒` |
//! | `quote` | punc, numbers — *disabled* | modes | `all short medium long thicc` |
//! | `zen` | — | modes | — |
//! | `custom` | punc, numbers | modes | `change` |
//!
//! The site *fades* the left and right groups out in zen rather than removing
//! them. A terminal has no opacity, so a card with nothing in it is not drawn and
//! the gap closes — the same result, and no empty box left behind.
//!
//! ## A pressed button is filled, not recoloured
//!
//! The site's bar uses `variant="text"`, where active means "main colour text on
//! no background" — a hue shift. That does not survive a terminal well: a dim
//! grey and a light grey are much harder to tell apart than a filled cell and an
//! empty one, and a hue shift is invisible entirely on a monochrome terminal.
//! So a pressed button here is drawn the way the site's `variant="button"` active
//! state is: accent background, background-coloured text.

use std::fmt::Write as _;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::config::bar::{self, Field};
use crate::config::theme::Theme;
use crate::config::Difficulty;
use crate::config::Mode as ConfigMode;

/// The three cards, in reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// Punctuation and numbers. Right-aligned against the mode card, which is
    /// what `place-self-end` does on the site.
    Left,
    /// The five modes. Always present.
    Centre,
    /// Whatever the mode's length is called.
    Right,
}

impl Block {
    pub const ALL: [Block; 3] = [Block::Left, Block::Centre, Block::Right];
}

/// One button in a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    /// The label as the interface shows it, already translated.
    ///
    /// An owned string rather than a key, because the bar is built from the config
    /// and the config does not know the interface's language — the app does, and it
    /// hands the finished label in. Storing a key here instead would mean every
    /// button needed the app to draw itself, which is the coupling the build-time
    /// split is there to avoid.
    pub label: String,
    /// What it changes.
    pub field: Field,
    pub active: bool,
    /// Shown dimmed and cannot be pressed.
    ///
    /// Quote mode disables punctuation and numbers. The site greys them out *and*
    /// forces them false when switching into quote, so a dimmed button here is
    /// never also an active one.
    pub disabled: bool,
}

impl Button {
    fn width(&self) -> usize {
        self.label.chars().count()
    }
}

/// A whole card: a title and its buttons.
///
/// The cards have no visible title on the site — they are three groups of buttons
/// — so this one is only drawn when there is a heading to give, which is never.
/// It exists so the block is a value rather than a bare list and can be measured
/// and tested.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    pub buttons: Vec<Button>,
}

impl Card {
    /// The card's rendered width, including the gaps and the border.
    ///
    /// `gap` is the space between buttons, `pad` the horizontal padding. A card
    /// is `pad + button + gap + button + … + pad`.
    pub fn width(&self, gap: usize, pad: usize) -> usize {
        if self.buttons.is_empty() {
            return 0;
        }
        let buttons: usize = self.buttons.iter().map(Button::width).sum();
        buttons + gap * (self.buttons.len() - 1) + pad * 2
    }
}

/// The whole bar: three cards and which field is selected.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bar {
    pub left: Card,
    pub centre: Card,
    pub right: Card,
    /// Index of the selected button within the *whole bar*, counting left to right.
    pub selected: usize,
    /// Whether the arrow keys move the selection at all.
    ///
    /// Off while a test is running, because the site hides the whole bar during a
    /// test (`opacity-0 pointer-events-none` when focused) — changing a setting
    /// mid-test would restart it under the typist's hands.
    pub interactive: bool,
}

impl Bar {
    /// Every button, left to right.
    ///
    /// The block is not carried: the three cards are read in order and the
    /// selection is a flat index, which is what a user moving left and right
    /// actually does — there is no "up to the other card".
    pub fn buttons(&self) -> Vec<&Button> {
        self.left
            .buttons
            .iter()
            .chain(self.centre.buttons.iter())
            .chain(self.right.buttons.iter())
            .collect()
    }

    /// The selected button, if there is one.
    pub fn selected_button(&self) -> Option<&Button> {
        self.buttons().into_iter().nth(self.selected)
    }

    /// The field the selection is on, for the app to change.
    pub fn selected_field(&self) -> Option<Field> {
        self.selected_button().map(|b| b.field)
    }

    /// Moves the selection, skipping disabled buttons and staying inside the bar.
    ///
    /// Disabled buttons are jumped over rather than landed on: a selection that
    /// can rest on something that does nothing is a selection that looks broken.
    pub fn move_selection(&mut self, by: isize) {
        if !self.interactive {
            return;
        }
        let buttons: Vec<(bool, Field)> = self
            .buttons()
            .into_iter()
            .map(|b| (b.disabled, b.field))
            .collect();
        if buttons.is_empty() {
            return;
        }
        let last = buttons.len() as isize - 1;
        let mut at = (self.selected as isize).clamp(0, last);
        let mut steps = 0;
        // A full lap always terminates because the bar has a fixed length.
        while steps <= buttons.len() {
            at = (at + by).clamp(0, last);
            steps += 1;
            let disabled = buttons
                .get(at as usize)
                .is_some_and(|(disabled, _)| *disabled);
            if !disabled {
                break;
            }
        }
        self.selected = at as usize;
    }

    /// Activates the selected button, returning the field it was on.
    pub fn activate(&self) -> Option<Field> {
        let button = self.buttons().into_iter().nth(self.selected)?;
        if button.disabled || !self.interactive {
            return None;
        }
        Some(button.field)
    }
}

/// Builds the bar for a configuration.
#[derive(Debug, Clone, Copy)]
pub struct BarState<'a> {
    pub mode: ConfigMode,
    pub punctuation: bool,
    pub numbers: bool,
    pub difficulty: Difficulty,
    pub quote_length: bar::QuoteLength,
    /// Seconds, for the `time` card's active check.
    pub time: u32,
    /// Words, for the `words` card.
    pub words: u32,
    /// Whether a custom passage is set, for the `change` button.
    pub has_custom_text: bool,
    /// The word list. Shown nowhere on the bar — the website's bar has no language
    /// control either — but the settings screen asks the same struct, so it is here
    /// rather than passed twice.
    pub language: &'a str,
    /// The interface language, which is what the labels are in.
    pub ui_language: crate::i18n::Lang,
}

impl Bar {
    /// The bar for a configuration.
    ///
    /// Which cards have anything in them is a rule, not a drawing, and it is the
    /// rule the website encodes as four conditional components.
    pub fn build(state: BarState<'_>) -> Self {
        let mut left = Vec::new();
        let mut right = Vec::new();

        // Quote mode disables both toggles, and zen hides the whole card.
        if state.mode != ConfigMode::Zen {
            let disabled = state.mode == ConfigMode::Quote;
            left.push(Button {
                label: tr(state.ui_language, crate::i18n::Key::Punctuation),
                field: Field::Punctuation,
                active: state.punctuation,
                disabled,
            });
            left.push(Button {
                label: tr(state.ui_language, crate::i18n::Key::Numbers),
                field: Field::Numbers,
                active: state.numbers,
                disabled,
            });
        }

        match state.mode {
            ConfigMode::Time => {
                for seconds in bar::TIMES {
                    right.push(Button {
                        label: seconds.to_string(),
                        field: Field::Time,
                        active: state.time == seconds,
                        disabled: false,
                    });
                }
                // The wrench is active whenever the value is not one of the
                // presets, which is how the site shows that a custom duration is
                // in effect.
                right.push(Button {
                    label: tr(state.ui_language, crate::i18n::Key::Other),
                    field: Field::TimeCustom,
                    active: !bar::TIMES.contains(&state.time),
                    disabled: false,
                });
            }
            ConfigMode::Words => {
                for words in bar::WORD_COUNTS {
                    right.push(Button {
                        label: words.to_string(),
                        field: Field::Words,
                        active: state.words == words,
                        disabled: false,
                    });
                }
                right.push(Button {
                    label: tr(state.ui_language, crate::i18n::Key::Other),
                    field: Field::WordsCustom,
                    active: !bar::WORD_COUNTS.contains(&state.words),
                    disabled: false,
                });
            }
            ConfigMode::Quote => {
                right.push(Button {
                    label: tr(state.ui_language, crate::i18n::Key::QuoteAll),
                    field: Field::QuoteLength,
                    active: state.quote_length == bar::QuoteLength::All,
                    disabled: false,
                });
                for length in bar::QUOTE_LENGTHS.iter().skip(1) {
                    right.push(Button {
                        label: tr(state.ui_language, length.key()),
                        field: Field::QuoteLength,
                        active: state.quote_length == *length,
                        disabled: false,
                    });
                }
            }
            ConfigMode::Custom => {
                right.push(Button {
                    label: tr(
                        state.ui_language,
                        if state.has_custom_text {
                            crate::i18n::Key::Change
                        } else {
                            crate::i18n::Key::Add
                        },
                    ),
                    field: Field::CustomText,
                    active: state.has_custom_text,
                    disabled: false,
                });
            }
            // Zen has no length. The site leaves the card in the DOM at
            // `opacity: 0`; a terminal draws nothing and the gap closes.
            ConfigMode::Zen => {}
        }

        let centre = bar::MODES
            .iter()
            .map(|mode| Button {
                label: tr(state.ui_language, mode.key()),
                field: Field::Mode,
                active: *mode == state.mode,
                disabled: false,
            })
            .collect();

        let mut bar = Self {
            left: Card { buttons: left },
            centre: Card { buttons: centre },
            right: Card { buttons: right },
            selected: 0,
            interactive: true,
        };
        // Start on the mode, which is the one field every test has and the one a
        // user is most likely to want to change.
        bar.selected = bar
            .buttons()
            .iter()
            .position(|b| b.field == Field::Mode)
            .unwrap_or(0);
        bar
    }

    /// Renders the bar into `width` columns, or `None` if it does not fit.
    ///
    /// The mode card is **centred on its own**, which is what the site's
    /// `grid-cols-[1fr_auto_1fr]` does: the two `1fr` columns absorb whatever the
    /// side cards do not use, so the mode buttons stay in the same place whether
    /// the bar has four time presets or one quote length. Centring the whole
    /// cluster instead moves every button each time the mode changes, which is
    /// the one thing a settings bar must not do.
    ///
    /// The left card is right-aligned against the mode card — `place-self-end` —
    /// and the right card is left-aligned against it, so the three read as one
    /// strip that grows in the middle.
    pub fn render(&self, width: u16, theme: Theme) -> Option<Vec<Line<'static>>> {
        if width == 0 {
            return None;
        }
        // Three attempts, in the order they are worth: padded and centred, then
        // unpadded and centred, then unpadded and packed against the left edge.
        // The first is what the site looks like. The second gives up the padding.
        // The third gives up the centring, which costs the mode buttons their
        // fixed place — so it is last, and only for a terminal that could not
        // have the bar otherwise.
        for attempt in [
            Attempt {
                m: Metrics::ROOMY,
                placement: Placement::Centred,
            },
            Attempt {
                m: Metrics::COMPACT,
                placement: Placement::Centred,
            },
            Attempt {
                m: Metrics::COMPACT,
                placement: Placement::Packed,
            },
        ] {
            if let Some(lines) = self.render_at(width, theme, attempt) {
                return Some(lines);
            }
        }
        // Nothing fits on one row. Two rows is better than no bar: the length card
        // moves below the modes rather than the whole strip disappearing. This is
        // what a Russian quote test needs — «все короткие средние длинные толстые»
        // is nine columns wider than `all short medium long thicc` — and it is also
        // what a sixty-column English terminal needs.
        self.render_wrapped(width, theme)
    }

    /// The two-row form: the modes and the toggles on one row, the length on the
    /// next.
    ///
    /// The modes stay centred on the first row, because that is the part of the
    /// bar the user is looking at, and the length is right-aligned under the row
    /// rather than centred — a centred length under a centred mode looks like two
    /// separate bars.
    fn render_wrapped(&self, width: u16, theme: Theme) -> Option<Vec<Line<'static>>> {
        let m = Metrics::COMPACT;
        let used = |card: &Card| card.width(m.gap, m.pad);
        // The first row is the toggles and the modes, and must fit on its own.
        let top = used(&self.left) + used(&self.centre) + m.card_gap;
        let bottom = used(&self.right);
        if top > width as usize || bottom > width as usize || width == 0 {
            return None;
        }
        let counts = [
            self.left.buttons.len(),
            self.centre.buttons.len(),
            self.right.buttons.len(),
        ];
        let (selected_card, within) = selection_in(&counts, self.selected);

        let centre_at = (width as usize).saturating_sub(used(&self.centre)) / 2;
        let left_at = centre_at.saturating_sub(m.card_gap + used(&self.left));
        let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(left_at))];
        let mut at = left_at;
        if !self.left.buttons.is_empty() {
            let card = render_card(
                &self.left.buttons,
                (selected_card == 0).then_some(within),
                theme,
                m,
            );
            at += card.width();
            spans.extend(card.spans);
        }
        spans.push(Span::raw(" ".repeat(centre_at.saturating_sub(at))));
        let centre = render_card(
            &self.centre.buttons,
            (selected_card == 1).then_some(within),
            theme,
            m,
        );
        spans.extend(centre.spans);
        pad_to(&mut spans, width as usize);

        // The length card, right-aligned, on its own row.
        let right = render_card(
            &self.right.buttons,
            (selected_card == 2).then_some(within),
            theme,
            m,
        );
        let right_at = (width as usize).saturating_sub(right.width());
        let mut second: Vec<Span<'static>> = vec![Span::raw(" ".repeat(right_at))];
        second.extend(right.spans);
        pad_to(&mut second, width as usize);

        Some(vec![Line::from(spans), Line::from(second)])
    }

    /// The narrowest the two-row form can be drawn.
    ///
    /// The first row is the toggles and the modes, the second is the length, so
    /// each row is its own constraint and the bar needs the wider of the two. In
    /// practice that is the modes — a five-button card is wider than a length card
    /// in every language — but it is the wider of the two, not an assumption.
    pub fn two_row_minimum(&self) -> usize {
        let m = Metrics::COMPACT;
        let top = self.left.width(m.gap, m.pad) + self.centre.width(m.gap, m.pad) + m.card_gap;
        let bottom = self.right.width(m.gap, m.pad);
        top.max(bottom)
    }

    /// How many rows the bar takes at this width: one if it fits on one, two if it
    /// fits on two, and none if it does not fit at all.
    ///
    /// Asked by the screen to decide how much room to leave, so the answer and the
    /// drawing cannot disagree about it.
    pub fn rows(&self, width: u16) -> u16 {
        let theme = crate::config::theme::ThemeName::Monkeytype.resolve();
        match self.render(width, theme) {
            Some(lines) => u16::try_from(lines.len()).unwrap_or(u16::MAX),
            None => 0,
        }
    }

    /// The narrowest width at which the bar can still be drawn.
    ///
    /// Centring the mode card needs room for the *widest* side card on both
    /// sides of it, so the requirement is not the sum of the three widths — it is
    /// the middle one plus twice the larger outer one.
    pub fn narrowest(&self) -> usize {
        let widest = self
            .left
            .width(Metrics::COMPACT.gap, Metrics::COMPACT.pad)
            .max(self.right.width(Metrics::COMPACT.gap, Metrics::COMPACT.pad));
        let centre = self
            .centre
            .width(Metrics::COMPACT.gap, Metrics::COMPACT.pad);
        centre + Metrics::COMPACT.card_gap * 2 + widest * 2
    }

    /// The width the packed form needs, which is simply the sum of the three
    /// cards and the gaps between them.
    pub fn packed_width(&self) -> usize {
        self.left.width(Metrics::COMPACT.gap, Metrics::COMPACT.pad)
            + self
                .centre
                .width(Metrics::COMPACT.gap, Metrics::COMPACT.pad)
            + self.right.width(Metrics::COMPACT.gap, Metrics::COMPACT.pad)
            + Metrics::COMPACT.card_gap * 2
    }

    /// One attempt at the layout: fixed spacing, fixed placement.
    fn render_at(&self, width: u16, theme: Theme, attempt: Attempt) -> Option<Vec<Line<'static>>> {
        let m = attempt.m;
        let used = |card: &Card| card.width(m.gap, m.pad);
        let (left_w, centre_w, right_w) = (used(&self.left), used(&self.centre), used(&self.right));
        if left_w + centre_w + right_w + m.card_gap * 2 > width as usize {
            return None;
        }

        // Which card the selection is in, and where within it.
        let counts = [
            self.left.buttons.len(),
            self.centre.buttons.len(),
            self.right.buttons.len(),
        ];
        let (selected_card, within) = selection_in(&counts, self.selected);

        // Centred puts the mode card in the middle of the screen and grows the
        // side cards towards the edges; packed lays the three out in order from
        // the left. The mode card's offset is `centre_at` either way, and that is
        // the whole point: within a placement the buttons do not move when the
        // mode changes.
        let (left_at, centre_at, right_at) = match attempt.placement {
            Placement::Centred => {
                let centre_at = (width as usize).saturating_sub(centre_w) / 2;
                if centre_at < m.card_gap + left_w {
                    return None;
                }
                if centre_at + centre_w + m.card_gap + right_w > width as usize {
                    return None;
                }
                (
                    centre_at - m.card_gap - left_w,
                    centre_at,
                    centre_at + centre_w + m.card_gap,
                )
            }
            Placement::Packed => {
                let left_at = 0usize;
                let centre_at = left_at + left_w + m.card_gap;
                (left_at, centre_at, centre_at + centre_w + m.card_gap)
            }
        };

        // Every span is placed at a known offset, and the gaps between the cards
        // are emitted as blanks. They have to be: a card's own padding is one
        // column each side, so without the gap the compact form would put
        // `numbers` and `time` next to each other with nothing between them and
        // the bar would read as one run of text.
        let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(left_at))];
        let mut at = left_at;
        let gap_to = |at: usize, target: usize| Span::raw(" ".repeat(target.saturating_sub(at)));
        if !self.left.buttons.is_empty() {
            let card = render_card(
                &self.left.buttons,
                (selected_card == 0).then_some(within),
                theme,
                m,
            );
            at += card.width();
            spans.extend(card.spans);
        }
        spans.push(gap_to(at, centre_at));
        at = centre_at;
        let centre = render_card(
            &self.centre.buttons,
            (selected_card == 1).then_some(within),
            theme,
            m,
        );
        at += centre.width();
        spans.extend(centre.spans);
        if !self.right.buttons.is_empty() {
            spans.push(gap_to(at, right_at));
            let card = render_card(
                &self.right.buttons,
                (selected_card == 2).then_some(within),
                theme,
                m,
            );
            spans.extend(card.spans);
        }
        let _ = at;
        // Pad out to the full width, so the bar occupies a fixed region instead
        // of one that changes width with its content.
        pad_to(&mut spans, width as usize);
        Some(vec![Line::from(spans)])
    }
}

/// Pads a row out to `width` columns.
///
/// The bar occupies a fixed region rather than one that changes width with its
/// content, which is what lets the screen reserve a fixed number of rows and what
/// stops a trailing space from being dropped somewhere upstream.
fn pad_to(spans: &mut Vec<Span<'static>>, width: usize) {
    let drawn: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if let Some(rest) = width.checked_sub(drawn) {
        spans.push(Span::raw(" ".repeat(rest)));
    }
}

/// Where the mode card sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// In the middle of the screen, as the site's `1fr auto 1fr` does. Needs room
    /// for the widest side card on both sides of the mode card.
    Centred,
    /// In order from the left edge. Needs only the sum of the widths, which is
    /// what makes the bar fit on a terminal the centred form does not.
    Packed,
}

/// One attempt at the bar.
#[derive(Debug, Clone, Copy)]
struct Attempt {
    m: Metrics,
    placement: Placement,
}

/// The spacing of one attempt at the bar.
///
/// Padding and gaps are the first thing to go when the terminal is narrow,
/// because a bar with no padding is still a bar and a bar that does not fit is
/// not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Metrics {
    gap: usize,
    pad: usize,
    card_gap: usize,
}

impl Metrics {
    /// Padding inside a card, a space between buttons, two between cards.
    const ROOMY: Metrics = Metrics {
        gap: 1,
        pad: 1,
        card_gap: 2,
    };
    /// No padding inside a card, but the gap between cards stays: two cards with
    /// a single space between them read as one run of text, and a bar that cannot
    /// be told apart is not a bar.
    const COMPACT: Metrics = Metrics {
        gap: 1,
        pad: 0,
        card_gap: 2,
    };
}

/// Which card the selection falls in, and its index within that card.
///
/// A selection index counts the whole bar, so it has to be mapped onto a card
/// whose button count is different — and a card that is empty is skipped, or the
/// mapping is off by however many buttons it would have had.
fn selection_in(counts: &[usize], selected: usize) -> (usize, usize) {
    let mut left = selected;
    for (index, count) in counts.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        if left < *count {
            return (index, left);
        }
        left -= *count;
    }
    (0, 0)
}

/// Renders one card as a bordered, padded line.
fn render_card(
    buttons: &[Button],
    selected_offset: Option<usize>,
    theme: Theme,
    m: Metrics,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(" ".repeat(m.pad), card_style(theme)));
    for (index, button) in buttons.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ".repeat(m.gap), card_style(theme)));
        }
        spans.push(button_span(button, Some(index) == selected_offset, theme));
    }
    spans.push(Span::styled(" ".repeat(m.pad), card_style(theme)));
    Line::from(spans)
}

/// The card's background, which on the site is `--sub-alt-color`.
///
/// A card needs a background of its own to *be* a card: three runs of text with
/// gaps between them read as one sentence, and the site's grouping is the thing
/// being copied here. `surface` is the slot themes keep distinct from the page,
/// which is exactly what this is.
fn card_style(theme: Theme) -> Style {
    Style::default().fg(theme.surface).bg(theme.background)
}

/// A button: on, off, disabled, and whether the arrows are on it.
///
/// Four states, and they are four different things:
///
/// - **on** — the accent colour, bold. This is the site's `variant="text"`
///   pressed state.
/// - **off** — the muted colour.
/// - **disabled** — muted, and *not* bold even when on. The site dims to
///   `opacity: 0.33`; a terminal has no opacity, so the muted colour is the honest
///   equivalent, and a disabled control that is also the active one would be
///   claiming to be on while refusing to be turned off.
/// - **selected** — foreground instead of muted, and underlined. Underline rather
///   than a fill, because the row of labels is what has to stay readable and an
///   underline marks a position without occupying the cell.
pub fn button_span(button: &Button, selected: bool, theme: Theme) -> Span<'static> {
    let style = if button.disabled {
        Style::default().fg(theme.muted)
    } else if button.active {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else if selected {
        Style::default()
            .fg(theme.foreground)
            .add_modifier(Modifier::UNDERLINED)
    } else {
        Style::default().fg(theme.muted)
    };
    Span::styled(button.label.clone(), style)
}

/// One interface string.
///
/// Every button's label comes through here, so there is exactly one place in this
/// file where a language is consulted.
fn tr(lang: crate::i18n::Lang, key: crate::i18n::Key) -> String {
    lang.tr(key).to_owned()
}

/// Formats a bar for a test, used by the tests and by anything that wants to
/// print one.
pub fn plain_text(bar: &Bar) -> String {
    let mut out = String::new();
    for (index, button) in bar.buttons().into_iter().enumerate() {
        if index > 0 {
            let _ = write!(out, " | ");
        }
        let mark = if button.active { '*' } else { ' ' };
        let _ = write!(out, "{mark}{}", button.label);
    }
    out
}

impl<'a> From<&'a crate::config::Config> for BarState<'a> {
    fn from(config: &'a crate::config::Config) -> Self {
        let test = &config.test;
        BarState {
            mode: test.mode,
            punctuation: test.punctuation,
            numbers: test.numbers,
            difficulty: test.difficulty,
            quote_length: test.quote_length,
            time: test.time,
            words: test.words,
            has_custom_text: !test.custom_text.is_empty(),
            language: &test.language,
            ui_language: config.ui_language,
        }
    }
}

/// Whether the bar fits in `width` columns.
pub fn fits(app: &App, width: u16) -> bool {
    width > 0 && app.bar().render(width, app.theme()).is_some()
}

/// Draws the bar into `area`, or draws nothing if it does not fit.
///
/// Returns whether anything was drawn, because the caller wants to know: a bar
/// that silently did not fit is a bar the user cannot find.
pub fn render(app: &App, area: Rect, theme: Theme, frame: &mut Frame) -> bool {
    if area.width == 0 || area.height == 0 {
        return false;
    }
    let bar = app.bar();
    let Some(lines) = bar.render(area.width, theme) else {
        return false;
    };
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        Rect::new(area.x, area.y, area.width, height),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::QuoteLength;

    use crate::i18n::{Key, Lang};

    fn state(mode: ConfigMode) -> BarState<'static> {
        state_in(mode, Lang::English)
    }

    /// The same, in a given language — which is how a test says "these are the
    /// English labels" without writing them out.
    fn state_in(mode: ConfigMode, lang: Lang) -> BarState<'static> {
        BarState {
            mode,
            punctuation: true,
            numbers: false,
            difficulty: Difficulty::Normal,
            quote_length: QuoteLength::All,
            time: 30,
            words: 25,
            has_custom_text: false,
            language: "english",
            ui_language: lang,
        }
    }

    /// The labels of a block, in order.
    fn labels(card: &Card) -> Vec<String> {
        card.buttons.iter().map(|b| b.label.clone()).collect()
    }

    /// One string out of the catalogue, for a test that wants to name a label.
    fn en(key: Key) -> &'static str {
        Lang::English.tr(key)
    }

    #[test]
    fn the_bar_is_three_cards_with_the_modes_in_the_middle() {
        let bar = Bar::build(state(ConfigMode::Time));
        assert_eq!(labels(&bar.left), [en(Key::Punctuation), en(Key::Numbers)]);
        assert_eq!(
            labels(&bar.centre),
            [
                en(Key::ModeTime),
                en(Key::ModeWords),
                en(Key::ModeQuote),
                en(Key::ModeZen),
                en(Key::ModeCustom),
            ],
            "the site's order, not an alphabetical one"
        );
        assert_eq!(
            labels(&bar.right),
            ["15", "30", "60", "120", en(Key::Other)]
        );
    }

    /// The bar is in the interface's language, and it is the *bar* — the first
    /// thing anyone sees and the thing with the most words on it.
    #[test]
    fn the_bar_is_in_the_configured_language() {
        let bar = Bar::build(state_in(ConfigMode::Time, Lang::Russian));
        assert_eq!(
            labels(&bar.left),
            [
                Lang::Russian.tr(Key::Punctuation),
                Lang::Russian.tr(Key::Numbers)
            ]
        );
        assert_eq!(
            labels(&bar.centre),
            [
                Lang::Russian.tr(Key::ModeTime),
                Lang::Russian.tr(Key::ModeWords),
                Lang::Russian.tr(Key::ModeQuote),
                Lang::Russian.tr(Key::ModeZen),
                Lang::Russian.tr(Key::ModeCustom),
            ]
        );
        assert_eq!(
            labels(&bar.right),
            ["15", "30", "60", "120", Lang::Russian.tr(Key::Other)]
        );
    }

    /// And it still fits: Russian is longer than English, so a bar that fits in
    /// one does not automatically fit in the other. A settings bar that only fits
    /// in English is a settings bar half the users cannot see.
    #[test]
    fn a_russian_bar_fits_in_eighty_columns_too() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Zen,
            ConfigMode::Custom,
        ] {
            let bar = Bar::build(state_in(mode, Lang::Russian));
            assert!(
                bar.render(80, theme).is_some(),
                "the {mode:?} bar does not fit in 80 columns in russian"
            );
        }
    }

    /// The word-list language is *not* translated — a list of languages is written
    /// in the languages it names — so switching the interface language must not
    /// change the word list's name.
    #[test]
    fn switching_the_interface_language_does_not_rename_the_word_list() {
        // The word list's name is not in the catalogue at all, which is the point:
        // a list of languages is written in the languages it names. The bar's
        // *labels* change and the word list does not.
        let english: String =
            labels(&Bar::build(state_in(ConfigMode::Time, Lang::English)).left).concat();
        let russian: String =
            labels(&Bar::build(state_in(ConfigMode::Time, Lang::Russian)).left).concat();
        assert_ne!(english, russian, "the labels did not change language");
        assert!(!english.contains("russian"), "{english}");
    }

    #[test]
    fn each_mode_puts_its_own_length_on_the_right() {
        let cases = [
            (
                ConfigMode::Words,
                vec!["10", "25", "50", "100", en(Key::Other)],
            ),
            (
                ConfigMode::Quote,
                vec![
                    en(Key::QuoteAll),
                    en(Key::QuoteShort),
                    en(Key::QuoteMedium),
                    en(Key::QuoteLong),
                    en(Key::QuoteThicc),
                ],
            ),
            (ConfigMode::Custom, vec![en(Key::Add)]),
            (ConfigMode::Zen, vec![]),
        ];
        for (mode, expected) in cases {
            let bar = Bar::build(state(mode));
            assert_eq!(labels(&bar.right), expected, "{mode:?}");
        }
    }

    /// Zen has nothing to the left of the modes and nothing to the right, so both
    /// cards go. A card with nothing in it is an empty box.
    #[test]
    fn zen_has_no_side_cards() {
        let bar = Bar::build(state(ConfigMode::Zen));
        assert!(bar.left.buttons.is_empty());
        assert!(bar.right.buttons.is_empty());
        assert_eq!(labels(&bar.centre).len(), 5, "the modes are still there");
    }

    /// Quote mode keeps the toggles but cannot use them, which is what the site
    /// does: `disabled` on the buttons *and* `overrideValue` forcing them false.
    #[test]
    fn quote_disables_the_toggles_rather_than_hiding_them() {
        let bar = Bar::build(state(ConfigMode::Quote));
        assert_eq!(labels(&bar.left), ["punctuation", "numbers"]);
        assert!(bar.left.buttons.iter().all(|b| b.disabled));
    }

    #[test]
    fn a_preset_is_active_and_the_custom_button_is_not() {
        let bar = Bar::build(state(ConfigMode::Time));
        let active: Vec<&str> = bar
            .right
            .buttons
            .iter()
            .filter(|b| b.active)
            .map(|b| b.label.as_str())
            .collect();
        assert_eq!(active, ["30"]);
    }

    /// The wrench is active when the value is not a preset, which is how the site
    /// shows that a custom duration is in effect.
    #[test]
    fn a_value_outside_the_presets_lights_the_custom_button() {
        let mut s = state(ConfigMode::Time);
        s.time = 42;
        let bar = Bar::build(s);
        let active: Vec<&str> = bar
            .right
            .buttons
            .iter()
            .filter(|b| b.active)
            .map(|b| b.label.as_str())
            .collect();
        assert_eq!(active, [en(Key::Other)], "42 seconds is a custom duration");
    }

    #[test]
    fn the_selection_starts_on_the_mode() {
        let bar = Bar::build(state(ConfigMode::Time));
        assert_eq!(bar.selected_field(), Some(Field::Mode));
    }

    /// A selection that can rest on a disabled button is a selection that looks
    /// broken, so the arrows jump over them.
    #[test]
    fn the_selection_skips_disabled_buttons() {
        let mut bar = Bar::build(state(ConfigMode::Quote));
        // Land on punctuation, which is disabled in quote mode.
        bar.selected = 0;
        bar.move_selection(1);
        assert_ne!(
            bar.selected_field(),
            Some(Field::Punctuation),
            "the selection rested on a disabled button"
        );
    }

    #[test]
    fn the_selection_never_escapes_the_bar() {
        let mut bar = Bar::build(state(ConfigMode::Time));
        for _ in 0..20 {
            bar.move_selection(-1);
        }
        assert_eq!(bar.selected, 0);
        for _ in 0..40 {
            bar.move_selection(1);
        }
        assert_eq!(bar.selected, bar.buttons().len() - 1);
    }

    /// The bar is hidden during a test, so the arrows must not act on it either.
    #[test]
    fn an_inert_bar_does_not_move() {
        let mut bar = Bar::build(state(ConfigMode::Time));
        bar.interactive = false;
        let before = bar.selected;
        bar.move_selection(1);
        assert_eq!(bar.selected, before);
        assert_eq!(bar.activate(), None, "an inert bar still activated a field");
    }

    fn button(active: bool, selected: bool) -> ratatui::text::Span<'static> {
        button_span(
            &Button {
                label: "punctuation".to_owned(),
                field: Field::Punctuation,
                active,
                disabled: false,
            },
            selected,
            crate::config::theme::ThemeName::Gruvbox.resolve(),
        )
    }

    /// The active option is a highlighted word: the accent colour, bold. No
    /// background, because a run of filled cells stops reading as a row of buttons.
    #[test]
    fn the_active_option_is_a_highlighted_word_and_not_a_fill() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let on = button(true, false);
        let off = button(false, false);
        assert!(
            on.style.bg.is_none(),
            "the active option is drawn with a fill"
        );
        assert_eq!(
            on.style.fg,
            Some(theme.accent),
            "the active text is not the accent"
        );
        assert!(
            on.style.add_modifier.contains(Modifier::BOLD),
            "the active text is not bold"
        );
        assert_eq!(
            off.style.fg,
            Some(theme.muted),
            "an inactive option is not muted"
        );
        assert_ne!(on.style, off.style);
    }

    /// The arrows are somewhere different from "on", and the two have to be
    /// distinguishable — an arrow on an inactive option must not look like an
    /// arrow on an active one, or there is no telling which is which.
    #[test]
    fn the_selection_and_the_active_state_can_be_told_apart() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let selected_off = button(false, true);
        let active = button(true, true);
        let inactive_untouched = button(false, false);
        assert_ne!(
            selected_off.style, inactive_untouched.style,
            "there is no way to see where the arrows are"
        );
        assert_ne!(selected_off.style, active.style);
        assert_eq!(selected_off.style.fg, Some(theme.foreground));
    }

    /// A disabled control that is also the active one is a control claiming to be
    /// on while refusing to be turned off. Quote mode produces exactly that, and
    /// the site forces both toggles false on entry so it does not.
    #[test]
    fn a_disabled_option_is_not_drawn_as_active() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let style = button_span(
            &Button {
                label: "punctuation".to_owned(),
                field: Field::Punctuation,
                active: true,
                disabled: true,
            },
            false,
            theme,
        )
        .style;
        assert_eq!(style.fg, Some(theme.muted));
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    /// A bar that cannot be laid out is not drawn half. A truncated card looks
    /// like a card with a missing button, and a card with a button missing is worse
    /// than a bar on two rows.
    #[test]
    fn a_bar_too_narrow_for_either_form_is_not_drawn_at_all() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [ConfigMode::Time, ConfigMode::Quote, ConfigMode::Zen] {
            for lang in [Lang::English, Lang::Russian] {
                let bar = Bar::build(state_in(mode, lang));
                // Not one column under the two-row form's own minimum.
                for width in 0..bar.two_row_minimum() as u16 {
                    assert!(
                        bar.render(width, theme).is_none(),
                        "the {mode:?} bar in {lang:?} was drawn truncated at {width} columns"
                    );
                }
                // And at its minimum it draws, on two rows.
                assert_eq!(
                    bar.rows(bar.two_row_minimum() as u16),
                    2,
                    "{mode:?} in {lang:?} did not draw at its two-row minimum"
                );
            }
        }
    }

    /// Two rows is the last form before giving up, and it is what a language with
    /// longer words needs: «все короткие средние длинные толстые» is nine columns
    /// wider than `all short medium long thicc`, which is the difference between
    /// having a bar and not having one.
    #[test]
    fn a_bar_that_does_not_fit_on_one_row_moves_the_length_below() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let russian = Bar::build(state_in(ConfigMode::Quote, Lang::Russian));
        let lines = russian
            .render(80, theme)
            .expect("the bar fits in eighty columns, on one row or two");
        assert_eq!(
            lines.len(),
            2,
            "the length card did not move to its own row"
        );
        assert_eq!(
            lines.len(),
            2,
            "the length card did not move to its own row"
        );
        let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let second: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            first.contains("дзен"),
            "the modes are not on the first row: {first}"
        );
        assert!(
            !first.contains("толстые"),
            "the length did not move: {first}"
        );
        assert!(
            second.contains("толстые"),
            "the length is not on the second row: {second}"
        );
        // And the modes stay centred, because that is the part being looked at.
        let at = first.find("дзен").expect("the modes");
        assert!(
            at > 20,
            "the modes are not centred on the first row: {at} in {first}"
        );
    }

    /// The number of rows the screen reserves is the number of rows the bar draws,
    /// asked of the same code.
    #[test]
    fn the_reserved_rows_match_the_drawn_rows() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for (mode, lang) in [
            (ConfigMode::Time, Lang::English),
            (ConfigMode::Quote, Lang::English),
            (ConfigMode::Quote, Lang::Russian),
            (ConfigMode::Time, Lang::Russian),
        ] {
            let bar = Bar::build(state_in(mode, lang));
            for width in [40u16, 60, 70, 76, 80, 92, 200] {
                let drawn = bar.render(width, theme).map_or(0, |l| l.len() as u16);
                assert_eq!(
                    bar.rows(width),
                    drawn,
                    "{mode:?} in {lang:?} at {width} columns: reserved and drew differently"
                );
            }
        }
    }

    /// Padding is the first thing to go, because a bar with no padding is still a
    /// bar. 80 columns is the width people are told to aim for, so the bar has to
    /// fit there.
    #[test]
    fn the_bar_fits_the_terminal_width_people_are_told_to_aim_for() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Custom,
            ConfigMode::Zen,
        ] {
            let bar = Bar::build(state(mode));
            assert!(
                bar.render(80, theme).is_some(),
                "the {mode:?} bar does not fit in 80 columns"
            );
        }
    }

    /// Padding is what makes the bar wider, and it is the first thing given up:
    /// the compact form is exactly six columns narrower, two per card.
    #[test]
    fn padding_is_what_makes_the_bar_wider() {
        let bar = Bar::build(state(ConfigMode::Time));
        let roomy = Metrics::ROOMY.card_gap * 2
            + bar.left.width(Metrics::ROOMY.gap, Metrics::ROOMY.pad)
            + bar.centre.width(Metrics::ROOMY.gap, Metrics::ROOMY.pad)
            + bar.right.width(Metrics::ROOMY.gap, Metrics::ROOMY.pad);
        let compact = Metrics::COMPACT.card_gap * 2
            + bar.left.width(Metrics::COMPACT.gap, Metrics::COMPACT.pad)
            + bar.centre.width(Metrics::COMPACT.gap, Metrics::COMPACT.pad)
            + bar.right.width(Metrics::COMPACT.gap, Metrics::COMPACT.pad);
        assert!(roomy > compact, "{roomy} is not wider than {compact}");
        assert_eq!(roomy - compact, 6, "the difference is not the padding");
    }

    /// Centring the mode card costs more than the sum of the widths, because the
    /// widest side card has to fit on *both* sides of it. That is the number the
    /// terminal has to be at least.
    #[test]
    fn centring_costs_more_than_the_sum_of_the_parts() {
        let bar = Bar::build(state(ConfigMode::Quote));
        let packed = bar.packed_width();
        let narrowest = bar.narrowest();
        assert!(
            narrowest > packed,
            "{narrowest} should be more than the packed {packed}"
        );
        // And the packed form is what brings quote mode inside 80 columns.
        assert!(
            packed <= 80,
            "the packed quote bar is {packed} columns wide"
        );
    }

    /// Centring the mode card costs more than the sum of the widths, so there is a
    /// width at which the bar is drawn on one row and one column less at which it
    /// is drawn on two. Neither is "not drawn" — only the two-row form failing is.
    #[test]
    fn one_column_less_moves_the_bar_to_two_rows_rather_than_away() {
        let bar = Bar::build(state(ConfigMode::Quote));
        let narrowest = bar.narrowest().min(bar.packed_width()) as u16;
        assert_eq!(
            bar.rows(narrowest),
            1,
            "it fits on one row at its own width"
        );
        assert_eq!(
            bar.rows(narrowest - 1),
            2,
            "one column less and it moved a row"
        );
    }

    #[test]
    fn a_rendered_bar_is_exactly_as_wide_as_it_claims() {
        let bar = Bar::build(state(ConfigMode::Time));
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let needed = bar.narrowest();
        for width in needed..(needed + 10) {
            let width = u16::try_from(width).expect("a small width");
            let lines = bar.render(width, theme).expect("it fits");
            let line = &lines[0];
            let used: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(used, width as usize, "width {width}");
        }
    }

    /// The mode buttons must not move when a mode hides one of the side cards —
    /// that is what the site's `1fr auto 1fr` grid does, and it is why the bar
    /// feels like a bar rather than a list that rewraps.
    #[test]
    fn the_modes_do_not_move_between_modes() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let offsets = [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Custom,
        ]
        .map(|mode| {
            let bar = Bar::build(state(mode));
            let lines = bar.render(200, theme).expect("it fits");
            let line = &lines[0];
            // Each button is its own span, so the mode card starts at the
            // first button labelled exactly "time".
            let mut at = 0usize;
            for span in &line.spans {
                if span.content.as_ref() == "time" {
                    return at;
                }
                at += span.content.chars().count();
            }
            panic!("the mode card is not in the line");
        });
        assert!(
            offsets.windows(2).all(|w| w[0] == w[1]),
            "the modes moved: {offsets:?}"
        );
    }

    #[test]
    fn zen_draws_only_the_modes() {
        let bar = Bar::build(state(ConfigMode::Zen));
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let lines = bar.render(80, theme).expect("it fits");
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("zen"), "{text}");
        assert!(!text.contains("punctuation"), "{text}");
        assert!(!text.contains("thicc"), "{text}");
    }

    #[test]
    fn plain_text_names_every_button() {
        let bar = Bar::build(state(ConfigMode::Time));
        let text = plain_text(&bar);
        for label in [
            "punctuation",
            "numbers",
            "time",
            "words",
            "quote",
            "zen",
            "custom",
        ] {
            assert!(text.contains(label), "{label} is missing from {text}");
        }
    }
}
