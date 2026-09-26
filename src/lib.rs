//! monkeytuipe — a terminal typing-test client for monkeytype.com.
//!
//! The crate is split so the TUI can be driven and tested headlessly:
//!
//! - [`app::App`] owns all state and the event loop.
//! - [`screens`] render and translate input into [`screens::Effect`]s.
//! - [`config`] is the on-disk `config.toml` plus themes and keybinds.
//! - [`api`] is the typed monkeytype HTTP client.
//! - [`terminal`] owns raw mode and the alternate screen, and restores both on panic.

pub mod action;
pub mod api;
pub mod app;
pub mod config;
pub mod screens;
pub mod stats;
pub mod terminal;
