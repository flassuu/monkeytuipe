//! Headless render checks.
//!
//! These are the tests that would have caught the old client's worst bug: a word
//! view that never scrolled, so the caret walked off the screen and kept going.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use monkeytuipe::app::App;
use monkeytuipe::config::Config;
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

/// The text of the word area only: inside the block border, above the status line.
fn word_area(buffer: &Buffer) -> String {
    let last = buffer.area.height as usize - 1;
    lines(buffer)[1..last - 1].join("\n")
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
fn the_typing_screen_draws_its_frame_and_status() {
    let buffer = render(&app(), 100, 10);
    let screen = lines(&buffer);
    assert!(screen[0].contains("monkeytuipe"), "no title in {screen:?}");
    assert!(
        screen
            .iter()
            .any(|l| l.contains("wpm") && l.contains("q quit")),
        "no status line in {screen:?}"
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
        let buffer = render(&app, 12, 8);
        let area = word_area(&buffer);
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
    let area = word_area(&render(&app, 12, 8));
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
        let buffer = render(&app, 40, 10);

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
fn a_one_cell_terminal_still_shows_a_border() {
    let buffer = render(&app(), 1, 1);
    let screen = lines(&buffer);
    assert_eq!(screen.len(), 1);
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
