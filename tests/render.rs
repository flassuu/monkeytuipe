//! Headless render checks.
//!
//! These are the tests that would have caught the old client's worst bug: a word
//! view that never scrolled, so the caret walked off the screen and kept going.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use monkeytuipe::app::App;
use monkeytuipe::config::theme::Theme;
use monkeytuipe::config::Config;
use monkeytuipe::screens::typing::{bar_rows, rows_for};
use monkeytuipe::screens::ScreenKind;

/// Renders the app at `width`x`height` and returns the buffer.
fn render(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    terminal.backend().buffer().clone()
}

/// Flattens a ratatui buffer into trimmed lines.
fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// The text of the word area only.
///
/// Taken from the screen's own layout rather than from row arithmetic, so a
/// layout change moves this with it instead of quietly testing the wrong band.
fn word_area(buffer: &Buffer, app: &App) -> String {
    area_text(buffer, layout_of(app, buffer).words)
}

/// The text of one rectangle of the screen.
fn area_text(buffer: &Buffer, area: Rect) -> String {
    let all = lines(buffer);
    (area.y..area.y + area.height)
        .map(|y| all.get(y as usize).cloned().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The screen's own layout, so a test and the screen agree on where things are.
fn layout_of(app: &App, buffer: &Buffer) -> monkeytuipe::screens::typing::TypingRows {
    rows_for(buffer.area, bar_rows(app, buffer.area.width))
}

fn app() -> App {
    let dir = std::env::temp_dir().join(format!("monkeytuipe-render-{}", std::process::id()));
    App::new(Config::default(), dir.join("config.toml"))
}

/// 62 distinct single-character words, so a word can never be split by wrapping.
fn single_char_words() -> Vec<String> {
    ('a'..='z')
        .chain('A'..='Z')
        .chain('0'..='9')
        .map(|c| c.to_string())
        .collect()
}

#[test]
fn the_typing_screen_draws_its_header_and_counters() {
    let buffer = render(&app(), 100, 20);
    let app = app();
    let buffer = render(&app, 100, 20);
    let rows = layout_of(&app, &buffer);
    let header = area_text(&buffer, rows.header);
    assert!(header.contains("monkeytuipe"), "no title in {header:?}");

    // The settings bar: the fields the website puts above the words, and the
    // one the user has selected.
    let bar = area_text(&buffer, rows.bar);
    for field in [
        "time",
        "punctuation",
        "numbers",
        "difficulty",
        "language",
        "blind",
    ] {
        assert!(
            bar.contains(field),
            "{field:?} missing from the bar: {bar:?}"
        );
    }

    let counters = area_text(&buffer, rows.counters);
    for expected in ["wpm", "acc", "time", "ctrl+c quit"] {
        assert!(
            counters.contains(expected),
            "{expected:?} missing from {counters:?}"
        );
    }
}

/// The counters go on one line, the way the site shows them.
#[test]
fn the_counters_share_a_single_row() {
    let app = app();
    let buffer = render(&app, 100, 20);
    let counters = layout_of(&app, &buffer).counters;
    assert_eq!(counters.height, 1, "the counters get one row, not a block");

    let row: String = (counters.x..counters.x + counters.width)
        .map(|x| buffer[(x, counters.y)].symbol())
        .collect();
    for label in ["wpm", "acc", "time"] {
        assert!(
            row.contains(label),
            "{label} is not on the counter row: {row:?}"
        );
    }
}

/// The words are centred, not left against the frame.
#[test]
fn the_words_are_centred() {
    let mut app = app();
    app.set_words(vec!["word".to_owned()]);
    let buffer = render(&app, 40, 20);
    let words = layout_of(&app, &buffer).words;

    // Read the row at full width: `lines` trims the trailing padding, and the
    // right margin is exactly the padding being trimmed.
    let row: String = (words.x..words.x + words.width)
        .map(|x| buffer[(x, words.y)].symbol())
        .collect();
    let left = row.len() - row.trim_start().len();
    let right = row.len() - row.trim_end().len();
    assert_eq!(left, right, "the word is not centred: {row:?}");
    assert!(left > 0, "the word is hard against the frame: {row:?}");
}

/// The text painted in `color` inside the words pane, left to right then top to
/// bottom.
///
/// The pane is the area inside the border, above the status line, so the border
/// and the counters — drawn in the same colours — are excluded. Blank cells are
/// dropped because the pane is pre-filled with the base style.
fn text_in_color(buffer: &Buffer, app: &App, color: ratatui::style::Color) -> String {
    area_text_in_color(buffer, layout_of(app, buffer).words, color)
}

/// The same, restricted to one rectangle.
fn area_text_in_color(buffer: &Buffer, area: Rect, color: ratatui::style::Color) -> String {
    (area.y..area.y + area.height)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let cell = &buffer[(*x, *y)];
            cell.fg == color && cell.symbol() != " "
        })
        .map(|(x, y)| buffer[(x, y)].symbol().to_owned())
        .collect()
}

/// The caret cells in the words pane, as (x, y, symbol).
///
/// The caret is the only thing drawn with the foreground colour as its
/// background, so it can be found by colour rather than by column arithmetic.
fn caret_cells(buffer: &Buffer, app: &App, theme: &Theme) -> Vec<(u16, u16, String)> {
    let area = layout_of(&app, buffer).words;
    (area.y..area.y + area.height)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| (x, y)))
        .filter(|(x, y)| buffer[(*x, *y)].bg == theme.foreground)
        .map(|(x, y)| (x, y, buffer[(x, y)].symbol().to_owned()))
        .collect()
}

#[test]
fn typed_letters_are_painted_in_the_correct_colour() {
    let mut app = app();
    app.set_words(vec!["word".to_owned()]);
    let theme = app.theme();

    for c in "wo".chars() {
        app.type_char(c);
    }
    let buffer = render(&app, 40, 20);

    assert_eq!(
        text_in_color(&buffer, &app, theme.correct),
        "wo",
        "the two matching letters"
    );
    assert_eq!(
        text_in_color(&buffer, &app, theme.incorrect),
        "",
        "nothing was mistyped, so nothing should be red"
    );

    // The caret has moved onto the third letter.
    let caret = caret_cells(&buffer, &app, &theme);
    assert_eq!(caret.len(), 1, "one caret: {caret:?}");
    assert_eq!(caret[0].2, "r", "the caret sits on the next letter");
}

#[test]
fn a_mistyped_letter_turns_red_but_the_caret_still_moves_on() {
    let mut app = app();
    app.set_words(vec!["word".to_owned()]);
    let theme = app.theme();

    for c in "woid".chars() {
        app.type_char(c);
    }
    let buffer = render(&app, 40, 20);

    // The mistake is shown against the character that should have been typed, so
    // the third cell is red and still reads 'r'.
    assert_eq!(
        text_in_color(&buffer, &app, theme.incorrect),
        "r",
        "only the third letter is wrong"
    );

    // The caret ran past the word, so it is parked on a blank rather than lost.
    let caret: Vec<String> = caret_cells(&buffer, &app, &theme)
        .into_iter()
        .map(|c| c.2)
        .collect();
    assert_eq!(caret, vec![" ".to_owned()]);
}

#[test]
fn a_committed_word_keeps_its_own_colour_and_does_not_bleed_into_the_next() {
    let mut app = app();
    app.set_words(vec!["good".to_owned(), "bad".to_owned()]);
    let theme = app.theme();

    for c in "good ".chars() {
        app.type_char(c);
    }
    app.type_char('x');
    let buffer = render(&app, 40, 20);

    assert_eq!(
        text_in_color(&buffer, &app, theme.correct),
        "good",
        "the finished word keeps its colour"
    );

    // The mistake in the word being typed is shown against the target character,
    // so the typist can see what they should have pressed.
    assert_eq!(
        text_in_color(&buffer, &app, theme.incorrect),
        "b",
        "not the x that was typed"
    );
}

#[test]
fn the_word_view_scrolls_so_the_caret_never_leaves_the_screen() {
    let mut app = app();
    let words = single_char_words();

    // 12 columns fits 6 words per row, so the view has to scroll well before
    // the end of the list.
    for cursor in 0..words.len() {
        app.set_words(words.clone());
        app.set_cursor_word(cursor);
        let buffer = render(&app, 12, 20);
        let area = word_area(&buffer, &app);
        let active = &words[cursor];
        assert!(
            area.contains(active.as_str()),
            "at word {cursor} ({active}) the active word is off screen:\n{area}"
        );
    }
}

#[test]
fn scrolling_actually_hides_earlier_words() {
    let mut app = app();
    let words = single_char_words();
    app.set_words(words.clone());
    app.set_cursor_word(words.len() - 1);
    let area = word_area(&render(&app, 12, 20), &app);
    assert!(
        !area.contains(&words[0]),
        "the first word should have scrolled off:\n{area}"
    );
    assert!(area.contains(words.last().expect("non-empty").as_str()));
}

#[test]
fn the_caret_is_a_single_cell_on_the_active_character() {
    // The caret is the only cell painted with the foreground colour as its
    // background, so it can be located by colour rather than by column math.
    let theme = app().theme();
    assert_eq!(theme.caret().bg, Some(theme.foreground));

    let words = ["aa", "bbbb", "c"];
    for cursor in 0..words.len() {
        let mut app = app();
        app.set_words(words.iter().map(|w| (*w).to_owned()).collect());
        app.set_cursor_word(cursor);
        let buffer = render(&app, 40, 20);

        let painted: Vec<(u16, u16, String)> = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .filter(|(x, y)| buffer[(*x, *y)].bg == theme.foreground)
            .map(|(x, y)| (x, y, buffer[(x, y)].symbol().to_owned()))
            .collect();

        assert_eq!(
            painted.len(),
            1,
            "expected exactly one caret cell at word {cursor}, got {painted:?}"
        );
        assert_eq!(
            painted[0].2,
            words[cursor].chars().next().unwrap().to_string()
        );
    }
}

#[test]
fn a_narrow_terminal_does_not_panic() {
    let mut app = app();
    app.set_words(single_char_words());
    for (width, height) in [(1u16, 1u16), (4, 3), (10, 4), (20, 6), (200, 60)] {
        for cursor in [0usize, 1, 30, 61] {
            app.set_cursor_word(cursor);
            let _ = render(&app, width, height);
        }
    }
}

#[test]
fn a_one_cell_terminal_still_renders() {
    let buffer = render(&app(), 1, 1);
    assert_eq!(lines(&buffer).len(), 1);
}

/// The chart is live: it appears once a test has run a whole second.
#[test]
fn the_chart_appears_once_the_test_has_run() {
    let mut app = app();
    app.set_words(vec!["word".to_owned()]);
    let rows = layout_of(&app, &render(&app, 60, 20)).chart;
    assert!(
        area_text(&render(&app, 60, 20), rows).trim().is_empty(),
        "an unstarted test has nothing to plot"
    );

    for c in "word ".chars() {
        app.type_char(c);
    }
    app.set_elapsed(std::time::Duration::from_millis(3200));
    let buffer = render(&app, 60, 20);
    let chart = area_text(&buffer, rows);
    assert!(
        chart.contains('█') || chart.contains('─'),
        "the chart is empty after three seconds: {chart:?}"
    );
}

#[test]
fn the_settings_screen_lists_every_row() {
    let mut app = app();
    app.show_screen(ScreenKind::Settings);
    let screen = lines(&render(&app, 60, 14));
    assert!(screen[0].contains("settings"), "no title in {screen:?}");
    for label in [
        "theme",
        "language",
        "punctuation",
        "numbers",
        "ape key",
        "submit results",
    ] {
        assert!(
            screen.iter().any(|l| l.contains(label)),
            "row {label:?} is missing from {screen:?}"
        );
    }
}

#[test]
fn the_settings_screen_marks_the_selected_row() {
    let mut app = app();
    app.show_screen(ScreenKind::Settings);
    let accent = app.theme().accent;
    let buffer = render(&app, 60, 14);
    let selected = (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let cell = &buffer[(*x, *y)];
            cell.fg == accent && cell.modifier.contains(ratatui::style::Modifier::BOLD)
        })
        .count();
    assert!(
        selected > 0,
        "nothing on the settings screen is highlighted"
    );
}

/// A finished test, ready to be looked at.
fn finished_app(seconds: u64) -> App {
    let mut config = Config::default();
    config.test.time = 10;
    let mut app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
    app.set_words(
        "the child become possible point face back the here however not still any"
            .split(' ')
            .map(str::to_owned)
            .collect(),
    );
    for (ms, text) in [
        (0u64, "the child become "),
        (1400, "possible point "),
        (3000, "face back the "),
        (4600, "here however "),
        (6000, "not still any "),
    ] {
        app.set_elapsed(Duration::from_millis(ms));
        for c in text.chars() {
            app.type_char(c);
        }
    }
    app.set_elapsed(Duration::from_secs(seconds));
    app.tick();
    app
}

#[test]
fn a_finished_test_shows_its_result_by_itself() {
    let app = finished_app(12);
    assert_eq!(app.screen_kind(), ScreenKind::Results);
}

#[test]
fn the_results_screen_shows_the_headline_figures() {
    let buffer = render(&finished_app(12), 74, 26);
    let screen = lines(&buffer).join("\n");
    for label in ["wpm", "raw", "chars", "acc", "cons", "time"] {
        assert!(screen.contains(label), "{label} is missing from the result");
    }
    // The figures are numbers, not placeholders.
    assert!(
        screen.contains("acc 100.0%") || screen.contains("acc 99."),
        "no accuracy in {screen:?}"
    );
}

#[test]
fn the_results_screen_draws_the_chart() {
    let buffer = render(&finished_app(12), 74, 26);
    let screen = lines(&buffer).join("\n");
    assert!(
        screen.contains('█') || screen.contains('─'),
        "no chart in {screen:?}"
    );
}

#[test]
fn the_results_screen_lists_the_keys_that_were_used() {
    let buffer = render(&finished_app(12), 74, 26);
    let screen = lines(&buffer).join("\n");
    for key in ['e', 't', 'h'] {
        assert!(
            screen.contains(key),
            "the letter table is missing {key:?}: {screen:?}"
        );
    }
    assert!(screen.contains('\u{2423}'), "the space is shown as a glyph");
}

#[test]
fn the_results_screen_offers_a_way_out() {
    let screen = lines(&render(&finished_app(12), 74, 26)).join("\n");
    assert!(screen.contains("ctrl+r again"), "{screen:?}");
    assert!(screen.contains("ctrl+c quit"), "{screen:?}");
}

/// A result you are looking at must not change under you.
#[test]
fn the_result_does_not_change_while_it_is_on_screen() {
    let mut app = finished_app(12);
    let before = lines(&render(&app, 74, 26));
    let scored = app.result().expect("a result");

    std::thread::sleep(Duration::from_millis(30));
    for _ in 0..5 {
        app.tick();
    }
    // Input on the results screen is dropped, not applied to a frozen test.
    app.press(KeyCode::Char('z'));

    let after = lines(&render(&app, 74, 26));
    assert_eq!(before, after, "the screen moved on its own");
    assert_eq!(app.result().expect("a result"), scored, "the score changed");
}

#[test]
fn the_results_screen_survives_a_tiny_terminal() {
    for (width, height) in [(1u16, 1u16), (4, 3), (10, 4), (20, 6), (200, 60)] {
        let _ = render(&finished_app(12), width, height);
    }
}
