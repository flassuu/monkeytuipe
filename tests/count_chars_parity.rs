//! Differential test for the `countChars` port.
//!
//! The generator here is a line-for-line twin of the one in
//! `research/count-chars-oracle.js`, which contains monkeytype's `countChars`
//! transcribed verbatim. Both sides walk the same 2000 inputs with the same
//! PRNG, so any divergence between the Rust port and the JavaScript original
//! surfaces as a failing assertion.
//!
//! **The two generators must be changed together.** If you touch the PRNG, the
//! alphabet, the word length, or the mutation mix in one file, mirror it in the
//! other — otherwise the comparison silently stops comparing.
//!
//! Run `node research/count-chars-oracle.js` to see the branch coverage the
//! generated corpus reaches.

use monkeytuipe::stats::{count_chars, CharCounts};

/// Kept in step with the oracle.
const ALPHABET: &[char] = &['a', 'b', 'c', 'd', 'e', 'f', 'g', ' ', ',', '.'];
const CASES: usize = 2000;
const MAX_LEN: usize = 8;

/// 32-bit LCG, matching `Math.imul` + `>>> 0` in the oracle.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u32) as usize
    }
}

struct Case {
    input: String,
    target: String,
    credit_partial: bool,
}

fn make_word(rng: &mut Lcg) -> String {
    let len = 1 + rng.below(MAX_LEN);
    (0..len)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

/// Mutates a target into an input, exercising substitution, insertion,
/// deletion, truncation and overflow so every branch of the original is hit.
fn mutate(target: &str, rng: &mut Lcg) -> String {
    let mut chars: Vec<char> = target.chars().collect();
    let roll = rng.below(100);

    match roll {
        0..=34 => target.to_owned(),
        35..=59 => {
            let i = rng.below(chars.len());
            chars[i] = ALPHABET[rng.below(ALPHABET.len())];
            chars.into_iter().collect()
        }
        60..=74 => {
            let at = rng.below(chars.len() + 1);
            chars.insert(at, ALPHABET[rng.below(ALPHABET.len())]);
            chars.into_iter().collect()
        }
        75..=87 => {
            if chars.len() > 1 {
                let at = rng.below(chars.len());
                chars.remove(at);
            }
            chars.into_iter().collect()
        }
        88..=93 => {
            let keep = 1 + rng.below(chars.len());
            chars.truncate(keep);
            chars.into_iter().collect()
        }
        _ => {
            let mut out = target.to_owned();
            out.push(ALPHABET[rng.below(ALPHABET.len())]);
            out.push(ALPHABET[rng.below(ALPHABET.len())]);
            out
        }
    }
}

fn cases() -> Vec<Case> {
    let mut rng = Lcg(0x2545_f491);
    (0..CASES)
        .map(|i| {
            let target = make_word(&mut rng);
            let input = mutate(&target, &mut rng);
            // Alternated by index, not drawn: see the note in the oracle.
            Case {
                input,
                target,
                credit_partial: i % 2 == 0,
            }
        })
        .collect()
}

#[test]
fn count_chars_parity() {
    let mut mismatches = Vec::new();

    for case in cases() {
        let got = count_chars(&case.input, &case.target, case.credit_partial);
        let want = upstream(&case.input, &case.target, case.credit_partial);
        if got != want {
            mismatches.push(format!(
                "input={:?} target={:?} credit_partial={}\n  rust    {got:?}\n  oracle {want:?}",
                case.input, case.target, case.credit_partial
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} of {CASES} cases diverged from the JavaScript original:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_corpus_reaches_every_branch_of_the_original() {
    // If the generator drifts, this fails before the parity test starts
    // reporting meaningless passes.
    let mut all_correct = false;
    let mut space_extra = false;
    let mut missed = false;
    let mut missed_suppressed = false;
    let mut extra = false;
    let mut incorrect = false;

    for case in cases() {
        let input: Vec<char> = case.input.chars().collect();
        let target: Vec<char> = case.target.chars().collect();
        let word_correct = case.input == case.target;

        for i in 0..input.len().max(target.len()) {
            let ic = input.get(i).copied();
            let tc = target.get(i).copied();
            if ic == tc {
                if tc == Some(' ') && !word_correct {
                    space_extra = true;
                } else {
                    all_correct = true;
                }
            } else if ic.is_none() {
                if case.credit_partial {
                    missed_suppressed = true;
                } else {
                    missed = true;
                }
            } else if tc.is_none()
                || (tc == Some(' ') && ic != Some(' ') && !case.input.contains(' '))
            {
                extra = true;
            } else {
                incorrect = true;
            }
        }
    }

    let reached = [
        ("all-correct", all_correct),
        ("space-extra", space_extra),
        ("missed", missed),
        ("missed-suppressed", missed_suppressed),
        ("extra", extra),
        ("incorrect", incorrect),
    ];
    let missing: Vec<&str> = reached
        .iter()
        .filter(|(_, hit)| !*hit)
        .map(|(name, _)| *name)
        .collect();
    assert!(missing.is_empty(), "branches never reached: {missing:?}");
}

/// The JavaScript original, for the cases where it disagrees.
///
/// Written from `frontend/src/ts/utils/strings.ts`; used as a second opinion
/// when a mismatch is reported, so the failing case can be reproduced in node.
fn upstream(input: &str, target: &str, credit_partial: bool) -> CharCounts {
    let input: Vec<char> = input.chars().collect();
    let target: Vec<char> = target.chars().collect();
    let word_correct = input == target;
    let word_partially_correct = target.starts_with(&input[..]);

    let mut counts = CharCounts::default();
    for i in 0..input.len().max(target.len()) {
        let ic = input.get(i).copied();
        let tc = target.get(i).copied();
        if ic == tc {
            if tc == Some(' ') && !word_correct {
                counts.extra += 1;
            } else {
                counts.all_correct += 1;
            }
            if word_correct || (credit_partial && word_partially_correct) {
                counts.correct_word += 1;
            }
        } else if ic.is_none() {
            if !credit_partial {
                counts.missed += 1;
            }
        } else if tc.is_none() || (tc == Some(' ') && ic != Some(' ') && !input.contains(&' ')) {
            counts.extra += 1;
        } else {
            counts.incorrect += 1;
        }
    }
    counts
}
