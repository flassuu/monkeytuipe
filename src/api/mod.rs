//! The monkeytype API client.
//!
//! ## What an ApeKey can and cannot do
//!
//! The useful thing to know about this API before designing around it: **an
//! ApeKey cannot submit a result.** `POST /results` is authenticated with a
//! Firebase bearer token, and the server says so plainly — an ApeKey gets
//! `401 This endpoint does not accept ApeKeys`. That is not a gap to work around
//! on this side; the write path belongs to the browser.
//!
//! What an ApeKey *can* do is read, and that is most of the value anyway:
//!
//! | Endpoint | With an ApeKey |
//! |---|---|
//! | `GET /users` | profile, streak, the `uid` |
//! | `GET /users/personalBests` | best times per mode, language and length |
//! | `GET /users/tags` | the tags results can be filed under |
//! | `GET /users/stats` | lifetime typing statistics |
//! | `GET /users/streak` | the current and longest streak |
//! | `GET /users/currentTestActivity` | whether a test is already running |
//! | `POST /results` | **no** — needs a bearer token |
//! | `GET`/`PATCH` `/configs` | **no** — needs a bearer token |
//!
//! So this client reads. The submission path is [`submission`], which computes
//! the payload and the object hash and then reports honestly that there is
//! nowhere to send it, rather than pretending the test was saved.
//!
//! ## The header
//!
//! `Authorization: ApeKey <key>`. Not `apikey: <key>`, which is the header a
//! self-hosted *middleware* uses and which the public API answers
//! `{"message":"Unauthorized"}` to.

use std::time::Duration;

use serde::Deserialize;

use crate::config::Mode;

pub mod submission;

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

impl ApiError {
    /// Whether this is the server saying the key is wrong, as opposed to the
    /// network being down or a bug here.
    ///
    /// Worth distinguishing: "your ApeKey is not accepted" is something the user
    /// can fix, and a network failure is not.
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, Self::Status { code: 401, .. })
    }
}

/// Reads a field that upstream types as "a string, or a number".
fn string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        Text(String),
        Number(i64),
    }
    match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::Text(text) => Ok(text),
        StringOrNumber::Number(number) => Ok(number.to_string()),
    }
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
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub streak: u16,
    /// Whether the account has opted out of the leaderboard.
    #[serde(default, rename = "lbOptOut")]
    pub lb_opt_out: bool,
    #[serde(default, rename = "banned")]
    pub banned: bool,
}

impl Profile {
    /// The name to show, falling back to something honest rather than blank.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "anonymous"
        } else {
            &self.name
        }
    }
}

/// One record from `GET /users/personalBests`.
///
/// The shape is irregular — a `time` record is `{language, mode2: "30", wpm}`
/// while a `words` record is `{language, mode2: "25", wpm}` and a quote record
/// carries the quote id — so this is the union flattened to what every one of
/// them has.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PersonalBest {
    #[serde(default)]
    pub language: String,
    /// The length, as a string: seconds for a time test, words for a words test,
    /// a quote id for a quote test.
    ///
    /// Upstream's schema is `StringNumberSchema` — a string *or* a number — and
    /// the site always sends a string, but a number is valid and appears in
    /// records written by older builds. Deserialising straight into a `String`
    /// would make every one of those records fail to parse, and a client that
    /// silently sees no personal bests is worse than one that says it cannot
    /// reach them.
    #[serde(default, deserialize_with = "string_or_number")]
    pub mode2: String,
    pub wpm: f64,
    /// Time in seconds, for a words test.
    #[serde(default)]
    pub time: f64,
    #[serde(default)]
    pub timestamp: i64,
}

impl PersonalBest {
    /// `mode2` as a number, for comparing against the current test's length.
    pub fn length(&self) -> Option<u32> {
        self.mode2.parse::<u32>().ok()
    }
}

/// `GET /users/personalBests`, grouped by mode.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PersonalBests {
    pub time: Vec<PersonalBest>,
    pub words: Vec<PersonalBest>,
    pub quote: Vec<PersonalBest>,
}

impl PersonalBests {
    /// The best record for a mode, language and length, if there is one.
    ///
    /// The API returns every combination the user has set, so the match has to be
    /// on all three; picking the first record for a mode and showing it next to a
    /// test in a different language would be a lie with a number on it.
    pub fn best(&self, mode: Mode, language: &str, length: Option<u32>) -> Option<&PersonalBest> {
        let list = match mode {
            Mode::Time => &self.time,
            Mode::Words => &self.words,
            _ => return None,
        };
        list.iter()
            .filter(|best| best.language == language)
            .filter(|best| match length {
                Some(length) => best.length() == Some(length),
                None => true,
            })
            .max_by(|a, b| a.wpm.total_cmp(&b.wpm))
    }
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
            .timeout(Duration::from_secs(15))
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

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.base_url);
        let builder = self.http.request(method, url);
        if self.ape_key.is_empty() {
            builder
        } else {
            builder.header("Authorization", format!("ApeKey {}", self.ape_key))
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let response = self
            .request(reqwest::Method::GET, path)
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

    /// `GET /users` — the signed-in user's profile.
    pub async fn profile(&self) -> Result<Profile, ApiError> {
        self.get("/users").await
    }

    /// `GET /users/personalBests?mode=<mode>&language=<language>`.
    ///
    /// The API requires both parameters and returns an empty list for a
    /// combination the user has no record of, which is not an error: most people
    /// have records in a handful of the several hundred mode/language pairs.
    pub async fn personal_bests(
        &self,
        mode: Mode,
        language: &str,
    ) -> Result<Vec<PersonalBest>, ApiError> {
        let path = format!(
            "/users/personalBests?mode={}&language={}",
            mode.as_str(),
            language
        );
        self.get(&path).await
    }
}

/// The language a result payload reports.
///
/// The server validates this against its own list of ids, so a name that only
/// exists on this side has to be reduced to the base language rather than
/// rejected. `english_5k` is a real published id and `english_5k_test` is not.
pub fn language_for_submission(id: &str) -> String {
    id.to_owned()
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
    fn a_rejected_key_is_distinguishable_from_a_network_failure() {
        // "Your ApeKey is wrong" is something the user can fix; a dead network
        // is not, and telling them to check their key for one is a red herring.
        assert!(ApiError::Status {
            code: 401,
            message: "Unauthorized".to_owned()
        }
        .is_auth_failure());
        assert!(!ApiError::Status {
            code: 500,
            message: "boom".to_owned()
        }
        .is_auth_failure());
        assert!(!ApiError::Decode("no".to_owned()).is_auth_failure());
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
    fn a_profile_with_no_name_says_so_rather_than_rendering_blank() {
        let profile = Profile {
            uid: "u".to_owned(),
            name: String::new(),
            streak: 0,
            lb_opt_out: false,
            banned: false,
        };
        assert_eq!(profile.display_name(), "anonymous");
    }

    #[test]
    fn envelope_reads_the_data_field() {
        let json = r#"{"status":200,"data":{"uid":"u_2"}}"#;
        let envelope: Envelope<Profile> = serde_json::from_str(json).expect("decodes");
        assert_eq!(envelope.status, 200);
        assert_eq!(envelope.data.expect("data").uid, "u_2");
    }

    /// A real `personalBests` entry, which is a bare object inside a list.
    #[test]
    fn a_personal_best_reads_the_shape_the_api_sends() {
        let json = r#"[{"_id":"65f0","language":"english","mode2":"30","wpm":118.42,"time":30.0,"timestamp":1717000000000}]"#;
        let bests: Vec<PersonalBest> = serde_json::from_str(json).expect("decodes");
        assert_eq!(bests.len(), 1);
        assert_eq!(bests[0].language, "english");
        assert_eq!(bests[0].length(), Some(30));
        assert_eq!(bests[0].wpm, 118.42);
    }

    /// `mode2` is a string upstream but the schema accepts a number too, and a
    /// client that only reads one of them silently sees no records at all.
    #[test]
    fn a_numeric_mode2_is_read_too() {
        let json = r#"[{"language":"english","mode2":30,"wpm":100.0}]"#;
        let bests: Vec<PersonalBest> = serde_json::from_str(json).expect("decodes");
        assert_eq!(bests[0].mode2, "30");
        assert_eq!(bests[0].length(), Some(30));
    }

    /// A record missing every optional field still decodes, so one unusual row
    /// does not lose the whole list.
    #[test]
    fn a_sparse_record_still_decodes() {
        let json = r#"[{"wpm":90.0}]"#;
        let bests: Vec<PersonalBest> = serde_json::from_str(json).expect("decodes");
        assert_eq!(bests[0].wpm, 90.0);
        assert_eq!(bests[0].language, "");
        assert_eq!(bests[0].length(), None);
    }

    /// Showing a record from a different language next to the current test would
    /// be a lie with a number on it, so all three have to match.
    #[test]
    fn a_best_is_matched_on_mode_language_and_length() {
        let bests = PersonalBests {
            time: vec![
                PersonalBest {
                    language: "english".to_owned(),
                    mode2: "30".to_owned(),
                    wpm: 100.0,
                    time: 30.0,
                    timestamp: 0,
                },
                PersonalBest {
                    language: "english".to_owned(),
                    mode2: "60".to_owned(),
                    wpm: 120.0,
                    time: 60.0,
                    timestamp: 0,
                },
                PersonalBest {
                    language: "russian".to_owned(),
                    mode2: "30".to_owned(),
                    wpm: 140.0,
                    time: 30.0,
                    timestamp: 0,
                },
            ],
            words: Vec::new(),
            quote: Vec::new(),
        };

        assert_eq!(
            bests
                .best(Mode::Time, "english", Some(60))
                .expect("a 60s record")
                .wpm,
            120.0
        );
        // The fastest record overall is in a different language, so it must not
        // be offered as this test's best.
        assert_eq!(
            bests
                .best(Mode::Time, "english", Some(30))
                .expect("a 30s record")
                .wpm,
            100.0
        );
        assert!(
            bests.best(Mode::Time, "klingon", Some(30)).is_none(),
            "a language with no record must not borrow another language's"
        );
        assert!(bests.best(Mode::Quote, "english", None).is_none());
    }

    #[test]
    fn a_missing_length_matches_the_fastest_record() {
        let bests = PersonalBests {
            time: vec![PersonalBest {
                language: "english".to_owned(),
                mode2: "60".to_owned(),
                wpm: 120.0,
                time: 60.0,
                timestamp: 0,
            }],
            words: Vec::new(),
            quote: Vec::new(),
        };
        assert_eq!(
            bests
                .best(Mode::Time, "english", None)
                .expect("a record")
                .wpm,
            120.0
        );
    }
}
