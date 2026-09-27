//! monkeytuipe — a terminal typing-test client for monkeytype.com.
//!
//! The crate is split so the TUI can be driven and tested headlessly:
//!
//! - [`app::App`] owns all state and the event loop.
//! - [`screens`] render and translate input into [`screens::Effect`]s.
//! - [`config`] is the on-disk `config.toml` plus themes and keybinds.
//! - [`engine`] is the test itself: words, input, and per-word state.
//! - [`words`] generates a test's word list and fetches languages on demand.
//! - [`stats`] is the scoring arithmetic, ported from the website.
//! - [`widgets`] draws the chart, which is the one piece worth reusing across
//!   the typing and results screens.
//! - [`i18n`] is the interface language. The website has none; this is our own.
//! - [`api`] is the typed monkeytype HTTP client.
//! - [`terminal`] owns raw mode and the alternate screen, and restores both on panic.

pub mod action;
pub mod api;
pub mod app;
pub mod config;
pub mod engine;
pub mod i18n;
pub mod screens;
pub mod stats;
pub mod terminal;
pub mod widgets;
pub mod words;
