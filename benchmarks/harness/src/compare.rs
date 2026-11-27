//! `bench compare`: reads `results.jsonl` and writes `summary.json` and
//! `summary.md`.
//!
//! For each metric, the baseline and target values from the same iteration
//! form a pair. The reported change is the median of the relative changes in
//! the pairs, with a bootstrap confidence interval. Only primary metrics (see
//! [`MetricsConfig`]) get a regression or improvement status. The rest are
//! informational.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::report;
use crate::results::{
    self,
    Metadata,
    Row,
    METADATA_FILE,
    RESULTS_FILE,
    SCHEMA_VERSION,
    SUMMARY_JSON,
    SUMMARY_MD,
};
use crate::stats::{self, Better, Status};

/// Settings of the statistical comparison. They are saved in `summary.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    /// Number of bootstrap resamples.
    pub resamples: usize,
    /// Confidence level of the interval, e.g. 0.95.
    pub confidence: f64,
    /// Base seed of the bootstrap. Each metric mixes in its own name, so
    /// re-running `bench compare` gives the same intervals.
    pub seed: u64,
    /// With fewer pairs than this, a metric can't be judged.
    pub min_pairs: usize,
}

impl Default for Comparison {
    fn default() -> Self {
        Self {
            resamples: 10_000,
            confidence: 0.95,
            seed: 3616,
            min_pairs: 3,
        }
    }
}

/// Makes the matching metrics primary: they can be marked as a regression or
/// an improvement. Metrics that no rule matches are informational.
#[derive(Debug, Clone)]
pub struct PrimaryRule {
    /// Scenario name, e.g. `rpc`.
    pub scenario: String,
    /// Glob over metric names, e.g. `latency.*`.
    pub metric: String,
    /// Smallest relative change that counts, as a fraction (0.05 = 5%).
    pub threshold: f64,
}

/// How `bench compare` judges metrics.
#[derive(Debug, Clone, Default)]
pub struct MetricsConfig {
    pub comparison: Comparison,
    // TODO: add a rule per scenario as they're implemented (e.g. RPC latency,
    // sync time, database size). Thresholds start as guesses and are
    // calibrated with A/A runs.
    pub primary: Vec<PrimaryRule>,
}

impl MetricsConfig {
    /// The rule that makes this metric primary, if any.
    pub fn find_primary(&self, scenario: &str, metric: &str) -> Option<&PrimaryRule> {
        self.primary.iter().find(|rule| {
            rule.scenario == scenario
                && glob::Pattern::new(&rule.metric).is_ok_and(|p| p.matches(metric))
        })
    }
}

/// Which binaries of a run to compare, by label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    /// Binary to compare against. Without one, the summary has only absolute
    /// values.
    pub baseline: Option<String>,
    /// Binary being judged.
    pub target: String,
}

impl Selection {
    /// Check the chosen labels against the labels in the run. If the run has
    /// one binary, `target` can be left out.
    pub fn resolve(
        labels: &[String],
        baseline: Option<String>,
        target: Option<String>,
    ) -> Result<Self> {
        let target = match (target, labels) {
            (Some(target), _) => target,
            (None, [only]) => only.clone(),
            (None, _) => bail!(
                "the run has binaries {}; choose them with --target (and --baseline)",
                labels.join(", ")
            ),
        };
        for label in std::iter::once(&target).chain(baseline.as_ref()) {
            ensure!(
                labels.contains(label),
                "no binary {label:?} in the run; it has {}",
                labels.join(", ")
            );
        }
        Ok(Self { baseline, target })
    }
}

/// Contents of `summary.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// Format version, see [`SCHEMA_VERSION`].
    pub schema: u32,
    pub selection: Selection,
    pub comparison: Comparison,
    /// Copy of `metadata.json`, so the report needs only this file.
    pub metadata: Metadata,
    pub scenarios: Vec<ScenarioSummary>,
}

/// How many primary metrics regressed or improved, and how many scenarios
/// are invalid.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub regression: usize,
    pub improvement: usize,
    pub invalid: usize,
}

impl Summary {
    /// Whether there is a baseline. Without one there are no changes and no
    /// statuses.
    pub fn has_baseline(&self) -> bool {
        self.selection.baseline.is_some()
    }

    pub fn counts(&self) -> Counts {
        let sum = |status| self.scenarios.iter().map(|sc| sc.count(status)).sum();
        Counts {
            regression: sum(Status::Regression),
            improvement: sum(Status::Improvement),
            invalid: self
                .scenarios
                .iter()
                .filter(|sc| sc.status() == Status::Invalid)
                .count(),
        }
    }

    /// Find a metric by scenario and name.
    pub fn metric(&self, scenario: &str, metric: &str) -> Option<&MetricSummary> {
        self.scenarios
            .iter()
            .find(|sc| sc.name == scenario)?
            .metrics
            .iter()
            .find(|m| m.metric == metric)
    }
}

/// The comparison of one scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSummary {
    pub name: String,
    /// Failed measurements. Any failure makes the scenario invalid.
    pub invalid: Vec<String>,
    /// Primary metrics first, then by name.
    pub metrics: Vec<MetricSummary>,
}

impl ScenarioSummary {
    /// Number of metrics with the given status.
    pub fn count(&self, status: Status) -> usize {
        self.metrics
            .iter()
            .filter(|m| m.status == Some(status))
            .count()
    }

    /// Overall status: invalid if a measurement failed or a primary metric
    /// can't be judged, otherwise the worst status of the primary metrics.
    pub fn status(&self) -> Status {
        [Status::Invalid, Status::Regression, Status::Improvement]
            .into_iter()
            .find(|&status| {
                self.count(status) > 0 || (status == Status::Invalid && !self.invalid.is_empty())
            })
            .unwrap_or(Status::Neutral)
    }
}

/// The comparison of one metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSummary {
    pub metric: String,
    pub unit: String,
    pub better: Better,
    /// Threshold of the primary rule, as a fraction. `None` for
    /// informational metrics.
    pub threshold: Option<f64>,
    /// Median of the baseline's values over all iterations.
    pub baseline: Option<f64>,
    /// Median of the target's values over all iterations.
    pub target: Option<f64>,
    /// Number of iterations with a value for both binaries.
    pub pairs: usize,
    /// Median relative change over the pairs, as a fraction.
    pub delta: Option<f64>,
    /// Confidence interval of `delta`.
    pub ci: Option<(f64, f64)>,
    /// Only primary metrics with a baseline get a status.
    pub status: Option<Status>,
    /// Why there is no `delta` or why the status is invalid.
    pub note: Option<String>,
}

impl MetricSummary {
    pub fn is_primary(&self) -> bool {
        self.threshold.is_some()
    }
}

/// Rows of one scenario, grouped by metric.
#[derive(Default)]
struct ScenarioRows {
    invalid: Vec<String>,
    metrics: BTreeMap<String, Series>,
}

/// All values of one metric.
struct Series {
    unit: String,
    better: Better,
    /// Label, then iteration, then value.
    values: BTreeMap<String, BTreeMap<usize, f64>>,
}

/// Group rows by scenario and metric. Fails if one metric appears with two
/// different units or directions.
fn group(rows: &[Row]) -> Result<BTreeMap<String, ScenarioRows>> {
    let mut scenarios = BTreeMap::<String, ScenarioRows>::new();
    for row in rows {
        match row {
            Row::Metric {
                scenario,
                iteration,
                measurement: m,
            } => {
                let series = scenarios
                    .entry(scenario.clone())
                    .or_default()
                    .metrics
                    .entry(m.metric.clone())
                    .or_insert_with(|| Series {
                        unit: m.unit.clone(),
                        better: m.better,
                        values: BTreeMap::new(),
                    });
                ensure!(
                    series.unit == m.unit && series.better == m.better,
                    "inconsistent unit or direction for {scenario}/{}",
                    m.metric
                );
                series
                    .values
                    .entry(m.label.clone())
                    .or_default()
                    .insert(*iteration, m.value);
            }
            Row::Invalid {
                scenario,
                iteration,
                failure,
            } => scenarios
                .entry(scenario.clone())
                .or_default()
                .invalid
                .push(format!(
                    "{} ({}, iteration {iteration})",
                    failure.reason, failure.label
                )),
        }
    }
    Ok(scenarios)
}

/// Compare the selected binaries on every metric in `rows`.
pub fn compare(
    rows: &[Row],
    cfg: &MetricsConfig,
    metadata: Metadata,
    selection: Selection,
) -> Result<Summary> {
    let scenarios = group(rows)?
        .into_iter()
        .map(|(name, rows)| {
            let mut metrics: Vec<MetricSummary> = rows
                .metrics
                .iter()
                .map(|(metric, series)| {
                    let rule = cfg.find_primary(&name, metric);
                    let seed = cfg.comparison.seed ^ hash64(&format!("{name}/{metric}"));
                    compare_metric(metric, series, &selection, rule, &cfg.comparison, seed)
                })
                .collect();
            if !rows.invalid.is_empty() {
                // A primary metric of an invalid scenario can't be judged.
                for m in metrics.iter_mut().filter(|m| m.status.is_some()) {
                    m.status = Some(Status::Invalid);
                    m.note = Some("scenario invalid".into());
                }
            }
            metrics.sort_by(|a, b| (!a.is_primary(), &a.metric).cmp(&(!b.is_primary(), &b.metric)));
            ScenarioSummary {
                name,
                invalid: rows.invalid,
                metrics,
            }
        })
        .collect();

    Ok(Summary {
        schema: SCHEMA_VERSION,
        selection,
        comparison: cfg.comparison.clone(),
        metadata,
        scenarios,
    })
}

/// Compare one metric: medians, paired change with its confidence interval
/// and, for a primary metric with a baseline, the status.
fn compare_metric(
    metric: &str,
    series: &Series,
    selection: &Selection,
    rule: Option<&PrimaryRule>,
    cmp: &Comparison,
    seed: u64,
) -> MetricSummary {
    let empty = BTreeMap::new();
    let values =
        |label: Option<&String>| label.and_then(|l| series.values.get(l)).unwrap_or(&empty);
    let base = values(selection.baseline.as_ref());
    let target = values(Some(&selection.target));
    let median = |v: &BTreeMap<usize, f64>| {
        (!v.is_empty()).then(|| stats::median(&v.values().copied().collect::<Vec<_>>()))
    };
    // Values from the iterations measured for both binaries.
    let (b, t): (Vec<f64>, Vec<f64>) = base
        .iter()
        .filter_map(|(i, b)| Some((*b, *target.get(i)?)))
        .unzip();

    let mut summary = MetricSummary {
        metric: metric.to_string(),
        unit: series.unit.clone(),
        better: series.better,
        threshold: rule.map(|r| r.threshold),
        baseline: median(base),
        target: median(target),
        pairs: b.len(),
        delta: None,
        ci: None,
        status: None,
        note: None,
    };

    if !b.is_empty() {
        match stats::paired_ratios(&b, &t) {
            Ok(changes) => {
                summary.delta = Some(stats::median(&changes));
                summary.ci = Some(stats::bootstrap_median_ci(
                    &changes,
                    cmp.resamples,
                    cmp.confidence,
                    seed,
                ));
            }
            Err(e) => summary.note = Some(e.to_string()),
        }
    }

    if let (Some(rule), Some(_)) = (rule, &selection.baseline) {
        summary.status = Some(match summary.ci {
            None => {
                summary
                    .note
                    .get_or_insert("no baseline/target pairs".into());
                Status::Invalid
            }
            Some(_) if summary.pairs < cmp.min_pairs => {
                summary.note = Some(format!(
                    "only {} pairs, need {}",
                    summary.pairs, cmp.min_pairs
                ));
                Status::Invalid
            }
            Some(ci) => stats::classify(series.better, ci, rule.threshold),
        });
    }
    summary
}

/// 64-bit hash of a string that is the same on every run and platform,
/// unlike `std`'s hasher. Gives each metric its own bootstrap seed.
fn hash64(s: &str) -> u64 {
    let digest = Sha256::digest(s.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().expect("digest has 32 bytes"))
}

/// Read a run directory, compare the binaries with the given labels (see
/// [`Selection::resolve`]), and write `summary.json` and `summary.md`.
pub fn compare_run_dir(
    run_dir: &Path,
    cfg: &MetricsConfig,
    baseline: Option<String>,
    target: Option<String>,
) -> Result<Summary> {
    let rows = results::read_results(&run_dir.join(RESULTS_FILE))?;
    let metadata: Metadata = results::read_json(&run_dir.join(METADATA_FILE))?;
    let labels: Vec<String> = metadata.binaries.keys().cloned().collect();
    let selection = Selection::resolve(&labels, baseline, target)?;
    let summary = compare(&rows, cfg, metadata, selection)?;
    results::write_json(&run_dir.join(SUMMARY_JSON), &summary)?;
    std::fs::write(run_dir.join(SUMMARY_MD), report::render(&summary))?;
    Ok(summary)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::results::{Failure, Measurement};

    pub fn cfg() -> MetricsConfig {
        MetricsConfig {
            comparison: Comparison {
                resamples: 2000,
                confidence: 0.95,
                seed: 1,
                min_pairs: 3,
            },
            primary: vec![PrimaryRule {
                scenario: "rpc".into(),
                metric: "latency.*".into(),
                threshold: 0.05,
            }],
        }
    }

    /// Rows of one `rpc` metric in seconds, for the `baseline` and `target`
    /// labels.
    pub fn rows(metric: &str, b: &[f64], t: &[f64]) -> Vec<Row> {
        let mut out = vec![];
        for (label, values) in [("baseline", b), ("target", t)] {
            for (iteration, value) in values.iter().enumerate() {
                out.push(Row::Metric {
                    scenario: "rpc".into(),
                    iteration,
                    measurement: Measurement {
                        label: label.into(),
                        metric: metric.into(),
                        value: *value,
                        unit: "s".into(),
                        better: Better::Lower,
                    },
                });
            }
        }
        out
    }

    /// Compare the `baseline` and `target` labels used by [`rows`].
    pub fn selection() -> Selection {
        Selection {
            baseline: Some("baseline".into()),
            target: "target".into(),
        }
    }

    fn run(rows: &[Row]) -> Summary {
        compare(rows, &cfg(), Metadata::default(), selection()).unwrap()
    }

    fn status(summary: &Summary, metric: &str) -> Option<Status> {
        summary.metric("rpc", metric).unwrap().status
    }

    #[test]
    fn selection_from_labels() {
        let labels = vec!["a".to_string(), "b".to_string()];
        let s = Selection::resolve(&labels, Some("a".into()), Some("b".into())).unwrap();
        assert_eq!((s.baseline.as_deref(), s.target.as_str()), (Some("a"), "b"));
        // Two binaries and no choice is ambiguous.
        assert!(Selection::resolve(&labels, None, None).is_err());
        assert!(Selection::resolve(&labels, None, Some("c".into())).is_err());
        // A single binary is picked by default.
        let one = vec!["a".to_string()];
        assert_eq!(Selection::resolve(&one, None, None).unwrap().target, "a");
    }

    #[test]
    fn primary_metrics() {
        let m = cfg();
        assert!(m.find_primary("rpc", "latency.p50").is_some());
        assert!(m.find_primary("rpc", "throughput").is_none());
        assert!(m.find_primary("sync", "latency.p50").is_none());
    }

    #[test]
    fn regression_improvement_neutral() {
        let mut data = rows("latency.p50", &[1.0; 5], &[1.2, 1.21, 1.19, 1.2, 1.22]);
        data.extend(rows("latency.p90", &[2.0; 5], &[1.6, 1.61, 1.59, 1.6, 1.6]));
        data.extend(rows(
            "latency.p99",
            &[1.0; 5],
            &[1.01, 0.99, 1.0, 1.02, 0.98],
        ));
        let s = run(&data);
        assert_eq!(status(&s, "latency.p50"), Some(Status::Regression));
        assert_eq!(status(&s, "latency.p90"), Some(Status::Improvement));
        assert_eq!(status(&s, "latency.p99"), Some(Status::Neutral));
        assert_eq!(s.scenarios[0].status(), Status::Regression);
        assert_eq!(
            s.counts(),
            Counts {
                regression: 1,
                improvement: 1,
                invalid: 0
            }
        );
    }

    #[test]
    fn noisy_metric_is_not_flagged() {
        // Median +8%, but one pair shows no change, so the interval reaches
        // below the threshold.
        let s = run(&rows(
            "latency.p50",
            &[1.0; 5],
            &[1.08, 1.0, 1.15, 1.08, 1.02],
        ));
        let m = s.metric("rpc", "latency.p50").unwrap();
        assert!((m.delta.unwrap() - 0.08).abs() < 1e-9);
        assert!(m.ci.unwrap().0 < 0.05);
        assert_eq!(m.status, Some(Status::Neutral));
    }

    #[test]
    fn informational_metrics_have_no_status() {
        let s = run(&rows("throughput", &[1.0; 5], &[2.0; 5]));
        let m = s.metric("rpc", "throughput").unwrap();
        assert!(!m.is_primary());
        assert_eq!(m.status, None);
        assert!((m.delta.unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn too_few_pairs_is_invalid() {
        let s = run(&rows("latency.p50", &[1.0, 1.0], &[2.0, 2.0]));
        let m = s.metric("rpc", "latency.p50").unwrap();
        assert_eq!(m.status, Some(Status::Invalid));
        assert!(m.note.as_deref().unwrap().contains("2 pairs"));
    }

    #[test]
    fn failure_makes_scenario_invalid() {
        let mut data = rows("latency.p50", &[1.0; 5], &[2.0; 5]);
        data.push(Row::Invalid {
            scenario: "rpc".into(),
            iteration: 1,
            failure: Failure {
                label: "target".into(),
                reason: "request failed".into(),
            },
        });
        let s = run(&data);
        assert_eq!(
            s.scenarios[0].invalid,
            ["request failed (target, iteration 1)"]
        );
        assert_eq!(status(&s, "latency.p50"), Some(Status::Invalid));
        assert_eq!(
            s.counts(),
            Counts {
                regression: 0,
                improvement: 0,
                invalid: 1
            }
        );
    }

    #[test]
    fn no_baseline_gives_absolute_values() {
        let s = compare(
            &rows("latency.p50", &[], &[1.0, 1.0, 1.0]),
            &cfg(),
            Metadata::default(),
            Selection {
                baseline: None,
                target: "target".into(),
            },
        )
        .unwrap();
        assert!(!s.has_baseline());
        let m = s.metric("rpc", "latency.p50").unwrap();
        assert_eq!((m.status, m.delta, m.target), (None, None, Some(1.0)));
    }

    #[test]
    fn inconsistent_units_are_rejected() {
        let mut data = rows("x", &[1.0], &[1.0]);
        if let Row::Metric { measurement, .. } = &mut data[1] {
            measurement.unit = "bytes".into();
        }
        assert!(compare(&data, &cfg(), Metadata::default(), selection()).is_err());
    }

    #[test]
    fn summary_json_round_trips() {
        let s = run(&rows("latency.p50", &[1.0; 3], &[1.1; 3]));
        let text = serde_json::to_string(&s).unwrap();
        let back: Summary = serde_json::from_str(&text).unwrap();
        assert_eq!(back.counts(), s.counts());
        assert!(text.contains(r#""ci":["#));
    }

    #[test]
    fn hash64_is_stable() {
        assert_eq!(hash64("rpc/latency.p50"), hash64("rpc/latency.p50"));
        assert_ne!(hash64("a"), hash64("b"));
    }
}
