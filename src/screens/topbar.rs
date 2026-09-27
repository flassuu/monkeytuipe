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

/// The Nerd Font glyph in front of each kind of button.
///
/// Every entry is a *name and a codepoint together*, because they are one fact and
/// writing them apart is how four of the eight were wrong for a release: the names
/// were the ones people use on the cheat sheet, the codepoints were written from
/// memory, and the two were never checked against each other. U+F0F7 was labelled
/// `nf-fa-mountain` and is `building_o`; U+F1E0 was `nf-fa-at` and is
/// `share_nodes`; U+F584 was `screwdriver_wrench` and is not in the font at all.
///
/// The codepoints below were read out of the upstream `MaterialDesignIconsDesktop.ttf`
/// and `FontAwesome.otf` cmap tables, not recalled. Both sets are checked by
/// [`the_icon_table_names_the_glyphs_it_uses`], which pins the name against the
/// codepoint so a future edit has to move both or fail.
///
/// Not translated and not configurable. A glyph is a name for a thing, the same way
/// `gruvbox` is a name in both languages, and the whole point is that the bar is
/// scannable — an icon set a user's font does not have is a row of empty boxes.
pub mod icon {
    /// `(nerd font name, the character it maps to)`.
    ///
    /// The character is written as an escape rather than pasted, because a pasted
    /// private-use character is invisible in a diff and in review — the one place
    /// where a wrong glyph would otherwise be cheapest to introduce.
    pub const TABLE: [(&str, &str); 8] = [
        // A dog, for punctuation. `at` would have been the obvious choice and its
        // codepoint is U+F1FA, not the U+F1E0 that `share_nodes` lives at.
        ("md-dog", "\u{f0a43}"),
        // `fa-hashtag`, unchanged and verified.
        ("fa-hashtag", "\u{f292}"),
        // A filled clock with the hand at two. `md-clock-time-two` is the filled one
        // and `md-clock-time-two-outline` the outline; `md-clock-time-four` — what
        // this was labelled as — is U+F1442, not the U+F1142 that
        // `minus-box-multiple-outline` lives at.
        ("md-clock-time-two", "\u{f1440}"),
        // `fa-font`, unchanged and verified.
        ("fa-font", "\u{f031}"),
        // The *left* quote mark, for a published passage. This was U+F10E, which is
        // `quote_right`; the left one is U+F10D.
        ("fa-quote_left", "\u{f10d}"),
        // A mountain, for the test with no end. Neither icon set has a rock: MDI
        // dropped `mountain` in v3.0.0 and has no replacement, and Font Awesome has
        // only this one. A mountain is a rock, so it is the honest nearest thing.
        ("fa-mountain", "\u{ef08}"),
        // `fa-wrench`, unchanged and verified.
        ("fa-wrench", "\u{f0ad}"),
        // A screwdriver crossed with a wrench — the classic "hand tools" glyph, and
        // the nearest single character to a key and a screwdriver, since no set has
        // both in one glyph. The codepoint is U+EF70; U+F584 is not in the font.
        ("fa-screwdriver_wrench", "\u{ef70}"),
    ];

    /// A dog's head, for punctuation.
    pub const PUNCTUATION: &str = "\u{f0a43}";
    /// A `#`, for numbers.
    pub const NUMBERS: &str = "\u{f292}";
    /// A clock reading two, for the timed test.
    pub const TIME: &str = "\u{f1440}";
    /// A letter, for counting words.
    pub const WORDS: &str = "\u{f031}";
    /// A quotation mark, for a published passage.
    pub const QUOTE: &str = "\u{f10d}";
    /// A mountain, for the test with no end.
    pub const ZEN: &str = "\u{ef08}";
    /// A wrench, for a passage of the user's own.
    pub const CUSTOM: &str = "\u{f0ad}";
    /// A screwdriver and a wrench, for a length that is not a preset.
    pub const OTHER: &str = "\u{ef70}";

    /// The glyph for a test mode, or nothing for a length button.
    ///
    /// A function rather than a table lookup because the five mode buttons all share
    /// one [`crate::config::bar::Field`] — `Field::Mode` says *which kind of setting*
    /// and not *which setting* — so a table keyed on the field would give all five
    /// the same glyph.
    pub fn for_mode(mode: crate::config::Mode) -> &'static str {
        match mode {
            crate::config::Mode::Time => TIME,
            crate::config::Mode::Words => WORDS,
            crate::config::Mode::Quote => QUOTE,
            crate::config::Mode::Zen => ZEN,
            crate::config::Mode::Custom => CUSTOM,
        }
    }
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
    /// The Nerd Font glyph in front of the label, or empty for a length.
    ///
    /// Carried rather than looked up, for the same reason the label is: the render
    /// must not have to know which mode a button is to draw it, or the same button
    /// would need two answers depending on who asked.
    pub icon: &'static str,
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
    /// The button's drawn width: the glyph, a space, and the label.
    ///
    /// The glyph is counted by characters, not bytes, because a Nerd Font glyph is
    /// three bytes of UTF-8 and one cell — counting bytes is how a bar with icons
    /// ends up a third wider than the space it was given.
    ///
    /// `icons` is false when the terminal is too narrow to afford them, and then the
    /// glyph costs nothing at all rather than being drawn and clipped.
    fn width(&self, icons: bool) -> usize {
        let label = self.label.chars().count();
        if !icons || self.icon.is_empty() {
            return label;
        }
        label + self.icon.chars().count() + 1
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
    pub fn width(&self, gap: usize, pad: usize, icons: bool) -> usize {
        if self.buttons.is_empty() {
            return 0;
        }
        let buttons: usize = self.buttons.iter().map(|b| b.width(icons)).sum();
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
                icon: icon::PUNCTUATION,
                field: Field::Punctuation,
                active: state.punctuation,
                disabled,
            });
            left.push(Button {
                label: tr(state.ui_language, crate::i18n::Key::Numbers),
                icon: icon::NUMBERS,
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
                        icon: "",
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
                    icon: icon::OTHER,
                    field: Field::TimeCustom,
                    active: !bar::TIMES.contains(&state.time),
                    disabled: false,
                });
            }
            ConfigMode::Words => {
                for words in bar::WORD_COUNTS {
                    right.push(Button {
                        label: words.to_string(),
                        icon: "",
                        field: Field::Words,
                        active: state.words == words,
                        disabled: false,
                    });
                }
                right.push(Button {
                    label: tr(state.ui_language, crate::i18n::Key::Other),
                    icon: icon::OTHER,
                    field: Field::WordsCustom,
                    active: !bar::WORD_COUNTS.contains(&state.words),
                    disabled: false,
                });
            }
            ConfigMode::Quote => {
                right.push(Button {
                    label: tr(state.ui_language, crate::i18n::Key::QuoteAll),
                    icon: "",
                    field: Field::QuoteLength,
                    active: state.quote_length == bar::QuoteLength::All,
                    disabled: false,
                });
                for length in bar::QUOTE_LENGTHS.iter().skip(1) {
                    right.push(Button {
                        label: tr(state.ui_language, length.key()),
                        icon: "",
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
                    icon: "",
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
                icon: icon::for_mode(*mode),
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
    /// The rows the bar occupies when it is drawn: a rule, the settings, a rule.
    ///
    /// Three, always, or nothing at all. It was five — a rule, a blank, the settings,
    /// a blank, a rule — and the blank rows were the right idea attached to the wrong
    /// amount: they made the box a *panel* with air in it, when what is wanted is a
    /// strip of settings inside a border, with the border tight to the text. A blank
    /// row inside a one-line box is a hole, and a hole is not padding.
    pub const ROWS: u16 = 3;

    /// Renders the bar into `width` columns, or `None` if it does not fit.
    ///
    /// One box, three cells, full width. Each cell holds one group of buttons,
    /// centred in its own third, with a divider between them:
    ///
    /// ```text
    /// ╭──────────────────────────────────────────╮
    /// │                   │                      │
    /// │  pun num           │  time words quote …  │  15 30 60 120 other
    /// │                   │                      │
    /// ╰──────────────────────────────────────────╯
    /// ```
    ///
    /// This replaced a four-step degradation ladder — padded and centred, unpadded
    /// and centred, packed against the left edge, then two rows — and the ladder
    /// was a mistake twice over. Each step asked the same question of the same
    /// cells, so there were four places where the answer could be wrong; and every
    /// step but the first moved the buttons, so the mode card was not in the same
    /// place twice on a terminal narrower than ninety columns. A settings bar whose
    /// controls move when you resize the window is a bar you have to re-find.
    ///
    /// Now the box is the full width, the cells take their own content and share the
    /// slack, and there is one question left: does it fit. It does not, the bar is
    /// not drawn — a truncated bar hides settings, and hiding a setting silently is
    /// worse than not showing it at all.
    ///
    /// The one concession to narrow terminals is the glyphs. They are decoration and
    /// the labels are content, so when the glyphs would cost the bar entirely they
    /// are the thing that goes: a bar without icons on a 92-column terminal beats no
    /// bar at all, and the icons come back the moment there is room. This is the one
    /// degradation step, and it is a single boolean rather than a ladder — nothing
    /// moves when it happens, which was the ladder's real sin.
    pub fn render(&self, width: u16, theme: Theme) -> Option<Vec<Line<'static>>> {
        let width = width as usize;
        let icons = width >= self.narrowest();
        if !icons && width < self.narrowest_with(false) {
            return None;
        }

        // Two borders and two dividers are not content. Each cell also reserves a
        // space against each of its two edges, so even at the narrowest width that
        // draws there is air next to every divider.
        let inner = width - 4;
        // The centre cell is its own content plus a space each side. The sides take
        // whatever is left, equally.
        //
        // Neither number may depend on the *other* cells' content, and that is the
        // whole trick. The five mode buttons are the same five buttons in every mode
        // — same labels, same glyphs — so the centre's width is the same whatever the
        // mode, and the sides follow from it without looking at them. The mode
        // buttons therefore cannot move when the mode changes.
        //
        // An earlier version shared the leftover space by weight across all three
        // cells, which made the centre depend on how wide the *side* cells were, and
        // the side cells hold different things in different modes — so the modes
        // jumped about four columns every time the mode changed. A settings bar whose
        // controls move when you change a setting is a bar you have to find again.
        let centre_w = self.centre.width(CELL_GAP, 0, icons) + 2;
        let side_w = inner.saturating_sub(centre_w) / 2;
        // An odd column goes to the left, so the right-hand divider is as close to
        // the edge as the geometry allows and the left-hand padding absorbs the rest.
        let mut cells = [side_w + (inner - centre_w - side_w * 2), centre_w, side_w];
        // What each cell needs, which is what the spare is measured against. The
        // centre's air is taken from the sides, so the sides' needs have to be known
        // before the row is laid out.
        //
        // A side cell needs one column, not two: it is centred, so a single spare
        // column lands on one side of its text, and the site's `place-self-end` puts
        // it against the divider, which is where the space is wanted. The centre
        // needs two, because it has a divider on *both* sides of it. The difference is
        // three columns across the whole bar, and three columns is the difference
        // between a Russian bar drawing at eighty and not drawing at all.
        let needed: [usize; 3] = [
            self.left.width(CELL_GAP, 0, icons) + 1,
            centre_w,
            self.right.width(CELL_GAP, 0, icons) + 1,
        ];

        let counts = [
            self.left.buttons.len(),
            self.centre.buttons.len(),
            self.right.buttons.len(),
        ];
        let (selected_cell, within) = selection_in(&counts, self.selected);
        let cards = [&self.left, &self.centre, &self.right];

        let content: Vec<Vec<Span<'static>>> = cards
            .iter()
            .enumerate()
            .map(|(index, card)| {
                render_cell(
                    &card.buttons,
                    (selected_cell == index).then_some(within),
                    theme,
                    icons,
                )
            })
            .collect();

        // The one row with anything in it. Its text is centred in its own cell — all
        // three, rather than the two sides hugging the middle the way the site's
        // `place-self-end` does — because a cell flush against the outer border and a
        // cell flush against the centre read as two different kinds of thing, and a
        // bar of three different things is not a bar.
        // The centre gets some air, taken from the two side cells' leftover. They
        // are sized to what is in them rather than to the screen, so at any width
        // that drew the bar at all they have some to give — and at the narrowest
        // width that draws they have none, and the air is simply zero.
        let spare = cells[0].saturating_sub(needed[0]) + cells[2].saturating_sub(needed[2]);
        let air = CENTRE_AIR.min(spare / 2) * 2;
        cells[0] -= air / 2;
        cells[2] -= air / 2;

        let row = {
            let mut spans: Vec<Span<'static>> = vec![Span::styled(BORDER, frame_style(theme))];
            for (index, cell_width) in cells.iter().enumerate() {
                let cell = &content[index];
                let used: usize = cell.iter().map(|s| s.content.chars().count()).sum();
                let slack = cell_width.saturating_sub(used);
                spans.push(Span::raw(" ".repeat(slack / 2)));
                spans.extend(cell.iter().cloned());
                spans.push(Span::raw(" ".repeat(slack - slack / 2)));
                if index < 2 {
                    spans.push(Span::styled(BORDER, frame_style(theme)));
                }
            }
            spans.push(Span::styled(BORDER, frame_style(theme)));
            Line::from(spans)
        };

        // The top and bottom rules: one continuous run between two corners, so
        // `width - 2` and not `width - 4`. Getting that wrong draws a box two columns
        // narrower than the rows inside it.
        let rule = |left: &'static str, right: &'static str| {
            Line::from(vec![
                Span::styled(left, frame_style(theme)),
                Span::styled("─".repeat(width - 2), frame_style(theme)),
                Span::styled(right, frame_style(theme)),
            ])
        };

        Some(vec![rule("╭", "╮"), row, rule("╰", "╯")])
    }

    /// The narrowest width at which the box can still be drawn, glyphs and all.
    ///
    /// The *sum* of the three cells, not the widest cell times three: each cell is
    /// sized to its own content, and the slack only exists if there is more width
    /// than content. The sides do not have to be as wide as the centre, because a
    /// cell is only as wide as what is in it.
    pub fn narrowest(&self) -> usize {
        self.narrowest_with(true)
    }

    /// [`Self::narrowest`] with a choice about the glyphs.
    ///
    /// Two numbers rather than one, because the glyphs are optional and the bar's
    /// minimum width is therefore two different facts: the width at which it can be
    /// drawn as designed, and the width at which it can be drawn at all.
    pub fn narrowest_with(&self, icons: bool) -> usize {
        // The centre plus its own margins, and the wider side cell plus its margins
        // on *both* sides — the two side cells are the same width, so the narrower
        // one has to fit in the wider one's space.
        //
        // The centre's extra air is not in here, and that is the point: it is taken
        // from whatever the side cells have left over, and at this width they have
        // nothing. If the air were a fixed part of the centre the bar would need four
        // more columns than it does, and an eighty-column terminal would lose the bar
        // altogether rather than losing two columns of padding.
        let centre = self.centre.width(CELL_GAP, 0, icons) + 2;
        let side = self
            .left
            .width(CELL_GAP, 0, icons)
            .max(self.right.width(CELL_GAP, 0, icons))
            + 1;
        centre + side * 2 + 4
    }

    /// How many rows the screen should reserve for the bar.
    ///
    /// [`Self::ROWS`], or nothing when the bar cannot be drawn — the screen asks
    /// this rather than assuming, so a terminal too narrow for the box does not
    /// reserve five empty rows with nothing in them.
    pub fn rows(&self, width: u16) -> u16 {
        if self
            .render(width, crate::config::theme::ThemeName::Monkeytype.resolve())
            .is_some()
        {
            Self::ROWS
        } else {
            0
        }
    }
}

// The ladder this replaced, kept as a note rather than as code.
//
// `Placement`, `Attempt` and `Metrics` existed to try four layouts in turn: padded
// and centred, unpadded and centred, packed against the left edge, then two rows
// with the lengths below. They are gone, and the reason is worth more than the code.
//
// Each step asked the same question of the same three cells, so there were four
// places where the answer could be wrong and no way to tell from the output which
// one had been used. And every step but the first moved the buttons: on a terminal
// narrower than about ninety columns the mode card was not in the same place twice,
// so a settings bar's controls moved when the window was resized. A control that
// moves is a control the user has to find again.
//
// One box, `1fr auto 1fr`, full width, and no bar at all when the content does not
// fit. The only question left is the one worth asking.

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

/// The vertical divider, which is also the left and right edge of the box.
///
/// One constant because the same character is the frame and the separator, and two
/// literals for one line of drawing is how a box ends up with rounded corners and
/// square edges.
const BORDER: &str = "│";

/// The space between two buttons in a cell.
const CELL_GAP: usize = 1;

/// The extra air inside the centre cell, on each side of its content.
///
/// The centre holds the five modes, so it is already the widest block by
/// arithmetic; this is what makes it *look* like the main one rather than merely
/// measuring like it.
///
/// It is taken from the side cells' leftover rather than added to the bar's minimum
/// width, which is why it costs nothing when the terminal is tight: the side cells
/// are sized to what is in them, so at the narrowest width that draws there is no
/// leftover to take and the air is simply zero. Adding it to the minimum instead
/// would have cost four columns, and four columns is the difference between an
/// eighty-column terminal drawing the bar and drawing nothing.
const CENTRE_AIR: usize = 3;

/// Renders one cell's buttons.
///
/// The cell's width is not passed in: the caller knows the three cell widths and is
/// the authority on where the dividers go, and it centres each cell's spans against
/// its own width. A cell that padded itself would have to agree with the caller
/// about the same number, and two places holding one number is how a divider ends up
/// a column out.
fn render_cell(
    buttons: &[Button],
    selected_offset: Option<usize>,
    theme: Theme,
    icons: bool,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (index, button) in buttons.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" ".repeat(CELL_GAP)));
        }
        spans.push(button_span(
            button,
            Some(index) == selected_offset,
            theme,
            icons,
        ));
    }
    spans
}

/// The frame's colour: the muted one, so the box is present without competing with
/// the settings inside it.
fn frame_style(theme: Theme) -> Style {
    Style::default().fg(theme.muted)
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
/// - **selected** — the foreground rather than the muted colour, and bold.
///
/// The active and selected states are colour and weight only. They used to be
/// underlined too, which was wrong twice over: a row of labels with rules under
/// some of them reads as a table of links rather than a row of settings, and on the
/// `auto` theme — where the foreground sits very close to the muted colour — the
/// underline was doing most of the work of saying which button the arrows were on.
///
/// `icons` is false on a terminal too narrow for the glyphs, and then the label is
/// drawn alone rather than being pushed off the end of its cell. See
/// [`Bar::render`].
pub fn button_span(button: &Button, selected: bool, theme: Theme, icons: bool) -> Span<'static> {
    let style = if button.disabled {
        Style::default().fg(theme.muted)
    } else if button.active {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if selected {
        Style::default()
            .fg(theme.foreground)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted)
    };
    // The glyph and the label are one span, so the two can never be styled
    // differently — a bold label must not leave a dim icon sitting in front of it.
    let text = if !icons || button.icon.is_empty() {
        button.label.clone()
    } else {
        format!("{} {}", button.icon, button.label)
    };
    Span::styled(text, style)
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

    /// Eighty columns is the width people are told to aim for, so the bar has to fit
    /// there — and the glyphs are what it gives up to do it.
    ///
    /// Quote mode is the exception, in both languages, and it is a property of the
    /// words rather than of the layout: the lengths are `all short medium long thicc`
    /// in English and «все короткие средние длинные толстые» in Russian, and the bar
    /// has to fit the longer one *and* leave the modes somewhere to sit. Eighty-nine
    /// columns is the honest minimum for English and a hundred and fourteen for Russian.
    /// The old
    /// layout answered this by moving the lengths onto a second row, which is a bar
    /// that changes shape as the window narrows — the thing this design exists to
    /// stop.
    #[test]
    fn the_bar_fits_in_eighty_columns_for_everything_but_a_quote_test() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Zen,
            ConfigMode::Custom,
        ] {
            for lang in [Lang::English, Lang::Russian] {
                let bar = Bar::build(state_in(mode, lang));
                let without = bar.narrowest_with(false);
                if mode == ConfigMode::Quote {
                    let stated = match lang {
                        Lang::English => 89,
                        Lang::Russian => 114,
                    };
                    assert_eq!(without, stated, "the {lang:?} quote minimum moved");
                    continue;
                }
                assert!(
                    without <= 80,
                    "the {mode:?} bar in {lang:?} needs {without} columns without its glyphs"
                );
                assert!(
                    bar.render(80, theme).is_some(),
                    "the {mode:?} bar in {lang:?} was not drawn at 80 columns"
                );
            }
        }
    }

    /// The glyphs are the only thing between a bar that fits and one that does not,
    /// and dropping them costs nothing else: the same labels, the same selection,
    /// the same five rows, nothing moved and nothing cut off.
    #[test]
    fn the_glyphs_are_dropped_before_the_bar_is() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let bar = Bar::build(state(ConfigMode::Time));
        let with_icons = bar.narrowest() as u16;
        let without = bar.narrowest_with(false) as u16;
        assert!(without < with_icons, "the glyphs cost nothing?");

        // A column below the designed width: drawn, and drawn without glyphs.
        let tight = with_icons - 1;
        assert!(tight >= without, "{tight} is below the floor, {without}");
        let text = bar.render(tight, theme).expect("it fits without glyphs");
        let joined: String = text
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !joined
                .chars()
                .any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)),
            "a glyph survived at {tight} columns: {joined}"
        );
        // Every label is still there, because the glyphs are decoration and the
        // labels are the settings.
        for button in bar.buttons() {
            assert!(
                joined.contains(&button.label),
                "{} is missing: {joined}",
                button.label
            );
        }
        assert_eq!(
            text.len(),
            Bar::ROWS as usize,
            "the box changed height without its glyphs"
        );

        // And at the designed width they are all back.
        let full = bar.render(with_icons, theme).expect("it fits with glyphs");
        let joined: String = full
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        for glyph in [icon::PUNCTUATION, icon::NUMBERS, icon::TIME] {
            assert!(
                joined.contains(glyph),
                "{glyph:?} is missing at {with_icons} columns"
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
                icon: icon::PUNCTUATION,
                field: Field::Punctuation,
                active,
                disabled: false,
            },
            selected,
            crate::config::theme::ThemeName::Gruvbox.resolve(),
            true,
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
                icon: icon::PUNCTUATION,
                field: Field::Punctuation,
                active: true,
                disabled: true,
            },
            false,
            theme,
            true,
        )
        .style;
        assert_eq!(style.fg, Some(theme.muted));
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    /// A bar that cannot be laid out is not drawn half. A truncated cell looks like
    /// a group of buttons with one missing, and a setting that is silently absent is
    /// worse than a bar that is not there.
    #[test]
    fn a_bar_too_narrow_is_not_drawn_at_all() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [ConfigMode::Time, ConfigMode::Quote, ConfigMode::Zen] {
            for lang in [Lang::English, Lang::Russian] {
                let bar = Bar::build(state_in(mode, lang));
                // The floor is the width at which the bar can be drawn at all, glyphs
                // or not — not the width at which it can be drawn as designed.
                for width in 0..bar.narrowest_with(false) as u16 {
                    assert!(
                        bar.render(width, theme).is_none(),
                        "the {mode:?} bar in {lang:?} was drawn truncated at {width} columns"
                    );
                }
                assert_eq!(
                    bar.rows(bar.narrowest_with(false) as u16),
                    Bar::ROWS,
                    "{mode:?} in {lang:?} did not draw at its own floor"
                );
            }
        }
    }

    /// The box is three rows: a rule, the settings, a rule, with nothing between the
    /// text and the border.
    ///
    /// Asserting the exact number is the point: the screen reserves rows from
    /// [`Bar::rows`] and draws from [`Bar::render`], and if those two ever disagree
    /// the bar overlaps the words or leaves a hole.
    #[test]
    fn the_bar_is_a_three_row_box() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for (mode, lang) in [
            (ConfigMode::Time, Lang::English),
            (ConfigMode::Quote, Lang::English),
            (ConfigMode::Quote, Lang::Russian),
            (ConfigMode::Time, Lang::Russian),
            (ConfigMode::Zen, Lang::English),
        ] {
            let bar = Bar::build(state_in(mode, lang));
            let width = (bar.narrowest() as u16).max(1);
            let lines = bar
                .render(width, theme)
                .unwrap_or_else(|| panic!("{mode:?} in {lang:?} does not fit in {width}"));
            assert_eq!(
                lines.len(),
                Bar::ROWS as usize,
                "{mode:?} in {lang:?}: not {} rows",
                Bar::ROWS
            );
            let text: Vec<String> = lines
                .iter()
                .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect();
            let last = text.len() - 1;
            assert!(
                text[0].starts_with('╭') && text[0].ends_with('╮'),
                "the top rule is not a rule: {}",
                text[0]
            );
            assert!(
                text[last].starts_with('╰') && text[last].ends_with('╯'),
                "the bottom rule is not a rule: {}",
                text[last]
            );
            // The settings row is the only one with anything in it, and it is closed
            // at both sides with a divider between each pair of cells.
            assert_eq!(
                text[1].matches(BORDER).count(),
                4,
                "the settings row is not framed: {}",
                text[1]
            );
            assert!(
                !text[1].trim_matches(['│', ' ']).is_empty(),
                "the settings row is empty: {}",
                text[1]
            );
        }
    }

    /// The three rows are a rule, the settings and a rule, and nothing is empty in
    /// between.
    ///
    /// It was five, with a blank above and below the settings. The blank rows were
    /// meant to make the box a panel with air in it; what they made was a hole inside
    /// a one-line box, and a hole is not padding.
    #[test]
    fn there_is_no_empty_row_inside_the_box() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        let bar = Bar::build(state(ConfigMode::Time));
        let lines = bar
            .render(bar.narrowest() as u16, theme)
            .expect("it fits at its own width");
        for (index, line) in lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let is_rule = text.starts_with('╭') || text.starts_with('╰');
            assert_eq!(
                is_rule,
                index == 0 || index == lines.len() - 1,
                "row {index} is neither a rule nor the settings: {text:?}"
            );
        }
    }

    /// The number of rows the screen reserves is the number of rows the bar draws,
    /// asked of the same code — and it is zero when the bar does not fit, so a
    /// narrow terminal does not reserve five empty rows.
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

    /// The width is the honest one: at the floor the bar is drawn, and one column
    /// less it is not drawn at all.
    ///
    /// There is no third form. It used to be one row, then two rows with the lengths
    /// below, then nothing — and a bar that changes shape as the window narrows is a
    /// bar whose controls move, which is worse than no bar.
    #[test]
    fn the_bar_appears_at_its_floor_and_not_one_column_less() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Zen,
            ConfigMode::Custom,
        ] {
            let bar = Bar::build(state(mode));
            let floor = bar.narrowest_with(false) as u16;
            assert!(
                bar.render(floor, theme).is_some(),
                "the {mode:?} bar does not fit in its own floor, {floor}"
            );
            assert!(
                bar.render(floor - 1, theme).is_none(),
                "the {mode:?} bar still fits one column below its floor"
            );
            assert_eq!(bar.rows(floor), Bar::ROWS);
            assert_eq!(bar.rows(floor - 1), 0, "rows were reserved for nothing");
            // And the designed width, with the glyphs, is a real and larger number.
            assert!(
                bar.narrowest() > bar.narrowest_with(false),
                "the {mode:?} bar claims the glyphs are free"
            );
        }
    }

    /// Every button is inside the box at every width that draws it.
    ///
    /// The three cells are sized to their own content, so a cell cannot be one
    /// column narrower than what is in it — but that is arithmetic, and arithmetic
    /// is what this checks.
    #[test]
    fn no_button_is_cut_off_at_the_narrowest_width() {
        let theme = crate::config::theme::ThemeName::Gruvbox.resolve();
        for mode in [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Custom,
        ] {
            for lang in [Lang::English, Lang::Russian] {
                let bar = Bar::build(state_in(mode, lang));
                let width = bar.narrowest() as u16;
                let text: String = bar
                    .render(width, theme)
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|line| line.spans.iter())
                    .map(|s| s.content.as_ref())
                    .collect();
                for button in bar.buttons() {
                    assert!(
                        text.contains(&button.label),
                        "{mode:?} in {lang:?}: {} is missing from the box: {text}",
                        button.label
                    );
                }
            }
        }
    }

    /// The five mode buttons carry the five mode glyphs, in order.
    ///
    /// Asserted over the buttons rather than against the bar's current mode,
    /// because the bar's current mode is one of the five and not the answer to
    /// "what glyph does the words button have" — that is the button's own business.
    ///
    /// The length card carries none: `15 30 60 120` is a row of numbers, and a glyph
    /// in front of each one would be eight glyphs saying nothing.
    #[test]
    fn the_icons_are_one_column_each_and_only_where_they_mean_something() {
        let bar = Bar::build(state(ConfigMode::Time));
        let modes: Vec<&Button> = bar
            .buttons()
            .into_iter()
            .filter(|b| b.field == Field::Mode)
            .collect();
        assert_eq!(modes.len(), 5, "the mode cell does not have five buttons");
        for (button, mode) in modes.iter().zip([
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Zen,
            ConfigMode::Custom,
        ]) {
            assert_eq!(
                button.icon,
                icon::for_mode(mode),
                "the {mode:?} button has the wrong glyph"
            );
        }
        for button in bar.buttons() {
            let cells = button.icon.chars().count();
            assert!(cells <= 1, "{}: a glyph is {cells} cells", button.label);
            match button.field {
                Field::Time | Field::Words | Field::QuoteLength | Field::CustomText => {
                    assert_eq!(button.icon, "", "{} should have no glyph", button.label)
                }
                _ => assert_eq!(cells, 1, "{} has no glyph", button.label),
            }
        }
    }

    /// The eight glyphs are eight distinct characters, because two buttons with the
    /// same icon are one icon.
    #[test]
    fn the_eight_icons_are_eight_different_glyphs() {
        let unique: std::collections::BTreeSet<&str> =
            icon::TABLE.iter().map(|(_, glyph)| *glyph).collect();
        assert_eq!(
            unique.len(),
            icon::TABLE.len(),
            "two icons are the same character"
        );
        for (name, glyph) in icon::TABLE {
            assert_eq!(glyph.chars().count(), 1, "{name} is not one character");
        }
    }

    /// The name and the codepoint are the same fact, so the table is the fact.
    ///
    /// This is the test that should have existed before the first eight were written.
    /// Four of them were wrong for a release — `U+F0F7` labelled `nf-fa-mountain` is
    /// `building_o`, `U+F1E0` labelled `nf-fa-at` is `share_nodes`, `U+F10E`
    /// labelled `nf-fa-quote_left` is `quote_right`, and `U+F584` labelled
    /// `nf-fa-screwdriver_wrench` is not in the font at all. Nobody noticed for a
    /// release because a Nerd Font draws *something* at almost any private-use
    /// codepoint, so a wrong icon looks like a wrong icon and not like a mistake.
    ///
    /// A name written beside its codepoint is at least reviewable: the pair is
    /// checkable against the upstream cmap tables, which is where the numbers below
    /// came from, and a reviewer who does not know the answer can still see that
    /// `building_o` is not a mountain.
    #[test]
    fn the_icon_table_names_the_glyphs_it_uses() {
        // The exact codepoints, read out of the upstream `FontAwesome.otf` and
        // `MaterialDesignIconsDesktop.ttf` cmap tables. If this test fails, either the
        // icon is wrong or the set has moved it — and both need a human, because a
        // wrong one is indistinguishable from a right one on screen.
        let expected: [(&str, u32); 8] = [
            ("md-dog", 0xf0a43),
            ("fa-hashtag", 0xf292),
            ("md-clock-time-two", 0xf1440),
            ("fa-font", 0xf031),
            ("fa-quote_left", 0xf10d),
            ("fa-mountain", 0xef08),
            ("fa-wrench", 0xf0ad),
            ("fa-screwdriver_wrench", 0xef70),
        ];
        for ((name, glyph), (want_name, want_cp)) in icon::TABLE.iter().zip(expected) {
            assert_eq!(
                *name, want_name,
                "the table and this test disagree on a name"
            );
            let got = glyph.chars().next().expect("one character") as u32;
            assert_eq!(
                got, want_cp,
                "{name} is U+{got:05X}, not U+{want_cp:05X} — \
                 either the codepoint is wrong or the icon set has moved it"
            );
        }
    }

    /// And the eight named constants are the same eight, so a caller using
    /// `icon::ZEN` cannot get a different glyph from one reached through the table.
    #[test]
    fn the_named_icons_are_the_table() {
        for (name, glyph) in [
            ("PUNCTUATION", icon::PUNCTUATION),
            ("NUMBERS", icon::NUMBERS),
            ("TIME", icon::TIME),
            ("WORDS", icon::WORDS),
            ("QUOTE", icon::QUOTE),
            ("ZEN", icon::ZEN),
            ("CUSTOM", icon::CUSTOM),
            ("OTHER", icon::OTHER),
        ] {
            assert!(
                icon::TABLE.iter().any(|(_, t)| *t == glyph),
                "icon::{name} is not in the table"
            );
        }
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

    /// The mode buttons must not move when a mode changes what the side cells hold.
    ///
    /// This is the property the old layout was built around — the site's
    /// `1fr auto 1fr`, where the two `1fr` columns absorb whatever the side cards do
    /// not use — and the box keeps it for a different reason: the centre cell is
    /// sized to its own content first, and only the leftover slack is shared, so
    /// changing the side cells cannot push the modes anywhere.
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
            // The middle row of the box, which is where the settings are.
            let line = &lines[Bar::ROWS as usize / 2];
            // Each button is its own span, so the mode cell starts at the first
            // button whose label is the timed one. The glyph comes from the constant
            // rather than being written here, because a test that spells out the
            // glyph is a second place for it to be wrong.
            let timed = format!("{} time", icon::TIME);
            let mut at = 0usize;
            for span in &line.spans {
                if span.content.as_ref() == timed {
                    return at;
                }
                at += span.content.chars().count();
            }
            panic!("the mode cell is not in the row");
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
        let lines = bar
            .render(bar.narrowest() as u16, theme)
            .expect("it fits at its own width");
        let text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
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
