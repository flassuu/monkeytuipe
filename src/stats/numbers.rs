//! Number helpers, ported from monkeytype's `packages/util/src/numbers.ts` and
//! `frontend/src/ts/utils/numbers.ts`.
//!
//! The goal is bit-for-bit agreement with the JavaScript originals, including
//! the two places where Rust's defaults differ and would silently skew a score:
//!
//! - `Math.round` rounds halves towards **positive** infinity; Rust's `f64::round`
//!   rounds halves **away from zero**. [`js_round`] bridges that.
//! - `mean` and `stdDev` return `0` for an empty array in the original (the
//!   `reduce` throws and is caught), not `NaN`.

/// `Math.round`: halves go towards positive infinity, unlike [`f64::round`].
///
/// `-0.5` becomes `0`, not `-1`.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Rounds to two decimal places, as `roundTo2` does.
pub fn round_to2(x: f64) -> f64 {
    js_round((x + f64::EPSILON) * 100.0) / 100.0
}

/// Words per minute from a character count over a duration.
///
/// A non-positive duration yields `0` rather than `inf`, as in the original.
pub fn calculate_wpm(char_count: f64, duration_seconds: f64) -> f64 {
    if duration_seconds <= 0.0 {
        return 0.0;
    }
    char_count / 5.0 / (duration_seconds / 60.0)
}

/// Arithmetic mean. An empty slice is `0`, matching the original's catch-all.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Population standard deviation — the divisor is `n`, not `n - 1`.
///
/// An empty slice is `0`, matching the original.
pub fn std_dev(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = mean(values);
    let variance = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / values.len() as f64;
    variance.sqrt()
}

/// Maps a coefficient of variation onto `0..=100`.
pub fn kogasa(cov: f64) -> f64 {
    100.0 * (1.0 - (cov + cov.powi(3) / 3.0 + cov.powi(5) / 5.0).tanh())
}

/// Consistency of a series: `kogasa(std_dev / mean)`.
///
/// Returns `0` when the mean is `0`, because the website substitutes `0` for a
/// `NaN` consistency rather than sending it.
pub fn consistency(series: &[f64]) -> f64 {
    let mean = mean(series);
    if mean == 0.0 || !mean.is_finite() {
        return 0.0;
    }
    round_to2(kogasa(std_dev(series) / mean))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-checked against `packages/util/__test__/numbers.spec.ts`.
    #[test]
    fn js_round_matches_javascript() {
        assert_eq!(js_round(0.5), 1.0);
        assert_eq!(
            js_round(-0.5),
            0.0,
            "JS rounds halves up, not away from zero"
        );
        assert_eq!(js_round(1.5), 2.0);
        assert_eq!(js_round(-1.5), -1.0);
        assert_eq!(js_round(2.4), 2.0);
        assert_eq!(js_round(2.6), 3.0);
        assert_eq!(js_round(-0.6), -1.0);
    }

    #[test]
    fn f64_round_would_have_disagreed() {
        // Documents why js_round exists rather than f64::round.
        assert_ne!(js_round(-0.5), (-0.5f64).round());
    }

    #[test]
    fn round_to2_keeps_two_decimals() {
        assert_eq!(round_to2(1.005), 1.01);
        assert_eq!(round_to2(2.345), 2.35);
        assert_eq!(round_to2(2.344), 2.34);
        assert_eq!(round_to2(100.0), 100.0);
        assert_eq!(round_to2(0.0), 0.0);
        assert_eq!(round_to2(74.3059), 74.31);
    }

    #[test]
    fn calculate_wpm_uses_five_characters_per_word() {
        // 300 characters in 60 seconds is 60 wpm.
        assert_eq!(calculate_wpm(300.0, 60.0), 60.0);
        assert_eq!(calculate_wpm(150.0, 30.0), 60.0);
        assert_eq!(calculate_wpm(0.0, 30.0), 0.0);
    }

    #[test]
    fn calculate_wpm_guards_against_non_positive_durations() {
        assert_eq!(calculate_wpm(100.0, 0.0), 0.0);
        assert_eq!(calculate_wpm(100.0, -5.0), 0.0);
        assert!(!calculate_wpm(100.0, 0.0).is_infinite());
    }

    #[test]
    fn mean_and_std_dev_of_an_empty_slice_are_zero_not_nan() {
        assert_eq!(mean(&[]), 0.0);
        assert_eq!(std_dev(&[]), 0.0);
        assert!(!mean(&[]).is_nan());
        assert!(!std_dev(&[]).is_nan());
    }

    #[test]
    fn mean_and_std_dev_match_hand_computed_values() {
        let values = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert_eq!(mean(&values), 5.0);
        // Population deviation: sqrt(32 / 8) = 2.
        assert!((std_dev(&values) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn std_dev_uses_the_population_divisor() {
        // Sample deviation would be sqrt(4/1) = 2; population is sqrt(4/2).
        let values = [1.0, 3.0];
        assert!((std_dev(&values) - 1.0).abs() < 1e-12, "divisor must be n");
    }

    #[test]
    fn kogasa_maps_zero_cov_to_a_hundred() {
        assert!((kogasa(0.0) - 100.0).abs() < 1e-12);
    }

    #[test]
    fn kogasa_decreases_as_covariance_rises() {
        let perfect = kogasa(0.0);
        let good = kogasa(0.2);
        let poor = kogasa(1.0);
        let awful = kogasa(3.0);
        assert!(perfect > good, "{perfect} > {good}");
        assert!(good > poor, "{good} > {poor}");
        assert!(poor > awful, "{poor} > {awful}");
    }

    #[test]
    fn kogasa_matches_the_javascript_original() {
        // Values produced by the upstream `kogasa`, checked against node.
        assert!((kogasa(0.0) - 100.0).abs() < 1e-9);
        assert!((kogasa(0.1) - 90.0).abs() < 1e-3, "got {}", kogasa(0.1));
        assert!((kogasa(0.2) - 80.0002).abs() < 1e-3, "got {}", kogasa(0.2));
        assert!((kogasa(0.5) - 50.1043).abs() < 1e-3, "got {}", kogasa(0.5));
        assert!((kogasa(1.0) - 8.9007).abs() < 1e-3, "got {}", kogasa(1.0));
    }

    #[test]
    fn kogasa_saturates_rather_than_going_negative() {
        // tanh saturates, so a wildly inconsistent series clamps near 0
        // instead of turning negative the way a plain sigmoid would.
        assert!(
            kogasa(2.0) >= 0.0 && kogasa(2.0) < 1e-6,
            "got {}",
            kogasa(2.0)
        );
        assert!(kogasa(3.0).abs() < 1e-9, "got {}", kogasa(3.0));
        assert!(kogasa(10.0).abs() < 1e-9, "got {}", kogasa(10.0));
    }

    #[test]
    fn consistency_of_a_flat_series_is_a_hundred() {
        assert_eq!(consistency(&[60.0, 60.0, 60.0, 60.0]), 100.0);
    }

    #[test]
    fn consistency_of_an_all_zero_series_is_zero_not_nan() {
        let value = consistency(&[0.0, 0.0, 0.0]);
        assert_eq!(value, 0.0);
        assert!(!value.is_nan());
    }

    #[test]
    fn consistency_of_an_empty_series_is_zero() {
        assert_eq!(consistency(&[]), 0.0);
    }
}
