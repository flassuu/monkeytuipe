# monkeytuipe

A terminal typing-test client for [monkeytype.com](https://monkeytype.com), built with
Rust and [ratatui](https://github.com/ratatui/ratatui). Results sync to monkeytype
using an ApeKey.

> **Status: early skeleton.** The app boots, has an event loop, a config file, a
> settings screen, and a typed API client. The typing engine, word lists, result
> submission, and the rest of the UI are not written yet — see the plan below.

## Stack

| Concern  | Choice                                                        |
| -------- | ------------------------------------------------------------- |
| TUI      | `ratatui` + `crossterm`                                       |
| Runtime  | `tokio` (event loop + HTTP)                                   |
| HTTP     | `reqwest` with `rustls`                                      |
| Config   | `toml` + `serde` at `~/.config/monkeytuipe/config.toml`       |
| Hashing  | `sha1`, added in Phase 2 with the `object-hash` port in `api/hash.rs` |

## Build

```sh
cargo build --release
cargo run
```

Requires a Rust toolchain (edition 2021, 1.80+).

## Configuration

The config file is optional — every field has a default. Create it by running once
and letting the app save on exit, or write it by hand:

```toml
# ~/.config/monkeytuipe/config.toml
ape_key = "your-monkeytype-apekey"   # https://monkeytype.com/settings
api_url = "https://api.monkeytype.com"
theme = "monkeytype"                 # monkeytype | gruvbox | nord
submit_results = true

[test]
time = 30
language = "english"
punctuation = true
numbers = false
quotes = "none"

[keybinds]
quit = ["q", "ctrl+c"]
start_test = ["t"]
restart = ["r"]
settings = [","]
```

Find the config path with `monkeytuipe --show-config-path`. Point elsewhere with
`--config <path>` or `MONKEYTUIPE_CONFIG`.

## Layout

```
src/
  lib.rs             crate root
  main.rs            CLI entry, boot, terminal ownership
  app.rs             state, event loop, keybind resolution
  action.rs          key event -> semantic Action
  terminal.rs        raw mode / alt screen with panic-safe restore
  api/               monkeytype HTTP client
  config/            toml config, themes, keybinds
  screens/           one module per screen
tests/
  render.rs          headless render checks against ratatui's TestBackend
```

## Keys

| Key       | Action                    |
| --------- | ------------------------- |
| `t`       | start a test              |
| `r`       | restart                   |
| `,`       | toggle settings           |
| `↑` `↓`   | move in settings          |
| `←` `→`   | change a setting          |
| `space`   | toggle a setting          |
| `esc`     | back                      |
| `q`       | quit                      |

## Tests

```sh
cargo test
```

Unit tests live next to the code; `tests/render.rs` renders the app into
ratatui's `TestBackend` and asserts on the resulting buffer — including that the
word view scrolls so the caret never leaves the screen.

## Plan

- **Phase 0 — foundation.** Event loop, config, themes. *(in progress)*
- **Phase 1 — engine.** Post-hoc character counting, incremental WPM/accuracy,
  per-keystroke `event_log`, 1s chart sampling.
- **Phase 2 — sync.** Port `object-hash` to Rust, `POST /results`, verify against
  the real API.
- **Phase 3 — visuals.** Live chart, per-key heatmap, results screen, command palette.
- **Phase 4 — data.** Personal bests, leaderboards, streak, language downloads.
- **Phase 5 — extras.** Quotes mode, `import`/`export`, Lua-free scripting.

## License

MIT
