//! Statistics for comparing two binaries: medians, paired relative changes
//! and bootstrap confidence intervals.

use rand::rngs::ChaCha8Rng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use statrs::statistics::{Data, OrderStatistics};

/// Which direction of change is good for a metric: `Lower` for times and
/// sizes, `Higher` for throughput.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Better {
    Lower,
    Higher,
}

/// Verdict on a primary metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The target is worse by more than the threshold.
    Regression,
    /// The target is better by more than the threshold.
    Improvement,
    /// No change beyond the threshold, or the noise is too large to tell.
    Neutral,
    /// Can't be judged, e.g. the scenario is invalid or there are too few
    /// pairs.
    Invalid,
}

/// Median of a non-empty slice.
pub fn median(xs: &[f64]) -> f64 {
    assert!(!xs.is_empty(), "median of empty slice");
    Data::new(xs.to_vec()).median()
}

/// Confidence interval of the median of `samples`, by the percentile
/// bootstrap: resample with replacement `resamples` times and take the
/// quantiles of the resampled medians (statrs, R-8 definition).
///
/// ChaCha8 gives the same numbers on every platform and version, so the same
/// data and seed always give the same interval.
pub fn bootstrap_median_ci(
    samples: &[f64],
    resamples: usize,
    confidence: f64,
    seed: u64,
) -> (f64, f64) {
    assert!(!samples.is_empty(), "bootstrap of empty slice");
    let n = samples.len();
    if n == 1 {
        return (samples[0], samples[0]);
    }
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut buf = vec![0.0; n];
    let medians: Vec<f64> = (0..resamples)
        .map(|_| {
            for slot in buf.iter_mut() {
                *slot = samples[rng.random_range(0..n)];
            }
            Data::new(buf.as_mut_slice()).median()
        })
        .collect();
    let mut medians = Data::new(medians);
    let alpha = (1.0 - confidence) / 2.0;
    (medians.quantile(alpha), medians.quantile(1.0 - alpha))
}

/// Relative change in each pair, `target / baseline - 1`. Fails if a
/// baseline value is 0 or the slices differ in length.
pub fn paired_ratios(baseline: &[f64], target: &[f64]) -> anyhow::Result<Vec<f64>> {
    anyhow::ensure!(baseline.len() == target.len(), "pair count mismatch");
    baseline
        .iter()
        .zip(target)
        .map(|(b, t)| {
            anyhow::ensure!(*b != 0.0, "baseline value is 0, relative change undefined");
            Ok(t / b - 1.0)
        })
        .collect()
}

/// Status of a primary metric from the confidence interval of its relative
/// change.
///
/// `ci` and `threshold` are fractions. Only an interval that lies entirely
/// beyond the threshold counts as a regression or an improvement.
pub fn classify(better: Better, ci: (f64, f64), threshold: f64) -> Status {
    let (lo, hi) = ci;
    match better {
        Better::Lower if lo > threshold => Status::Regression,
        Better::Lower if hi < -threshold => Status::Improvement,
        Better::Higher if hi < -threshold => Status::Regression,
        Better::Higher if lo > threshold => Status::Improvement,
        _ => Status::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_even_and_odd() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }

    #[test]
    fn bootstrap_is_deterministic_and_brackets_the_median() {
        let xs = [0.01, 0.03, -0.02, 0.05, 0.0];
        let a = bootstrap_median_ci(&xs, 2000, 0.95, 7);
        let b = bootstrap_median_ci(&xs, 2000, 0.95, 7);
        assert_eq!(a, b);
        assert!(a.0 <= median(&xs) && median(&xs) <= a.1);
    }

    #[test]
    fn bootstrap_single_sample() {
        assert_eq!(bootstrap_median_ci(&[0.2], 100, 0.95, 0), (0.2, 0.2));
    }

    #[test]
    fn paired_ratios_checks_input() {
        assert_eq!(
            paired_ratios(&[2.0, 4.0], &[3.0, 2.0]).unwrap(),
            vec![0.5, -0.5]
        );
        assert!(paired_ratios(&[0.0], &[1.0]).is_err());
        assert!(paired_ratios(&[1.0], &[1.0, 2.0]).is_err());
    }

    #[test]
    fn classify_lower_is_better() {
        let t = 0.05;
        assert_eq!(classify(Better::Lower, (0.06, 0.10), t), Status::Regression);
        assert_eq!(
            classify(Better::Lower, (-0.10, -0.06), t),
            Status::Improvement
        );
        // Straddles the threshold: can't be told apart from noise.
        assert_eq!(classify(Better::Lower, (0.02, 0.14), t), Status::Neutral);
        assert_eq!(classify(Better::Lower, (-0.08, -0.01), t), Status::Neutral);
        // On the threshold is not beyond it.
        assert_eq!(classify(Better::Lower, (0.05, 0.09), t), Status::Neutral);
    }

    #[test]
    fn classify_higher_is_better() {
        let t = 0.5;
        assert_eq!(
            classify(Better::Higher, (-2.0, -0.6), t),
            Status::Regression
        );
        assert_eq!(classify(Better::Higher, (0.6, 2.0), t), Status::Improvement);
        assert_eq!(classify(Better::Higher, (-0.6, 0.1), t), Status::Neutral);
    }
}
