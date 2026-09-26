//! `monkeytuipe` — command-line entry point.
//!
//! All behaviour lives in the library; this file only parses arguments, hands
//! the terminal over to [`terminal`], and guarantees restoration on the way out.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use monkeytuipe::app::App;
use monkeytuipe::config::Config;
use monkeytuipe::{api, terminal};

#[derive(Debug, Parser)]
#[command(name = "monkeytuipe", version, about, long_about = None)]
struct Cli {
    /// Path to the config file.
    /// Defaults to `$MONKEYTUIPE_CONFIG`, else `$XDG_CONFIG_HOME/monkeytuipe/config.toml`.
    #[arg(long, short = 'c', value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print the resolved config path and exit.
    #[arg(long)]
    show_config_path: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(Config::default_path);

    if cli.show_config_path {
        println!("{}", config_path.display());
        return Ok(());
    }

    let config = Config::load(&config_path)?;

    // Build the client up front so a bad base URL surfaces at boot rather than
    // after a finished test.
    let client = api::ApiClient::new(&config.api_url, &config.ape_key);
    let authenticated = client.is_authenticated();

    // `arm` before `init`: if entering the alternate screen fails, the panic
    // hook that restores the terminal is already in place.
    let mut guard = terminal::TerminalGuard::arm();
    let mut tui = terminal::init().context("entering the terminal")?;

    let result = App::new(config, config_path).run(&mut tui);

    // Restore before propagating, so a failed test run still leaves a usable shell.
    guard.restore().context("restoring the terminal")?;
    result?;

    if !authenticated {
        eprintln!("note: no ape_key is set, results will not be submitted");
    }
    Ok(())
}
