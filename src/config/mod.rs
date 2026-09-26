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
            version: Config::VERSION,
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
    /// Schema version of this file. Bumped whenever a migration is added.
    ///
    /// A file with no `version` key reads as `0`, which is the signal to run
    /// the migration chain. Without it, a config written by any older build
    /// shadows the current defaults forever — the reason a shipped fix can look
    /// like it did nothing on the machine that already ran the app.
    ///
    /// The field default is `0`, not [`Config::VERSION`]: the container-level
    /// `#[serde(default)]` would otherwise fill a missing key from
    /// `Config::default()` and make "written by an old build" indistinguishable
    /// from "current".
    #[serde(default = "unversioned")]
    pub version: u32,
    /// monkeytype ApeKey. Empty means anonymous (no submission).
    pub ape_key: String,
    pub api_url: String,
    pub theme: theme::ThemeName,
    pub keybinds: keybinds::Keybinds,
    pub test: TestConfig,
    /// Send finished tests to monkeytype. Requires `ape_key`.
    pub submit_results: bool,
}

/// The version a config file reads as when it carries no `version` key.
const fn unversioned() -> u32 {
    0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TestConfig {
    pub time: u32,
    pub words: u32,
    pub mode: Mode,
    pub language: String,
    pub punctuation: bool,
    pub numbers: bool,
    pub difficulty: Difficulty,
    pub blind: bool,
    /// Bias word choice towards the frequent end of a frequency-ordered list.
    ///
    /// Off by default to match the website. On a 200-word list it makes the
    /// test much more like real prose and much less varied, so it is a taste
    /// call rather than an improvement.
    pub zipf: bool,
    pub quotes: String,
}

/// Which test to run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Time,
    Words,
    Quote,
}

/// How forgiving the test is about a wrong first key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    /// A leading space skips the word, and Shift is never penalised.
    #[default]
    Normal,
    /// A leading space does not skip the word, and Shift is part of correctness.
    Expert,
    /// As expert, and a word cannot be retyped from the start after a mistake.
    Master,
}

impl Mode {
    /// The name as it appears in the config file and in the UI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Words => "words",
            Self::Quote => "quote",
        }
    }
}

impl Difficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Expert => "expert",
            Self::Master => "master",
        }
    }

    /// Whether a leading space is allowed to skip an untouched word.
    pub fn allows_leading_skip(self) -> bool {
        self == Self::Normal
    }

    /// Whether Shift state is judged, so `Shift+a` is not a way to type `A`.
    pub fn checks_shift(self) -> bool {
        self != Self::Normal
    }
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            time: 30,
            words: 25,
            mode: Mode::default(),
            language: "english".to_owned(),
            punctuation: true,
            numbers: false,
            difficulty: Difficulty::default(),
            blind: false,
            zipf: false,
            quotes: "none".to_owned(),
        }
    }
}

impl Config {
    /// The environment variable that overrides [`Config::ape_key`].
    pub const APE_KEY_ENV: &'static str = "MONKEYTUIPE_APEKEY";

    /// Current config schema version.
    pub const VERSION: u32 = 1;

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

    /// Brings a config written by an older build up to [`Config::VERSION`].
    ///
    /// Migrations rewrite user data, so the caller is expected to keep a backup:
    /// [`Config::load_migrating`] does that automatically.
    pub fn migrate(&mut self) {
        if self.version < 1 {
            // The bare-letter defaults made those letters untypeable. A file
            // that still carries them is far more likely to be a default the app
            // wrote than a choice, so it gets the working bindings.
            self.keybinds.migrate();
        }
        self.version = Self::VERSION;
    }

    /// Loads the config, migrating an out-of-date file in place.
    ///
    /// A stale file is copied to `<config>.bak` before being rewritten, so a
    /// migration that turns out to be wrong is still one `cp` away from being
    /// undone. A file that is already current, or absent, is left untouched.
    pub fn load_migrating(path: &Path) -> anyhow::Result<Self> {
        let mut config = Self::load(path)?;
        if !path.exists() || config.version >= Self::VERSION {
            return Ok(config);
        }
        let backup = path.with_extension("toml.bak");
        std::fs::copy(path, &backup)
            .with_context(|| format!("backing up config to {}", backup.display()))?;
        config.migrate();
        config
            .save(path)
            .with_context(|| format!("rewriting migrated config {}", path.display()))?;
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

    /// A file that omits `version` is by definition written by a build that
    /// predates versioning, so it reads as stale and gets migrated. Everything
    /// it did not mention still comes from the defaults.
    #[test]
    fn an_unversioned_file_migrates_to_the_current_defaults() {
        let config: Config = toml::from_str("").expect("empty toml is valid");
        assert_eq!(config.version, 0, "no version key means no version");
        let mut config = config;
        config.migrate();
        assert_eq!(config, Config::default());
    }

    /// Partial files keep the defaults for what they omit, at the current version.
    #[test]
    fn a_partial_file_keeps_defaults_and_its_version() {
        let config: Config = toml::from_str("version = 1\ntheme = \"nord\"\n").expect("parse");
        assert_eq!(config.version, 1);
        assert_eq!(config.theme, theme::ThemeName::Nord);
        assert_eq!(config.keybinds, keybinds::Keybinds::default());
        assert_eq!(config.test, TestConfig::default());
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

    /// A config file written by a build that predates the "a bare letter is
    /// untypeable" rule must not shadow the working defaults forever.
    #[test]
    fn a_stale_config_is_migrated_and_backed_up() {
        let dir = std::env::temp_dir().join(format!("monkeytuipe-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("config.toml");

        // Exactly the file an old build would have written: no `version` key,
        // and the old bindings.
        std::fs::write(
            &path,
            r#"
ape_key = "sekrit"
api_url = "https://api.monkeytype.com"
theme = "monkeytype"
submit_results = true

[keybinds]
quit = ["q", "ctrl+c"]
up = ["up", "k"]
down = ["down", "j"]
left = ["left", "h"]
right = ["right", "l"]
select = ["space"]
back = ["esc", "enter"]
settings = [","]
start_test = ["t"]
restart = ["r"]

[test]
time = 30
language = "russian"
punctuation = true
numbers = false
quotes = "none"
"#,
        )
        .expect("write stale config");

        let migrated = Config::load_migrating(&path).expect("migrate");

        assert_eq!(migrated.version, Config::VERSION);
        assert_eq!(migrated.keybinds, keybinds::Keybinds::default());
        // ... and the user's own settings came through untouched
        assert_eq!(migrated.ape_key, "sekrit");
        assert_eq!(migrated.test.language, "russian");

        // the rewrite landed, and so did the backup of what was there before
        assert_eq!(Config::load(&path).expect("reload"), migrated);
        let backup = std::fs::read_to_string(dir.join("config.toml.bak")).expect("backup exists");
        assert!(
            backup.contains(r#"restart = ["r"]"#),
            "backup holds the old file"
        );
        assert!(
            !backup.contains("version"),
            "backup is the pre-migration text"
        );

        // a second run is a no-op: no new backup, no further change
        let before = std::fs::metadata(dir.join("config.toml.bak")).expect("backup");
        let again = Config::load_migrating(&path).expect("second load");
        assert_eq!(again, migrated);
        let after = std::fs::metadata(dir.join("config.toml.bak")).expect("backup");
        assert_eq!(
            before.len(),
            after.len(),
            "an up-to-date config must not be rewritten"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_current_config_is_not_rewritten() {
        let dir = std::env::temp_dir().join(format!("monkeytuipe-current-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("config.toml");
        Config::default().save(&path).expect("save");

        let loaded = Config::load_migrating(&path).expect("load");
        assert_eq!(loaded, Config::default());
        assert!(
            !path.with_extension("toml.bak").exists(),
            "nothing to back up, so no backup is made"
        );

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
