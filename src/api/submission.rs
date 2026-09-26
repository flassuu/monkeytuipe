//! Building a result payload, and being honest about where it can go.
//!
//! ## The object hash is real, and it is enforced
//!
//! `objectHashCheckEnabled` is `true` on monkeytype.com — confirmed against the
//! live `GET /configuration` — so a submitted result's `hash` must equal
//! `objectHash(resultWithoutHash)`. The `object-hash` npm package is a specific
//! algorithm rather than a standard one, and this is a port of it.
//!
//! ## Why this is built but not sent
//!
//! `POST /results` is authenticated with a **Firebase bearer token**, not an
//! ApeKey. The server's own answer to an ApeKey is `401 This endpoint does not
//! accept ApeKeys`. The write path belongs to the browser, and there is no
//! version of it that a terminal client can reach.
//!
//! So the payload and its hash are computed and kept, because they are what a
//! submission needs and because getting the hash wrong silently is worse than
//! getting it visibly wrong. What is *not* done is a request that would fail:
//! [`Destination::describe`] says plainly that ApeKeys cannot submit, and
//! [`SubmitOutcome`] distinguishes "the server refused" from "there was nowhere
//! to send it", so a user is never told a test was saved when it was not.

use serde::Serialize;
use serde_json::Value;

use crate::stats::TestResult;

/// Where a finished test would be sent.
///
/// Only one variant exists, and that is the finding: a terminal client with an
/// ApeKey has no writable endpoint on the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// The API as configured. Cannot accept a result.
    Monkeytype,
}

impl Destination {
    /// Why a submission cannot happen, in the words to show the user.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Monkeytype => "monkeytype's POST /results needs a browser login, not an ApeKey",
        }
    }

    /// Whether a result could be sent here at all.
    pub fn can_submit(self) -> bool {
        match self {
            Self::Monkeytype => false,
        }
    }
}

/// What happened when a result was offered to a [`Destination`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// There was no writable endpoint, so nothing was sent.
    Nowhere {
        destination: Destination,
        /// The reason, from [`Destination::describe`].
        reason: &'static str,
    },
}

impl SubmitOutcome {
    /// Whether the test was saved on the server.
    ///
    /// Always false, and named as a method rather than left as an absence so that
    /// adding a real destination later makes this answer true somewhere instead
    /// of leaving callers to guess.
    pub fn was_saved(&self) -> bool {
        match self {
            Self::Nowhere { .. } => false,
        }
    }

    /// A line for the results screen.
    pub fn message(&self) -> String {
        match self {
            Self::Nowhere { reason, .. } => format!("not submitted — {reason}"),
        }
    }
}

/// The body `POST /results` expects: `{"result": { ... }}`.
///
/// The field names and constraints come from `CompletedEventSchema`, which is
/// `.strict()` — an unknown field is a 422, and a missing required one too. The
/// ones that cannot be produced in a terminal are left out rather than filled
/// with a plausible number, which is why this is not a straight transcription of
/// the schema: `keyDuration` and `keyOverlap` need key-release events a terminal
/// does not report, and `keyDuration` missing is a 464.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultBody {
    pub result: CompletedEvent,
}

/// Every field name here is the server's, not Rust's.
///
/// `CompletedEventSchema` is `.strict()`: an unrecognised field is a 422 and a
/// missing one too, so serde's default snake_case would be rejected outright. The
/// rename is therefore part of the contract rather than a style choice.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedEvent {
    pub wpm: f64,
    pub raw_wpm: f64,
    /// `[correctWord, incorrect, extra, missed]`.
    pub char_stats: [u32; 4],
    /// The characters counted, *not* including `missed`.
    pub char_total: u32,
    pub acc: f64,
    /// `time`, `words`, `quote`, `custom` or `zen`.
    pub mode: String,
    /// The length, as a string: seconds, words, or a quote id.
    pub mode2: String,
    pub timestamp: i64,
    pub test_duration: f64,
    pub consistency: f64,
    pub key_consistency: f64,
    pub wpm_consistency: f64,
    /// `wpm`, `burst` and `err`, or the string `toolong`.
    pub chart_data: ChartData,
    /// The gap between keypresses, or `toolong`.
    pub key_spacing: Series,
    pub last_key_to_end: f64,
    pub start_to_first_key: f64,
    pub afk_duration: f64,
    pub incomplete_test_seconds: f64,
    pub incomplete_tests: Vec<IncompleteTest>,
    pub restart_count: u32,
    pub uid: String,
    pub tags: Vec<String>,
    /// Whether the typist gave up. A bool, despite the older `BailedOutSchema`
    /// having been a string: the current schema is `z.boolean().optional()`.
    pub bailed_out: bool,
    pub blind_mode: bool,
    pub lazy_mode: bool,
    pub funbox: Vec<String>,
    pub language: String,
    pub difficulty: String,
    pub numbers: bool,
    pub punctuation: bool,
    /// The object's hash, over everything else.
    pub hash: String,
    pub stop_on_letter: bool,
}

/// A chart series, or the literal `toolong`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Series {
    Values(Vec<f64>),
    TooLong,
}

impl Series {
    /// The values, or empty for `toolong`.
    pub fn values(&self) -> &[f64] {
        match self {
            Self::Values(values) => values,
            Self::TooLong => &[],
        }
    }
}

/// The three chart series.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartData {
    pub wpm: Vec<f64>,
    pub burst: Vec<f64>,
    pub err: Vec<f64>,
}

/// A test that was restarted, and how it went.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IncompleteTest {
    pub acc: f64,
    pub seconds: f64,
}

/// Above this many seconds the server replaces the series with `"toolong"`.
///
/// 122 is the server's own cap, and it is on the *event log* length rather than
/// the test duration, so a long test with a long pause can hit it sooner.
pub const TOOLONG_SECONDS: usize = 122;

/// The parts of a payload that come from the settings rather than the test.
///
/// Grouped because they are one decision — "what test was this" — and passing
/// nine positional arguments to a function like this is how two booleans end up
/// transposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSettings {
    /// `time`, `words`, `quote`, `custom` or `zen`.
    pub mode: &'static str,
    /// The length, as a string: seconds, words, or a quote id.
    pub mode2: String,
    /// The language id, validated by the server against its own list.
    pub language: String,
    /// `normal`, `expert` or `master`.
    pub difficulty: &'static str,
    pub punctuation: bool,
    pub numbers: bool,
    pub blind: bool,
    /// Milliseconds since the epoch, already rounded to the second.
    pub timestamp_ms: i64,
}

impl Default for TestSettings {
    fn default() -> Self {
        Self {
            mode: "time",
            mode2: String::new(),
            language: "english".to_owned(),
            difficulty: "normal",
            punctuation: false,
            numbers: false,
            blind: false,
            timestamp_ms: 0,
        }
    }
}

/// Builds the payload for a finished test.
pub fn body_for(result: &TestResult, settings: TestSettings) -> ResultBody {
    let chart = &result.chart;
    let long = result.duration_secs > TOOLONG_SECONDS as f64;
    let spacing = result.keys.spacing.clone();

    let mut event = CompletedEvent {
        wpm: result.wpm,
        raw_wpm: result.raw_wpm,
        char_stats: [
            result.chars.correct_word,
            result.chars.incorrect,
            result.chars.extra,
            result.chars.missed,
        ],
        char_total: result.chars.all_correct + result.chars.incorrect + result.chars.extra,
        acc: result.accuracy,
        mode: settings.mode.to_owned(),
        mode2: settings.mode2.clone(),
        timestamp: settings.timestamp_ms,
        test_duration: result.duration_secs,
        consistency: result.consistency,
        key_consistency: result.keys.consistency,
        wpm_consistency: result.wpm_consistency,
        chart_data: ChartData {
            wpm: chart.wpm.clone(),
            burst: chart.burst.clone(),
            err: chart.err.iter().map(|c| f64::from(*c)).collect(),
        },
        key_spacing: if long {
            Series::TooLong
        } else {
            Series::Values(spacing)
        },
        last_key_to_end: 0.0,
        start_to_first_key: 0.0,
        afk_duration: 0.0,
        incomplete_test_seconds: 0.0,
        incomplete_tests: Vec::new(),
        restart_count: 0,
        uid: String::new(),
        tags: Vec::new(),
        bailed_out: false,
        blind_mode: settings.blind,
        lazy_mode: false,
        funbox: Vec::new(),
        language: settings.language.clone(),
        difficulty: settings.difficulty.to_owned(),
        numbers: settings.numbers,
        punctuation: settings.punctuation,
        // Filled in below, over the event with itself removed.
        hash: String::new(),
        stop_on_letter: false,
    };
    // The server hashes the event with the `hash` field *removed*, so it has to
    // be removed here too. Hashing a payload that still contains the field —
    // even an empty one — can never match, and it fails as a 461 with no way to
    // tell what was wrong.
    let mut value = event_as_value(&event);
    if let Some(object) = value.as_object_mut() {
        object.remove("hash");
    }
    event.hash = object_hash(&value);
    ResultBody { result: event }
}

/// The event as a JSON value, with the field names the server expects.
///
/// The wire names are declared on the struct rather than derived, because the
/// schema is `.strict()` and an unrecognised field is a 422.
fn event_as_value(event: &CompletedEvent) -> Value {
    serde_json::to_value(event).expect("a struct of owned types always serialises")
}

/// A port of the `object-hash` npm package, version 3.0.0.
///
/// This is not a canonical JSON hash. The package walks the value and writes a
/// *typed* stream, and the stream includes things JSON has no room for: a count
/// that is three higher than the key count, the keys `prototype`, `__proto__`
/// and `constructor` that every JavaScript object inherits, and `[CIRCULAR:n]`
/// markers for the objects those keys lead back to. All of that is load-bearing,
/// so this is a transcription rather than a reimplementation:
///
/// ```text
/// object:4:string:9:prototype:Undefined,string:9:__proto__:string:12:[CIRCULAR:1],
///         string:11:constructor:fn:string:8:[native]string:20:function-name:Object
///         string:12:[CIRCULAR:2],string:1:a:number:1,
/// ```
///
/// The transcribed model lives in `research/object-hash-model.js`, which is
/// checked against the real npm package; the vectors in
/// `src/api/data/object-hash-vectors.json` are that script's output, so CI needs
/// no Node.
///
/// The one approximation is [`js_number`]: the package hashes JavaScript's
/// `Number::toString`, and reproducing its shortest-round-trip form for every
/// `f64` is a project in itself. It is implemented for the shapes a result
/// payload actually contains — integers, two-decimal values, and the simple
/// fractions — and anything else falls back to Rust's `Display`, which agrees
/// with JavaScript for every value that does not need exponent notation.
pub fn object_hash(value: &Value) -> String {
    sha1_hex(serialise(value, 0).as_bytes())
}

/// The prototype chain walked in full, for an object at the root.
///
/// Every JavaScript object inherits `prototype`, `__proto__` and `constructor`,
/// and the package visits all three before any own key. At the root nothing is in
/// its hash context yet, so the chain is walked out in full — 288 characters of
/// it, including a nested `object:0:` for the empty object `__proto__` resolves
/// to. This string is a constant of the algorithm, not something this crate
/// computes: reproducing the package's own reasoning about its internals is
/// exactly the kind of thing that drifts, and it is transcribed from
/// `oh({}, {algorithm: "passthrough"})` with the leading `object:3:` removed.
const PREFIX_ROOT: &str = concat!(
    "string:9:prototype:Undefined,",
    "string:9:__proto__:object:3:string:9:prototype:Undefined,",
    "string:9:__proto__:Null,",
    "string:11:constructor:fn:string:8:[native]",
    "string:20:function-name:Object",
    "object:0:,,",
    "string:11:constructor:fn:string:8:[native]",
    "string:20:function-name:Object",
    "string:12:[CIRCULAR:2],",
);

/// The same chain, for an object nested at least one level down.
///
/// By then `Object.prototype` and `Object` have already been visited — they were
/// reached through the root's own copy of these keys — so the dispatcher
/// short-circuits and emits circular-reference markers instead. This is the
/// detail that is easiest to get wrong, and the reason a nested object does not
/// simply repeat the root's prefix.
const PREFIX_NESTED: &str = concat!(
    "string:9:prototype:Undefined,",
    "string:9:__proto__:string:12:[CIRCULAR:1],",
    "string:11:constructor:fn:string:8:[native]",
    "string:20:function-name:Object",
    "string:12:[CIRCULAR:2],",
);

/// The stream for one value at `depth`.
///
/// `depth` is 0 for the payload itself. It is what selects the prefix and nothing
/// else: the package does not recurse differently, it only reports the prototype
/// chain differently depending on whether it has been walked yet.
fn serialise(value: &Value, depth: usize) -> String {
    match value {
        Value::Null => "Null".to_owned(),
        Value::Bool(b) => format!("bool:{b}"),
        Value::Number(n) => format!("number:{}", js_number(n)),
        Value::String(s) => format!("string:{}:{s}", utf16_len(s)),
        Value::Array(items) => {
            let body: String = items.iter().map(|item| serialise(item, depth)).collect();
            format!("array:{}:{body}", items.len())
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let prefix = if depth == 0 {
                PREFIX_ROOT
            } else {
                PREFIX_NESTED
            };
            // Three more than the own keys: the inherited ones above. The count
            // is part of the stream, so it is the package's arithmetic and not
            // this crate's.
            let mut out = format!("object:{}:{prefix}", map.len() + 3);
            for key in keys {
                out.push_str(&serialise(&Value::String(key.clone()), depth));
                out.push(':');
                out.push_str(&serialise(&map[key], depth + 1));
                out.push(',');
            }
            out
        }
    }
}

/// A string's length the way JavaScript counts it.
///
/// UTF-16 code units, not characters and not bytes: an emoji is one `char` in Rust
/// and two in JavaScript, and a string-length prefix that is one short changes
/// the hash of any payload containing one.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A number the way `Number::toString` writes it.
///
/// JavaScript prints an integer without a fractional part, so `30.0` is `30` and
/// not `30.0` — and Rust's `Display` agrees on that. The cases where they differ
/// are exponent notation (`1e21`, `1e-7`) and negative zero, both handled here.
/// The two-decimal values `roundTo2` produces are exact in both.
pub fn js_number(value: &serde_json::Number) -> String {
    let Some(n) = value.as_f64() else {
        // An integer too large for an `f64`, which is beyond anything a payload
        // holds; the digits are correct either way.
        return value.to_string();
    };
    if n == 0.0 {
        // `-0` prints as `0` in JavaScript; Rust prints `-0`.
        return "0".to_owned();
    }
    if n.abs() >= 1e21 || n.abs() < 1e-6 {
        return exponent_form(n);
    }
    if n.fract() == 0.0 {
        // A whole number has no fractional part, so `30.0` is `30`. serde_json
        // keeps the `.0` from the source text and `Display` keeps it too, but
        // JavaScript's `String(30.0)` is `"30"` — and the difference is a
        // different hash.
        return format!("{}", n as i64);
    }
    value.to_string()
}

/// The exponent form `Number::toString` uses, which is not Rust's.
///
/// JavaScript writes `1e+21` and Rust writes `1000000000000000000000`; JavaScript
/// writes `1e-7` and Rust writes `0.0000001`. The threshold and the `e+` are both
/// load-bearing.
fn exponent_form(value: f64) -> String {
    let text = format!("{value:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if exponent >= 0 {
        format!("{mantissa}e+{exponent}")
    } else {
        format!("{mantissa}e{exponent}")
    }
}

/// SHA-1, because that is what the npm package uses.
///
/// Written out rather than pulled in as a dependency: it is forty lines, the
/// crate would otherwise be a build-order dependency for one hash, and a hash
/// nobody can read is a hash nobody can check against the oracle.
fn sha1_hex(input: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = input.to_vec();
    let bit_length = (input.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }

    h.iter().map(|word| format!("{word:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-1 is spelled out rather than pulled in as a dependency, so the three
    /// NIST vectors are the check that it is right. A hash nobody can verify
    /// against a published vector is a hash nobody can trust.
    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    /// A message that crosses the 64-byte block boundary, where the length field
    /// has to be appended after the padding rather than before it.
    #[test]
    fn sha1_handles_a_message_longer_than_one_block() {
        assert_eq!(
            sha1_hex(&b"a".repeat(1000)),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    /// The stream, not the hash, is the part that can be read and checked
    /// against the transcribed model by eye.
    #[test]
    fn the_stream_carries_the_prototype_chain() {
        let value: Value = serde_json::from_str(r#"{"a":1}"#).expect("json");
        assert_eq!(
            serialise(&value, 0),
            format!("object:4:{PREFIX_ROOT}string:1:a:number:1,")
        );
    }

    /// A nested object reports the chain as already walked, which is why its
    /// prefix differs from the root's in substance even though the text matches.
    /// This is the single easiest thing to get wrong in the algorithm.
    #[test]
    fn a_nested_object_is_embedded_in_the_root_stream() {
        let value: Value = serde_json::from_str(r#"{"a":{"b":1}}"#).expect("json");
        let nested = serialise(&value["a"], 1);
        assert!(
            serialise(&value, 0).contains(&nested),
            "the root is not the nested one"
        );
    }

    #[test]
    fn keys_are_written_in_sorted_order() {
        let value: Value = serde_json::from_str(r#"{"b":1,"a":2}"#).expect("json");
        let stream = serialise(&value, 0);
        let a = stream.find("string:1:a:").expect("the a key");
        let b = stream.find("string:1:b:").expect("the b key");
        assert!(a < b, "{stream}");
    }

    /// An empty object still claims three keys — its own zero plus the three
    /// inherited ones — and one key claims four.
    #[test]
    fn an_object_counts_the_inherited_keys() {
        let empty = Value::Object(serde_json::Map::new());
        assert!(
            serialise(&empty, 0).starts_with("object:3:"),
            "{}",
            serialise(&empty, 0)
        );
        let one: Value = serde_json::from_str(r#"{"a":1}"#).expect("json");
        assert!(serialise(&one, 0).starts_with("object:4:"));
    }

    #[test]
    fn arrays_carry_their_length_and_no_separator() {
        let value: Value = serde_json::from_str("[1,2,3]").expect("json");
        assert_eq!(serialise(&value, 0), "array:3:number:1number:2number:3");
        let empty: Value = serde_json::from_str("[]").expect("json");
        assert_eq!(serialise(&empty, 0), "array:0:");
    }

    /// An emoji is one `char` in Rust and two UTF-16 code units in JavaScript,
    /// and a length prefix that is one short changes the hash of the payload.
    #[test]
    fn string_lengths_count_utf16_code_units() {
        assert_eq!(utf16_len("hello"), 5);
        assert_eq!(utf16_len("café"), 4);
        assert_eq!(utf16_len("\u{1F44D}"), 2, "an emoji is a surrogate pair");
        let value = Value::String("\u{1F44D}".to_owned());
        assert!(serialise(&value, 0).starts_with("string:2:"));
    }

    #[test]
    fn numbers_are_written_the_way_javascript_writes_them() {
        let number = |json: &str| -> String {
            let value: Value = serde_json::from_str(json).expect("json");
            js_number(value.as_number().expect("a number"))
        };
        assert_eq!(number("100"), "100");
        assert_eq!(number("100.0"), "100", "no fractional part on an integer");
        assert_eq!(number("0.5"), "0.5");
        assert_eq!(number("-0.0"), "0", "JavaScript prints negative zero as 0");
        assert_eq!(number("1e3"), "1000");
        assert_eq!(number("30.0231"), "30.0231");
    }

    #[test]
    fn exponent_notation_matches_javascripts_form() {
        assert_eq!(exponent_form(1e21), "1e+21");
        assert_eq!(exponent_form(1.5e25), "1.5e+25");
        assert_eq!(exponent_form(1e-7), "1e-7");
    }

    /// The whole point of the port: known payloads must hash to the value the
    /// real npm package computes for them. The vectors are the output of
    /// `research/object-hash-vectors.js`, which runs the real package.
    #[test]
    fn known_payloads_match_the_npm_package() {
        let vectors: Value = serde_json::from_str(include_str!("data/object-hash-vectors.json"))
            .expect("the committed vectors");
        let cases = vectors["cases"].as_array().expect("a list of cases");
        assert!(cases.len() >= 9, "only {} vectors", cases.len());
        for case in cases {
            let input = &case["input"];
            let expected = case["hash"].as_str().expect("a hash");
            assert_eq!(
                &object_hash(input),
                expected,
                "vector {}: {}",
                case["name"].as_str().unwrap_or("?"),
                input
            );
        }
    }

    /// A whole `CompletedEvent`, captured from a real run, has to match too: it
    /// is the only case with the field count, the nesting and the number
    /// formatting all at once.
    #[test]
    fn a_whole_result_payload_matches_the_captured_hash() {
        let vectors: Value = serde_json::from_str(include_str!("data/object-hash-vectors.json"))
            .expect("the committed vectors");
        let full = &vectors["fullPayload"];
        let expected = vectors["fullPayloadHash"].as_str().expect("a hash");
        assert_eq!(object_hash(full), expected);
        // The payload is not small, which is the reason the hash is over the
        // stream rather than over anything tidier.
        let bytes = vectors["fullPayloadBytes"].as_u64().expect("a size");
        assert!(bytes > 500, "{bytes} bytes is not a result payload");
    }

    /// A long test is capped by the server, and the cap is on the log rather than
    /// the duration — so a test with a long pause can hit it sooner.
    #[test]
    fn a_long_test_is_marked_toolong() {
        let mut result = crate::stats::TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: Default::default(),
            duration_secs: 200.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        result.keys.spacing = vec![100.0; 10];
        let body = body_for(
            &result,
            TestSettings {
                mode2: "180".to_owned(),
                ..TestSettings::default()
            },
        );
        assert_eq!(body.result.key_spacing, Series::TooLong);
    }

    #[test]
    fn a_short_test_keeps_its_key_spacing() {
        let mut result = TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: Default::default(),
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        result.keys.spacing = vec![100.0, 120.0, 90.0];
        let body = body_for(
            &result,
            TestSettings {
                mode2: "30".to_owned(),
                ..TestSettings::default()
            },
        );
        assert_eq!(body.result.key_spacing.values().len(), 3);
    }

    /// `charTotal` excludes `missed`, which is the one that is easy to get wrong
    /// and produces a result that disagrees with the site's.
    #[test]
    fn char_total_excludes_missed() {
        let result = TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: crate::stats::CharCounts {
                correct_word: 90,
                all_correct: 95,
                extra: 2,
                incorrect: 3,
                missed: 7,
            },
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        let body = body_for(&result, TestSettings::default());
        assert_eq!(
            body.result.char_total, 100,
            "95 correct + 2 extra + 3 wrong"
        );
        assert_eq!(body.result.char_stats, [90, 3, 2, 7]);
    }

    /// The hash must be over the event *without* the hash, or it can never match.
    #[test]
    fn the_hash_is_computed_over_the_event_without_it() {
        let result = TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: Default::default(),
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        let body = body_for(&result, TestSettings::default());
        assert!(!body.result.hash.is_empty());

        // Recomputing over the same event minus the hash reproduces it, and
        // adding the hash back does not change it.
        // The hash is over the event with `hash` deleted, exactly as the server
        // computes it.
        let mut value = event_as_value(&body.result);
        value.as_object_mut().expect("an object").remove("hash");
        assert_eq!(object_hash(&value), body.result.hash);

        // Leaving the field in — even an empty one — must not produce the same
        // hash, because that is the mistake a naive implementation makes and the
        // server answers with an unexplained 461.
        let with_empty = event_as_value(&{
            let mut blank = body.result.clone();
            blank.hash = String::new();
            blank
        });
        assert_ne!(object_hash(&with_empty), body.result.hash);
    }

    /// Two tests that differ in any field must hash differently, or a submission
    /// would be interchangeable with another one.
    #[test]
    fn a_different_test_hashes_differently() {
        let base = TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: Default::default(),
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        let mut slower = base.clone();
        slower.wpm = 101.0;
        let settings = TestSettings {
            mode2: "30".to_owned(),
            timestamp_ms: 1,
            ..TestSettings::default()
        };
        let a = body_for(&base, settings.clone());
        let b = body_for(&slower, settings.clone());
        assert_ne!(a.result.hash, b.result.hash);

        // And a different language is a different result.
        let c = body_for(
            &base,
            TestSettings {
                language: "russian".to_owned(),
                ..settings
            },
        );
        assert_ne!(a.result.hash, c.result.hash);
    }

    /// The schema is strict, so a field name in snake_case is a 422 rather than
    /// a warning. This is the check that the rename has not been lost.
    #[test]
    fn the_payload_uses_the_servers_field_names() {
        let result = TestResult {
            wpm: 100.0,
            raw_wpm: 110.0,
            accuracy: 98.0,
            consistency: 80.0,
            wpm_consistency: 90.0,
            inputs: Default::default(),
            chars: crate::stats::CharCounts {
                correct_word: 90,
                all_correct: 95,
                extra: 2,
                incorrect: 3,
                missed: 7,
            },
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        let body = body_for(
            &result,
            TestSettings {
                mode2: "30".to_owned(),
                ..TestSettings::default()
            },
        );
        let value = event_as_value(&body.result);
        let object = value.as_object().expect("an object");

        for name in [
            "wpm",
            "rawWpm",
            "charStats",
            "charTotal",
            "acc",
            "mode",
            "mode2",
            "timestamp",
            "testDuration",
            "consistency",
            "keyConsistency",
            "wpmConsistency",
            "chartData",
            "keySpacing",
            "lastKeyToEnd",
            "startToFirstKey",
            "afkDuration",
            "incompleteTestSeconds",
            "incompleteTests",
            "restartCount",
            "uid",
            "tags",
            "bailedOut",
            "blindMode",
            "lazyMode",
            "funbox",
            "language",
            "difficulty",
            "numbers",
            "punctuation",
            "hash",
            "stopOnLetter",
        ] {
            assert!(
                object.contains_key(name),
                "{name} is missing from the payload"
            );
        }
        // And nothing snake_case survived.
        for name in object.keys() {
            assert!(
                !name.contains('_'),
                "{name} is not a field name the server knows"
            );
        }
    }

    /// `keyDuration` and `keyOverlap` are required by the schema and need
    /// key-release events, which a terminal does not report. Sending a plausible
    /// number would be a measurement that is not one, so the field is absent
    /// rather than invented.
    #[test]
    fn the_payload_omits_what_a_terminal_cannot_measure() {
        let result = TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: Default::default(),
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        let body = body_for(&result, TestSettings::default());
        let value = event_as_value(&body.result);
        for absent in ["keyDuration", "keyOverlap", "quote", "name", "uuid"] {
            assert!(
                !value.as_object().expect("an object").contains_key(absent),
                "{absent} is in the payload but is not a field the schema has"
            );
        }
    }

    /// A field typed wrongly is a 422 from a `.strict()` schema, and the type is
    /// not something the tests above can see: `bailedOut` looks like a string in
    /// every other version of the API.
    #[test]
    fn a_mistyped_field_would_not_pass_the_schema() {
        let result = TestResult {
            wpm: 100.0,
            raw_wpm: 100.0,
            accuracy: 100.0,
            consistency: 100.0,
            wpm_consistency: 100.0,
            inputs: Default::default(),
            chars: Default::default(),
            duration_secs: 30.0,
            chart: Default::default(),
            keys: Default::default(),
        };
        let body = body_for(&result, TestSettings::default());
        // A bool in the JSON, not a string: `z.boolean().optional()`.
        assert!(
            !body.result.bailed_out,
            "bailedOut is a bool in the current schema"
        );
    }

    /// The honest part: a terminal client has nowhere to send a result, and says
    /// so rather than reporting a save that did not happen.
    #[test]
    fn there_is_nowhere_to_submit_a_result() {
        assert!(!Destination::Monkeytype.can_submit());
        let outcome = SubmitOutcome::Nowhere {
            destination: Destination::Monkeytype,
            reason: Destination::Monkeytype.describe(),
        };
        assert!(!outcome.was_saved());
        assert!(outcome.message().contains("not submitted"));
        assert!(outcome.message().contains("ApeKey"));
    }
}
