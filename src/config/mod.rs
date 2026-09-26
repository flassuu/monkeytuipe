//! On-disk configuration.

pub mod keybinds;
pub mod theme;

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Defaults applied to any field the config file leaves out.
impl Default for Config {
    fn default() -> Self {
        Self {
            ape_key: String::new(),
            api_url: "https://api.monkeytype.com".to_owned(),
            theme: theme::ThemeName::default(),
            keybinds: keybinds::Keybinds::default(),
            test: TestConfig::default(),
            submit_results: true,
        }
    }
}

/// The user's `config.toml`.
///
/// Only a few fields exist so far; screens and engine settings land in later
/// phases and are added here as they become real.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// monkeytype ApeKey. Empty means anonymous (no submission).
    pub ape_key: String,
    pub api_url: String,
    pub theme: theme::ThemeName,
    pub keybinds: keybinds::Keybinds,
    pub test: TestConfig,
    /// Send finished tests to monkeytype. Requires `ape_key`.
    pub submit_results: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TestConfig {
    pub time: u32,
    pub language: String,
    pub punctuation: bool,
    pub numbers: bool,
    pub quotes: String,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            time: 30,
            language: "english".to_owned(),
            punctuation: true,
            numbers: false,
            quotes: "none".to_owned(),
        }
    }
}

impl Config {
    /// The environment variable that overrides [`Config::ape_key`].
    pub const APE_KEY_ENV: &'static str = "MONKEYTUIPE_APEKEY";

    /// The ApeKey to actually use.
    ///
    /// [`Config::APE_KEY_ENV`] wins over the config file, so a key can live in
    /// the shell or a password manager and never touch disk. The file remains
    /// a fallback for people who would rather not set an env var every time.
    pub fn resolved_ape_key(&self) -> Option<String> {
        std::env::var(Self::APE_KEY_ENV)
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
            .or_else(|| {
                let key = self.ape_key.trim();
                (!key.is_empty()).then(|| key.to_owned())
            })
    }

    /// The path the config is read from: `$MONKEYTUIPE_CONFIG`, else
    /// `$XDG_CONFIG_HOME/monkeytuipe/config.toml`, else `~/.config/monkeytuipe/config.toml`.
    pub fn default_path() -> PathBuf {
        if let Some(explicit) = std::env::var_os("MONKEYTUIPE_CONFIG") {
            return PathBuf::from(explicit);
        }
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
        base.join("monkeytuipe").join("config.toml")
    }

    /// Loads the config, falling back to defaults when the file is absent.
    ///
    /// A malformed file is an error rather than a silent default, so typos do not
    /// quietly discard a user's settings.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        Ok(config)
    }

    /// Writes the config, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(path, text).with_context(|| format!("writing config {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_yields_defaults() {
        let config: Config = toml::from_str("").expect("empty toml is valid");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = toml::from_str::<Config>("definitely_not_a_field = 1")
            .expect_err("unknown fields must not be ignored");
        assert!(err.to_string().contains("definitely_not_a_field"), "{err}");
    }

    #[test]
    fn missing_file_yields_defaults() {
        let config = Config::load(Path::new("/nonexistent/monkeytuipe/config.toml"))
            .expect("absent file is not an error");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("monkeytuipe-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut config = Config::default();
        config.test.language = "russian".to_owned();
        config.save(&path).expect("save");
        assert_eq!(Config::load(&path).expect("load"), config);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// All the ApeKey resolution rules in one test.
    ///
    /// The environment is process-global and `cargo test` runs tests in
    /// parallel threads, so these cases must not be split across separate
    /// `#[test]` functions or they would clobber each other.
    #[test]
    fn ape_key_resolution() {
        const ENV: &str = Config::APE_KEY_ENV;

        // SAFETY: the only test in this process that touches the variable, and
        // it restores whatever was there on the way out.
        let previous = std::env::var_os(ENV);
        let restore = || match previous.clone() {
            Some(value) => unsafe { std::env::set_var(ENV, value) },
            None => unsafe { std::env::remove_var(ENV) },
        };

        let mut config = Config::default();

        unsafe { std::env::remove_var(ENV) };
        assert_eq!(config.resolved_ape_key(), None, "set nowhere");
        config.ape_key = "   ".to_owned();
        assert_eq!(config.resolved_ape_key(), None, "whitespace is not a key");

        config.ape_key = "from-file".to_owned();
        assert_eq!(
            config.resolved_ape_key().as_deref(),
            Some("from-file"),
            "the file is the fallback"
        );

        unsafe { std::env::set_var(ENV, "  from-env  ") };
        assert_eq!(
            config.resolved_ape_key().as_deref(),
            Some("from-env"),
            "the env var wins, and is trimmed"
        );

        unsafe { std::env::set_var(ENV, "") };
        assert_eq!(
            config.resolved_ape_key().as_deref(),
            Some("from-file"),
            "an empty env var must not shadow the file"
        );

        restore();
    }
}
