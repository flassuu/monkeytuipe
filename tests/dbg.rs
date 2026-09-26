use monkeytuipe::app::App;
use monkeytuipe::config::Config;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::Duration;

fn show(app: &App, w: u16, h: u16) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.render(f)).unwrap();
    let b = t.backend().buffer().clone();
    println!("--- {w}x{h}");
    for y in 0..h {
        println!(
            "{:2}|{}|",
            y,
            (0..w).map(|x| b[(x, y)].symbol()).collect::<String>()
        );
    }
}

#[test]
fn show_typing() {
    let mut config = Config::default();
    config.test.time = 10;
    let mut app = App::new(config, std::path::PathBuf::from("/tmp/opencode/x.toml"));
    app.set_words("the child become possible point face back the here however not still any because down by after over just this nation and then more words to type here we go".split(' ').map(|s| s.to_owned()).collect());
    for (ms, text) in [
        (0u64, "the child become "),
        (1400, "possible point "),
        (3000, "face back the "),
        (4600, "here howev"),
        (6100, "er not stil"),
    ] {
        app.set_elapsed(Duration::from_millis(ms));
        for c in text.chars() {
            app.type_char(c);
        }
    }
    app.set_elapsed(Duration::from_millis(8200));
    show(&app, 88, 26);
    show(&app, 60, 20);
    show(&app, 40, 16);
}
