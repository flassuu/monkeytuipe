//! monkeytype API client.
//!
//! Scope for now: a typed client with ApeKey auth and the profile endpoint,
//! which is where the `uid` needed for result hashing comes from. Result
//! submission lands in a later phase — see `RESEARCH.md` §3.2.

use serde::Deserialize;

pub const DEFAULT_BASE_URL: &str = "https://api.monkeytype.com";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("http transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("could not decode the server response: {0}")]
    Decode(String),

    #[error("http {code}: {message}")]
    Status { code: u16, message: String },

    /// monkeytype rejects results it considers impossible or unverified.
    #[error("the server rejected the request: {code} — {message}")]
    Rejected { code: u16, message: String },
}

/// The envelope every monkeytype endpoint wraps its payload in.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    status: u16,
    message: Option<String>,
    data: Option<T>,
}

/// The slice of `GET /users` the app actually needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Profile {
    pub uid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub streak: u16,
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    ape_key: String,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, ape_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("monkeytuipe/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("a static client config cannot fail to build");
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            ape_key: ape_key.into(),
        }
    }

    /// True when an ApeKey is configured, i.e. authenticated requests are possible.
    pub fn is_authenticated(&self) -> bool {
        !self.ape_key.is_empty()
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .get(&url)
            .header("apikey", &self.ape_key)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| ApiError::Status {
                code: e.status().map_or(0, |s| s.as_u16()),
                message: e.to_string(),
            })?;

        let envelope: Envelope<T> = response
            .json()
            .await
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        if envelope.status >= 200 {
            return envelope
                .data
                .ok_or_else(|| ApiError::Decode("response had no data field".to_owned()));
        }

        let message = envelope.message.unwrap_or_default();
        Err(classify(envelope.status, message))
    }

    /// `GET /users` — the signed-in user's profile, including the `uid` that
    /// result hashes are computed over.
    pub async fn profile(&self) -> Result<Profile, ApiError> {
        self.get("/users").await
    }
}

/// Maps a monkeytype error status onto [`ApiError`].
///
/// 46x codes are the anticheat's rejections; they are surfaced distinctly so the
/// UI can explain *why* a result was refused instead of showing a generic failure.
pub fn classify(status: u16, message: String) -> ApiError {
    if (460..480).contains(&status) {
        ApiError::Rejected {
            code: status,
            message,
        }
    } else {
        ApiError::Status {
            code: status,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anticheat_codes_are_rejections() {
        for code in [460, 463, 471, 479] {
            assert!(
                matches!(classify(code, "no".to_owned()), ApiError::Rejected { code: c, .. } if c == code),
                "{code} should be a Rejected"
            );
        }
    }

    #[test]
    fn other_codes_are_plain_status_errors() {
        for code in [400, 401, 404, 500] {
            assert!(
                matches!(classify(code, "no".to_owned()), ApiError::Status { code: c, .. } if c == code),
                "{code} should be a Status"
            );
        }
    }

    #[test]
    fn trailing_slashes_are_normalised() {
        let client = ApiClient::new("https://api.monkeytype.com/", "");
        assert_eq!(client.base_url, "https://api.monkeytype.com");
    }

    #[test]
    fn no_ape_key_means_unauthenticated() {
        assert!(!ApiClient::new(DEFAULT_BASE_URL, "").is_authenticated());
        assert!(ApiClient::new(DEFAULT_BASE_URL, "abc123").is_authenticated());
    }

    #[test]
    fn profile_deserialises_from_a_bare_data_object() {
        let json = r#"{"uid":"u_1","name":"flassuu","streak":7,"extra":"ignored"}"#;
        let profile: Profile = serde_json::from_str(json).expect("decodes");
        assert_eq!(profile.uid, "u_1");
        assert_eq!(profile.streak, 7);
    }

    #[test]
    fn envelope_reads_the_data_field() {
        let json = r#"{"status":200,"data":{"uid":"u_2"}}"#;
        let envelope: Envelope<Profile> = serde_json::from_str(json).expect("decodes");
        assert_eq!(envelope.status, 200);
        assert_eq!(envelope.data.expect("data").uid, "u_2");
    }
}
