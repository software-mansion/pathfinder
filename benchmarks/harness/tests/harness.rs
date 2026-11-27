//! End-to-end tests: `bench run` stops on an unimplemented scenario before
//! measuring, the mock `rpc` scenario goes through run and compare, and
//! `bench compare` works on a run directory.

use std::path::Path;

use pathfinder_bench::compare::{compare_run_dir, MetricsConfig, PrimaryRule, Summary};
use pathfinder_bench::results::{
    write_json,
    Measurement,
    Metadata,
    ResultsWriter,
    Row,
    METADATA_FILE,
    RESULTS_FILE,
    SUMMARY_MD,
};
use pathfinder_bench::run::{run, RunArgs};
use pathfinder_bench::scenarios::{Binary, ScenarioKind};
use pathfinder_bench::stats::{Better, Status};

#[test]
fn unimplemented_scenario_fails_before_measuring() {
    let dir = tempfile::tempdir().unwrap();
    let runs = dir.path().join("runs");
    let args = RunArgs {
        binaries: vec![Binary {
            label: "main".into(),
            path: "/bin/sh".into(),
        }],
        scenarios: vec![ScenarioKind::Sync],
        iterations: 1,
        output_dir: Some(runs.clone()),
    };
    let err = run(&args).unwrap_err().to_string();
    assert!(err.contains("not implemented"), "{err}");
    assert!(!runs.exists(), "no run directory should be created");
}

#[test]
fn mock_rpc_scenario_runs_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let args = RunArgs {
        binaries: ["main", "feat"]
            .into_iter()
            .map(|label| Binary {
                label: label.into(),
                path: "/bin/sh".into(),
            })
            .collect(),
        scenarios: vec![ScenarioKind::Rpc],
        iterations: 3,
        output_dir: Some(dir.path().to_path_buf()),
    };
    let run_dir = run(&args).unwrap();
    let summary = compare_run_dir(
        &run_dir,
        &MetricsConfig::default(),
        Some("main".into()),
        Some("feat".into()),
    )
    .unwrap();
    let metric = summary.metric("rpc", "latency.p50").unwrap();
    assert_eq!((metric.pairs, metric.delta), (3, Some(0.0)));
    assert!(report(&run_dir).contains("latency.p50"));
}

/// A run directory with one metric measured for `main` and `feat` over three
/// iterations.
fn write_run(dir: &Path, main: [f64; 3], feat: [f64; 3]) {
    let metadata = Metadata {
        binaries: [("main", "/bin/main"), ("feat", "/bin/feat")]
            .into_iter()
            .map(|(label, path)| (label.into(), path.into()))
            .collect(),
        ..Default::default()
    };
    write_json(&dir.join(METADATA_FILE), &metadata).unwrap();
    let mut results = ResultsWriter::create(&dir.join(RESULTS_FILE)).unwrap();
    for (label, values) in [("main", main), ("feat", feat)] {
        for (iteration, value) in values.into_iter().enumerate() {
            let measurement = Measurement {
                label: label.into(),
                metric: "time".into(),
                value,
                unit: "s".into(),
                better: Better::Lower,
            };
            results
                .write(&Row::Metric {
                    scenario: "sync".into(),
                    iteration,
                    measurement,
                })
                .unwrap();
        }
    }
}

/// The `summary.md` written by `bench compare`.
fn report(run_dir: &Path) -> String {
    std::fs::read_to_string(run_dir.join(SUMMARY_MD)).unwrap()
}

fn config() -> MetricsConfig {
    MetricsConfig {
        primary: vec![PrimaryRule {
            scenario: "sync".into(),
            metric: "time".into(),
            threshold: 0.05,
        }],
        ..Default::default()
    }
}

fn compare(dir: &Path, baseline: Option<&str>, target: &str) -> Summary {
    compare_run_dir(
        dir,
        &config(),
        baseline.map(Into::into),
        Some(target.into()),
    )
    .unwrap()
}

#[test]
fn slowdown_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    write_run(dir.path(), [10.0, 10.1, 9.9], [12.0, 12.1, 11.9]);

    let summary = compare(dir.path(), Some("main"), "feat");
    let metric = summary.metric("sync", "time").unwrap();
    assert_eq!(metric.status, Some(Status::Regression));
    assert_eq!(metric.pairs, 3);

    let text = report(dir.path());
    assert!(text.contains("**Result: 1 regression**"), "{text}");
}

#[test]
fn target_only_gives_absolute_values() {
    let dir = tempfile::tempdir().unwrap();
    write_run(dir.path(), [10.0; 3], [12.0; 3]);

    let summary = compare(dir.path(), None, "feat");
    assert!(!summary.has_baseline());
    let metric = summary.metric("sync", "time").unwrap();
    assert_eq!((metric.status, metric.target), (None, Some(12.0)));
    assert!(report(dir.path()).contains("No baseline"));
}
