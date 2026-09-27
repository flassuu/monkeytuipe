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
    let app = app();
    let buffer = render(&app, 100, 20);
    let rows = layout_of(&app, &buffer);
    let header = area_text(&buffer, rows.header);
    assert!(header.contains("monkeytuipe"), "no title in {header:?}");

    // The top bar: the three cards the website puts above the words, in its
    // order — the toggles, the five modes, then the length for a timed test.
    let bar = area_text(&buffer, rows.bar);
    for field in [
        "punctuation",
        "numbers",
        "time",
        "words",
        "quote",
        "zen",
        "custom",
        "15",
        "30",
        "60",
        "120",
    ] {
        assert!(
            bar.contains(field),
            "{field:?} missing from the bar: {bar:?}"
        );
    }
    // And in that order, because the cards are laid out left to right.
    let at = |needle: &str| bar.find(needle).unwrap_or_else(|| panic!("no {needle}"));
    assert!(at("punctuation") < at("numbers"), "{bar:?}");
    assert!(at("numbers") < at("time"), "{bar:?}");
    assert!(at("zen") < at("custom"), "{bar:?}");
    assert!(at("custom") < at("15"), "{bar:?}");

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
    let area = layout_of(app, buffer).words;
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
    // Only what the settings screen owns. Punctuation, numbers, difficulty and
    // the length live in the bar, and a test that expected them here is testing
    // a layout the screen deliberately does not have.
    for label in [
        "theme",
        "language",
        "custom text",
        "ape key",
        "submit results",
        "back to typing",
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

/// Blind mode shows only the word being typed, so the test is about reading ahead
/// rather than recall. A setting that does nothing on screen is worse than no
/// setting at all, so this checks it actually hides the words.
///
/// Reached through the command list, which is the only way a user gets there:
/// the bar has no blind button, because neither does the site's.
#[test]
fn blind_mode_shows_only_the_active_word() {
    let mut app = app();
    app.set_words(
        "alpha bravo charlie delta echo"
            .split(' ')
            .map(str::to_owned)
            .collect(),
    );
    let buffer = render(&app, 60, 20);
    let whole = area_text(&buffer, layout_of(&app, &buffer).words);
    assert!(
        whole.contains("alpha") && whole.contains("echo"),
        "{whole:?}"
    );

    // Turn it on through the command list, the way the user would: escape, type.
    let index = monkeytuipe::screens::commands::filter("blind")
        .first()
        .map(|m| m.command)
        .expect("a blind command in the list");
    app.run_command(index);
    assert!(app.is_blind());

    let buffer = render(&app, 60, 20);
    let blind = area_text(&buffer, layout_of(&app, &buffer).words);
    assert!(
        blind.contains("alpha"),
        "the active word is gone: {blind:?}"
    );
    for hidden in ["bravo", "charlie", "delta", "echo"] {
        assert!(
            !blind.contains(hidden),
            "{hidden} is still visible in blind mode: {blind:?}"
        );
    }
}

// ---- the top bar --------------------------------------------------------

/// An app pinned to one mode, with its word list under control.
fn in_mode(mode: monkeytuipe::config::Mode) -> App {
    let mut config = Config::default();
    config.test.mode = mode;
    let mut app = App::new(config, PathBuf::from("/nonexistent/config.toml"));
    app.set_words(
        "the child become possible point face back the here however not still any because"
            .split(' ')
            .map(str::to_owned)
            .collect(),
    );
    app
}

/// The bar's own row, which is the row the layout gives it.
fn bar_row(app: &App, buffer: &Buffer) -> String {
    area_text(buffer, layout_of(app, buffer).bar)
}

/// Every mode draws a whole bar, with nothing missing off the right edge.
///
/// Quote mode is the tightest: its right card is as wide as the mode card, so it
/// is the one that cannot be centred in 80 columns and falls back to being packed
/// against the left edge. It still has to be *there*.
#[test]
fn every_mode_draws_a_complete_bar_in_eighty_columns() {
    for mode in [
        monkeytuipe::config::Mode::Time,
        monkeytuipe::config::Mode::Words,
        monkeytuipe::config::Mode::Quote,
        monkeytuipe::config::Mode::Zen,
        monkeytuipe::config::Mode::Custom,
    ] {
        let app = in_mode(mode);
        let buffer = render(&app, 80, 20);
        let bar = bar_row(&app, &buffer);
        for expected in ["time", "words", "quote", "zen", "custom"] {
            assert!(
                bar.contains(expected),
                "{mode:?} bar is missing {expected:?}: {bar:?}"
            );
        }
        match mode {
            monkeytuipe::config::Mode::Zen => {
                assert!(!bar.contains("punctuation"), "zen has a left card: {bar:?}");
                assert!(!bar.contains("thicc"), "zen has a right card: {bar:?}");
            }
            monkeytuipe::config::Mode::Quote => {
                for length in ["all", "short", "medium", "long", "thicc"] {
                    assert!(
                        bar.contains(length),
                        "{mode:?} bar is missing {length:?}: {bar:?}"
                    );
                }
            }
            monkeytuipe::config::Mode::Custom => assert!(bar.contains("add"), "{bar:?}"),
            _ => {
                for toggles in ["punctuation", "numbers"] {
                    assert!(
                        bar.contains(toggles),
                        "{mode:?} bar is missing {toggles:?}: {bar:?}"
                    );
                }
            }
        }
    }
}

/// The cards are separate things, with something between them. Three runs of text
/// with nothing in the gaps read as one sentence, and the grouping is the thing
/// being copied from the site.
#[test]
fn the_cards_are_separated_by_something() {
    let app = in_mode(monkeytuipe::config::Mode::Time);
    let buffer = render(&app, 80, 20);
    let bar = bar_row(&app, &buffer);
    for boundary in ["numbers", "custom"] {
        let at = bar
            .find(boundary)
            .unwrap_or_else(|| panic!("no {boundary}"));
        let after = &bar[at + boundary.len()..];
        assert!(
            after.starts_with("  "),
            "nothing between {boundary:?} and the next card: {bar:?}"
        );
    }
}

/// Below the width the bar needs there is no bar at all rather than half of one.
/// The words and the counters are the test; the bar is a convenience.
#[test]
fn a_narrow_terminal_gets_no_bar_rather_than_a_broken_one() {
    let app = in_mode(monkeytuipe::config::Mode::Time);
    let buffer = render(&app, 40, 20);
    let bar = bar_row(&app, &buffer);
    assert!(bar.trim().is_empty(), "a 40-column bar was drawn: {bar:?}");
    assert!(bar_rows(&app, 40) == 0, "but a row was reserved for it");
}

// ---- the input window ---------------------------------------------------

/// The command window, opened and searched, as a user would.
fn commands_window(query: &str) -> (App, Buffer) {
    let mut app = in_mode(monkeytuipe::config::Mode::Time);
    app.press(KeyCode::Esc);
    for c in query.chars() {
        app.press(KeyCode::Char(c));
    }
    let buffer = render(&app, 80, 24);
    (app, buffer)
}

#[test]
fn the_command_window_draws_the_list_it_found() {
    let (_app, buffer) = commands_window("th");
    let text = lines(&buffer).join("\n");
    assert!(text.contains("commands"), "the window has no title: {text}");
    assert!(
        text.contains("next theme"),
        "the first match is missing: {text}"
    );
    assert!(
        text.contains("type to search"),
        "the hint is missing: {text}"
    );
}

#[test]
fn the_command_window_says_when_nothing_matched() {
    let (_app, buffer) = commands_window("qwertyuiop");
    let text = lines(&buffer).join("\n");
    assert!(text.contains("commands"), "{text}");
    // The field still holds the query, so it can be corrected rather than retyped.
    assert!(text.contains("qwertyuiop"), "{text}");
}

/// The window is drawn over the words, opaquely. A box with the words legible
/// through it is harder to read than one without.
#[test]
fn the_window_covers_what_is_under_it() {
    let mut app = in_mode(monkeytuipe::config::Mode::Time);
    let before = render(&app, 80, 24);
    app.press(KeyCode::Esc);
    let after = render(&app, 80, 24);
    assert_ne!(
        before, after,
        "opening the window changed nothing on screen"
    );
    let mut changed = 0;
    for y in 0..24u16 {
        for x in 0..80u16 {
            if before[(x, y)] != after[(x, y)] {
                changed += 1;
            }
        }
    }
    assert!(
        changed > 40,
        "only {changed} cells changed; the window is a sliver"
    );
}

// ---- zen ----------------------------------------------------------------

/// Zen has no target: it shows what was typed and marks nothing as wrong.
///
/// This is the part of zen that is not a word test at all, and it is the part a
/// user notices first — a zen that shows the word list and colours it is just a
/// normal test with the length removed.
#[test]
fn zen_shows_what_was_typed_and_nothing_that_was_not() {
    let mut app = in_mode(monkeytuipe::config::Mode::Zen);
    app.set_elapsed(Duration::from_millis(400));
    for c in "hello wrld ".chars() {
        app.type_char(c);
    }
    app.set_elapsed(Duration::from_millis(2400));
    let buffer = render(&app, 80, 20);
    let words = word_area(&buffer, &app);
    assert!(words.contains("hello"), "{words:?}");
    assert!(words.contains("wrld"), "{words:?}");
    // The generated target is nowhere in sight.
    assert!(
        !words.contains("become"),
        "zen showed its target words: {words:?}"
    );

    // Only the word pane. The chart draws its falling edge in the error colour,
    // which is correct there, so looking at the whole screen would count it.
    let theme = app.theme();
    let pane = layout_of(&app, &buffer).words;
    let mut reds = String::new();
    for y in pane.y..pane.y + pane.height {
        for x in pane.x..pane.x + pane.width {
            if buffer[(x, y)].fg == theme.incorrect {
                reds.push_str(buffer[(x, y)].symbol());
            }
        }
    }
    assert_eq!(reds, "", "zen marked these as wrong: {reds:?}");
}

// ---- the thing that was reported as a bug ------------------------------

/// A word that was typed correctly is not drawn in the error colour.
///
/// Reported as "finished words light up red, even correct ones". The engine and
/// the renderer both turned out to be right, so this pins the behaviour from the
/// outside: if it ever comes back, this is the test that says so.
#[test]
fn a_correctly_typed_word_is_not_drawn_in_the_error_colour() {
    let mut app = in_mode(monkeytuipe::config::Mode::Time);
    app.set_elapsed(Duration::from_millis(300));
    for c in "the child ".chars() {
        app.type_char(c);
    }
    app.set_elapsed(Duration::from_millis(1900));
    // One wrong word, so the test would notice a red that is not its own.
    for c in "bxcome ".chars() {
        app.type_char(c);
    }
    let buffer = render(&app, 80, 20);
    let theme = app.theme();
    let words = layout_of(&app, &buffer).words;

    let mut red = String::new();
    let mut green = String::new();
    for y in words.y..words.y + words.height {
        for x in words.x..words.x + words.width {
            let cell = &buffer[(x, y)];
            if cell.fg == theme.incorrect {
                red.push_str(cell.symbol());
            } else if cell.fg == theme.correct {
                green.push_str(cell.symbol());
            }
        }
    }
    assert!(
        green.contains("child"),
        "the correct word is not drawn correct: {green:?}"
    );
    assert!(
        green.contains("the"),
        "the first correct word is not drawn correct: {green:?}"
    );
    // Exactly the word that was mistyped, and nothing else. A red anywhere else
    // in the line is the bug this test exists for.
    assert_eq!(
        red, "become",
        "the wrong characters are not exactly the wrong word"
    );
}
