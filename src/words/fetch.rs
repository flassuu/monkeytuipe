//! Downloading a word list, and caching it on disk.
//!
//! The binary embeds the base list of six languages (see [`super::language`]).
//! Every other list is fetched once from upstream and kept in
//! `~/.cache/monkeytuipe/languages/`, so a test in `kurdish` costs one
//! download for the life of the cache and nothing after that.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::words::language::{self, Language};

/// Where the lists live upstream. No auth, no rate limit worth worrying about.
pub const UPSTREAM_BASE: &str =
    "https://raw.githubusercontent.com/monkeytypegame/monkeytype/master/frontend/static/languages";

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("http transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("{0} is not a language monkeytype publishes")]
    Unknown(String),

    #[error("the downloaded list for {id} could not be read: {reason}")]
    Malformed { id: String, reason: String },

    #[error("could not use the cache at {path}: {reason}")]
    Io { path: PathBuf, reason: String },
}

impl FetchError {
    fn io(path: &Path, err: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            reason: err.to_string(),
        }
    }
}

/// The cache directory: `$MONKEYTUIPE_CACHE`, else
/// `$XDG_CACHE_HOME/monkeytuipe/languages`, else `~/.cache/monkeytuipe/languages`.
pub fn cache_dir() -> PathBuf {
    if let Some(explicit) = std::env::var_os("MONKEYTUIPE_CACHE") {
        return PathBuf::from(explicit).join("languages");
    }
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache"));
    base.join("monkeytuipe").join("languages")
}

/// The cache file for one language.
pub fn cache_path(id: &str) -> PathBuf {
    cache_dir().join(format!("{id}.json"))
}

/// Why a list is unavailable, when [`fetch`] could not produce one.
#[derive(Debug)]
pub enum Unavailable {
    /// No network, and nothing cached. An embedded language is the fallback.
    Offline(Box<FetchError>),
    /// The list does not exist upstream.
    Unknown(String),
    /// The list exists but could not be understood.
    Malformed(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline(err) => write!(f, "{err}"),
            Self::Unknown(id) => write!(f, "{id} is not a language monkeytype publishes"),
            Self::Malformed(reason) => write!(f, "{reason}"),
        }
    }
}

/// Resolves a language id to a word list.
///
/// Three sources, cheapest first: the compiled-in copy, then the cache, then the
/// network. A network failure is only fatal if the list was not available
/// anywhere else, so an offline first run still works in every embedded
/// language.
pub async fn fetch(id: &str) -> Result<Language, Unavailable> {
    if let Some(language) = language::embedded(id) {
        return Ok(language);
    }
    if let Some(language) = read_cache(id) {
        return Ok(language);
    }
    match download(id).await {
        Ok(language) => {
            // A cache write failure must not cost the user their test.
            let _ = write_cache(id, &language);
            Ok(language)
        }
        Err(err) => Err(match err {
            FetchError::Unknown(id) => Unavailable::Unknown(id),
            FetchError::Malformed { reason, .. } => Unavailable::Malformed(reason),
            other => Unavailable::Offline(Box::new(other)),
        }),
    }
}

/// Reads a cached list, treating any failure as a miss.
///
/// A cache is a convenience, not a source of truth: a truncated write from an
/// interrupted download should cost a refetch, not an error the user cannot
/// clear.
pub fn read_cache(id: &str) -> Option<Language> {
    let path = cache_path(id);
    let text = std::fs::read_to_string(&path).ok()?;
    match language::parse(&text) {
        Some(language) => Some(language),
        None => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

fn write_cache(id: &str, language: &Language) -> Result<(), FetchError> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| FetchError::io(&dir, e))?;
    let path = cache_path(id);
    let text = serde_json::to_string(language).map_err(|e| FetchError::Malformed {
        id: id.to_owned(),
        reason: e.to_string(),
    })?;
    std::fs::write(&path, text).map_err(|e| FetchError::io(&path, e))
}

/// Fetches a list from upstream, bypassing the cache.
pub async fn download(id: &str) -> Result<Language, FetchError> {
    // Upstream ids are lowercase letters, digits and underscores. Checking here
    // keeps a config value from turning into a path or a URL of its own.
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(FetchError::Unknown(id.to_owned()));
    }

    let url = format!("{UPSTREAM_BASE}/{id}.json");
    let client = reqwest::Client::builder()
        .user_agent(concat!("monkeytuipe/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(FetchError::Transport)?;

    let response = client.get(&url).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(FetchError::Unknown(id.to_owned()));
    }
    let text = response
        .error_for_status()
        .map_err(FetchError::Transport)?
        .text()
        .await
        .map_err(FetchError::Transport)?;

    language::parse(&text).ok_or_else(|| FetchError::Malformed {
        id: id.to_owned(),
        reason: "not a word list, or it has no words in it".to_owned(),
    })
}

/// Whether a list can be had with no network at all.
pub fn is_available_offline(id: &str) -> bool {
    language::embedded(id).is_some() || read_cache(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_embedded_language_never_touches_the_network() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let language = runtime
            .block_on(fetch("english"))
            .expect("english is embedded");
        assert!(!language.is_empty());
    }

    #[test]
    fn a_path_traversal_is_not_a_language_id() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        for bad in ["../etc/passwd", "a/b", "", "English", "x.json"] {
            let err = runtime
                .block_on(download(bad))
                .expect_err("must be rejected before any request");
            assert!(
                matches!(err, FetchError::Unknown(_)),
                "{bad:?} gave {err:?}"
            );
        }
    }

    #[test]
    fn the_cache_path_is_inside_the_cache_directory() {
        let path = cache_path("english");
        assert!(path.ends_with("languages/english.json"), "{path:?}");
    }
}
