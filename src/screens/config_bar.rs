//! The settings bar: the strip of choices along the top of the typing screen.
//!
//! This is the website's settings bar rebuilt out of keys. There, every test
//! setting is a button in one row above the words and clicking one changes the
//! test immediately; here the bar is always on screen, one field is selected, the
//! arrows move between fields and change them, and the test is rebuilt as you go
//! so a choice can be seen before it is typed.
//!
//! ## Why the selected field is drawn rather than pointed at
//!
//! The selected field is shown in the accent colour, not surrounded by brackets
//! or marked with a cursor. On a bar that already has one idea per field — a
//! colour for on, a word for off, `30s` for a length — a cursor competes with the
//! thing it is pointing at, and a bracket turns every field into two fields.
//! Colour is also the only cue that survives a bar too narrow to hold a
//! decoration, which a 40-column terminal does not.
//!
//! ## Why the values wrap onto a second row
//!
//! Seven fields do not fit on one line of an 80-column terminal, and a bar that
//! scrolls or truncates is a bar where half the settings cannot be seen. So the
//! bar wraps: one field per word, laid out by the same wrapping rules as the
//! typing screen, and the selection scrolls so that the selected field is always
//! on screen.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::config::bar::Field;
use crate::config::theme::Theme;

/// How many rows the bar takes. Two, because seven fields do not fit on one.
pub const ROWS: u16 = 2;

/// A field and the value it currently holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub field: Field,
    /// What the website prints: the value alone for the mode and the length, and
    /// `label value` for everything else.
    pub label: String,
    pub selected: bool,
}

impl Cell {
    /// The text of one cell, as it appears on the bar.
    pub fn text(&self) -> String {
        if self.field.is_value_only() || self.field.label().is_empty() {
            self.label.clone()
        } else {
            format!("{} {}", self.field.label(), self.label)
        }
    }

    /// The cell's width in columns.
    pub fn width(&self) -> usize {
        self.text().chars().count()
    }
}

/// The bar's cells, in order, for the app's current settings.
pub fn cells(app: &App) -> Vec<Cell> {
    let selected = app.bar_selection();
    app.bar_fields()
        .into_iter()
        .enumerate()
        .map(|(index, field)| Cell {
            field,
            label: app.bar_value(field),
            selected: index == selected,
        })
        .collect()
}

/// Draws the bar into `area`.
///
/// A terminal with no room for a bar gets no bar: the words and the chart are
/// what the screen is for, and a settings bar squeezed into a single row of a
/// 10-line terminal is not settings.
pub fn render(app: &App, area: Rect, theme: Theme, frame: &mut Frame) {
    let cells = cells(app);
    if cells.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let lines = layout(&cells, area.width as usize, area.height as usize);
    let spans: Vec<Line> = lines
        .into_iter()
        .map(|row| {
            let mut spans: Vec<Span> = Vec::new();
            for cell in row {
                let style = if cell.selected {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    theme.chrome()
                };
                spans.push(Span::styled(format!("{} ", cell.text()), style));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(spans).style(theme.base()), area);
}

/// Lays the cells out into rows that fit, keeping the selected one on screen.
///
/// Returns empty when the bar does not fit, so the caller draws nothing rather
/// than a truncated strip.
pub(crate) fn layout(cells: &[Cell], width: usize, height: usize) -> Vec<Vec<Cell>> {
    if cells.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }
    let width = width.max(1);
    // Every cell is followed by a space, so a cell of `w` columns needs w + 1.
    let fits = |cell: &Cell| cell.width() < width;
    // A field too wide for the bar on its own is dropped rather than wrapped
    // across lines, which would break it into unselectable fragments.
    let usable: Vec<&Cell> = cells.iter().filter(|cell| fits(cell)).collect();
    if usable.is_empty() {
        return Vec::new();
    }

    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut used = 0usize;
    for cell in usable {
        let need = cell.width() + 1;
        if used + need > width && !current.is_empty() {
            rows.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(cell.clone());
        used += need;
    }
    if !current.is_empty() {
        rows.push(current);
    }

    // Scroll so the selected field is on one of the rows that will be drawn.
    let Some(selected) = cells.iter().position(|cell| cell.selected) else {
        return rows;
    };
    let at = rows
        .iter()
        .position(|row| row.iter().any(|cell| cell.selected))
        .unwrap_or(0);
    if at >= height {
        let first = at + 1 - height.min(rows.len());
        rows.drain(..first);
    }
    rows.truncate(height);
    let _ = selected;
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::bar::{Bar, Field};
    use crate::config::Config;

    /// A bar cell, for the layout tests, which do not need an `App`.
    fn cell(field: Field, label: &str, selected: bool) -> Cell {
        Cell {
            field,
            label: label.to_owned(),
            selected,
        }
    }

    fn word_cells() -> Vec<Cell> {
        let fields = Bar::fields(crate::config::Mode::Words);
        fields
            .into_iter()
            .enumerate()
            .map(|(i, field)| {
                let label = match field {
                    Field::Mode => "words",
                    Field::Length => "25",
                    Field::Punctuation => "off",
                    Field::Numbers => "off",
                    Field::Difficulty => "normal",
                    Field::Language => "english 1k",
                    Field::Blind => "off",
                    _ => "?",
                };
                cell(field, label, i == 2)
            })
            .collect()
    }

    #[test]
    fn the_mode_and_the_length_print_their_value_alone() {
        // On the website these are the first two buttons and nothing labels them.
        assert_eq!(cell(Field::Mode, "time", false).text(), "time");
        assert_eq!(cell(Field::Length, "30s", false).text(), "30s");
    }

    #[test]
    fn every_other_field_is_labelled() {
        assert_eq!(
            cell(Field::Punctuation, "on", false).text(),
            "punctuation on"
        );
        assert_eq!(
            cell(Field::Difficulty, "master", false).text(),
            "difficulty master"
        );
    }

    #[test]
    fn a_bar_that_fits_stays_on_one_row() {
        let cells = word_cells();
        let needed: usize = cells.iter().map(|c| c.width() + 1).sum();
        let rows = layout(&cells, needed, 2);
        assert_eq!(rows.len(), 1, "needed {needed} columns: {rows:?}");
    }

    /// Seven fields at their full labels do not fit on an 80-column terminal, so
    /// the bar is two rows by design rather than one row that truncates. Every
    /// field still has to be reachable: a bar that drops the difficulty because
    /// the terminal is narrow is a bar with a different set of settings depending
    /// on the window.
    #[test]
    fn the_full_bar_takes_two_rows_and_loses_nothing() {
        let cells = word_cells();
        let needed: usize = cells.iter().map(|c| c.width() + 1).sum();
        assert!(needed > 80, "the premise changed: it now fits on one row");
        let rows = layout(&cells, 80, ROWS as usize);
        assert!(rows.len() <= 2, "{rows:?}");
        let shown: Vec<Field> = rows.iter().flatten().map(|c| c.field).collect();
        for field in cells.iter().map(|c| c.field) {
            assert!(shown.contains(&field), "{field:?} was dropped: {rows:?}");
        }
    }

    /// Seven fields do not fit on a narrow terminal, and the bar wraps rather
    /// than dropping the settings the user cannot see.
    #[test]
    fn a_narrow_bar_wraps_instead_of_losing_fields() {
        let cells = word_cells();
        let rows = layout(&cells, 30, 4);
        assert!(rows.len() > 1, "everything was forced onto one row");
        let shown: Vec<Field> = rows.iter().flatten().map(|c| c.field).collect();
        for field in cells.iter().map(|c| c.field) {
            assert!(shown.contains(&field), "{field:?} was dropped: {rows:?}");
        }
    }

    #[test]
    fn no_row_is_wider_than_the_bar() {
        for width in 20..=100usize {
            for row in layout(&word_cells(), width, 6) {
                let used: usize = row.iter().map(|c| c.width() + 1).sum();
                assert!(used <= width, "width {width}: a row used {used}: {row:?}");
            }
        }
    }

    /// A field wider than the whole bar has to go: wrapping it across lines would
    /// break it into fragments that cannot be selected.
    #[test]
    fn a_field_too_wide_for_the_bar_is_dropped() {
        let cells = vec![
            cell(Field::Mode, "time", false),
            cell(Field::CustomText, "a very long passage indeed", true),
            cell(Field::Length, "30s", false),
        ];
        let rows = layout(&cells, 20, 2);
        let shown: Vec<Field> = rows.iter().flatten().map(|c| c.field).collect();
        assert!(!shown.contains(&Field::CustomText), "{rows:?}");
        assert!(shown.contains(&Field::Mode) && shown.contains(&Field::Length));
    }

    /// Changing mode can put the selection on a row the bar has no space for.
    #[test]
    fn the_selected_field_is_always_on_a_drawn_row() {
        let cells = word_cells();
        for height in 1..=4usize {
            for width in [20usize, 40, 80] {
                let rows = layout(&cells, width, height);
                assert!(
                    rows.iter().flatten().any(|c| c.selected),
                    "the selection is off screen at {width}x{height}: {rows:?}"
                );
                assert!(rows.len() <= height);
            }
        }
    }

    #[test]
    fn a_selection_on_a_later_row_scrolls_the_bar() {
        let mut cells = word_cells();
        let last = cells.len() - 1;
        cells[last].selected = true;
        for c in &mut cells[..last] {
            c.selected = false;
        }
        let rows = layout(&cells, 30, 1);
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].iter().any(|c| c.selected),
            "the selection scrolled off: {rows:?}"
        );
    }

    #[test]
    fn a_bar_with_nothing_in_it_draws_nothing() {
        assert!(layout(&[], 80, 2).is_empty());
    }

    #[test]
    fn a_bar_narrower_than_any_field_draws_nothing() {
        // Better no bar than a strip of one character.
        let cells = vec![cell(Field::CustomText, "long", true)];
        assert!(layout(&cells, 3, 2).is_empty());
    }

    #[test]
    fn a_zero_sized_bar_draws_nothing() {
        assert!(layout(&word_cells(), 0, 2).is_empty());
        assert!(layout(&word_cells(), 80, 0).is_empty());
    }

    /// The bar has to reflect the config, or it is decoration.
    #[test]
    fn the_bar_reads_the_config() {
        let mut config = Config::default();
        config.test.time = 60;
        config.test.punctuation = false;
        let app = crate::app::App::new(config, std::path::PathBuf::from("/nonexistent/c.toml"));
        let cells = cells(&app);
        let by = |field: Field| cells.iter().find(|c| c.field == field).unwrap().clone();
        assert_eq!(by(Field::Mode).text(), "time");
        assert_eq!(by(Field::Length).text(), "60s");
        assert_eq!(by(Field::Punctuation).text(), "punctuation off");
    }

    #[test]
    fn a_zen_bar_offers_essentially_nothing() {
        let mut config = Config::default();
        config.test.mode = crate::config::Mode::Zen;
        let app = crate::app::App::new(config, std::path::PathBuf::from("/nonexistent/c.toml"));
        let texts: Vec<String> = cells(&app).iter().map(Cell::text).collect();
        assert_eq!(texts, ["zen", "blind off"]);
    }
}
