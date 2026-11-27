//! `bench run`: measures pathfinder binaries on one machine and saves the raw
//! results. `bench compare` compares them later.
//!
//! Each iteration runs all binaries back to back, and the first binary
//! changes every iteration (A then B, B then A, and so on). Slow drift and
//! machine noise then affect all binaries equally, and values from the same
//! iteration can be compared as pairs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{ensure, Context as _, Result};
use clap::builder::RangedU64ValueParser;
use clap::Args;
use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use crate::results::{
    write_json,
    Machine,
    Metadata,
    ResultsWriter,
    Row,
    METADATA_FILE,
    RESULTS_FILE,
};
use crate::scenarios::{Binary, Iteration, Scenario, ScenarioKind};

/// Print a progress line with the current time to stderr.
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!(
            "[{}] {}",
            now(format_description!("[hour]:[minute]:[second]")),
            format_args!($($arg)*)
        )
    };
}

/// Arguments of `bench run`.
#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// A binary to measure, as `LABEL=PATH`, e.g. `v0.24.3=./pathfinder-old`.
    /// Repeat the flag for each binary. The label names the binary in the
    /// results (a version, a branch or anything else), and `bench compare`
    /// refers to it.
    #[arg(
        long = "binary",
        value_name = "LABEL=PATH",
        required = true,
        value_parser = parse_binary
    )]
    pub binaries: Vec<Binary>,
    /// Scenarios to run, comma-separated.
    #[arg(long, value_delimiter = ',', required = true)]
    pub scenarios: Vec<ScenarioKind>,
    /// How many times to measure the binaries. Each iteration measures all of
    /// them, in alternating order.
    #[arg(
        long,
        value_name = "N",
        default_value_t = 3,
        value_parser = RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub iterations: usize,
    /// Where to create the run directory (default: benchmarks/runs). The run
    /// directory is named after the date, time and binary labels.
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,
}

/// Run the scenarios and return the run directory with the results.
pub fn run(args: &RunArgs) -> Result<PathBuf> {
    let repo = repo_root();
    let binaries = resolve_binaries(&args.binaries)?;

    // Set up every scenario first. An unimplemented scenario then fails right
    // away instead of after the others have run for a long time.
    let mut scenarios = args
        .scenarios
        .iter()
        .map(|&kind| Ok((kind, Scenario::new(kind)?)))
        .collect::<Result<Vec<_>>>()?;

    let runs_dir = match &args.output_dir {
        Some(dir) => absolute_path(dir),
        None => repo.join("benchmarks/runs"),
    };
    let run_dir = create_run_dir(&runs_dir, &binaries)?;

    let metadata = Metadata {
        binaries: binaries
            .iter()
            .map(|binary| (binary.label.clone(), binary.path.clone()))
            .collect(),
        machine: Machine::detect(),
        harness_commit: git_commit(&repo),
    };
    write_json(&run_dir.join(METADATA_FILE), &metadata)?;

    let mut results = ResultsWriter::create(&run_dir.join(RESULTS_FILE))?;
    for (kind, scenario) in &mut scenarios {
        run_scenario(*kind, scenario, &binaries, args.iterations, &mut results)?;
    }

    log!("results in {}", run_dir.display());
    Ok(run_dir)
}

/// Measure one scenario `iterations` times and append the results.
fn run_scenario(
    kind: ScenarioKind,
    scenario: &mut Scenario,
    binaries: &[Binary],
    iterations: usize,
    results: &mut ResultsWriter,
) -> Result<()> {
    let name = kind.as_str();
    for index in 0..iterations {
        let order = iteration_order(binaries, index);
        log!(
            "{name}: iteration {}/{} ({})",
            index + 1,
            iterations,
            labels(&order).join(" → ")
        );
        let iteration = scenario.run_iteration(&order)?;
        save_iteration(name, index, iteration, results)?;
    }
    Ok(())
}

/// Append the measurements and failures of one iteration to `results.jsonl`.
fn save_iteration(
    scenario: &str,
    iteration: usize,
    result: Iteration,
    results: &mut ResultsWriter,
) -> Result<()> {
    for measurement in result.measurements {
        results.write(&Row::Metric {
            scenario: scenario.into(),
            iteration,
            measurement,
        })?;
    }
    for failure in result.failures {
        log!(
            "{scenario}: invalid: {} ({})",
            failure.reason,
            failure.label
        );
        results.write(&Row::Invalid {
            scenario: scenario.into(),
            iteration,
            failure,
        })?;
    }
    Ok(())
}

/// Parse `--binary LABEL=PATH`, e.g. `v0.24.3=./pathfinder`.
fn parse_binary(value: &str) -> Result<Binary, String> {
    match value.split_once('=') {
        Some((label, path)) if !label.is_empty() && !path.is_empty() => Ok(Binary {
            label: label.to_string(),
            path: PathBuf::from(path),
        }),
        _ => Err("expected LABEL=PATH, e.g. v0.24.3=./pathfinder".into()),
    }
}

/// Check that the labels are unique and the files exist, and make the paths
/// absolute. Keeps the order from the command line. Whether a binary runs at
/// all shows up when a scenario starts it.
fn resolve_binaries(binaries: &[Binary]) -> Result<Vec<Binary>> {
    let mut seen = HashSet::new();
    binaries
        .iter()
        .map(|binary| {
            let label = &binary.label;
            ensure!(seen.insert(label), "binary label {label:?} given twice");
            let path = absolute_path(&binary.path);
            ensure!(path.is_file(), "{label}: {} not found", path.display());
            Ok(Binary {
                label: label.clone(),
                path,
            })
        })
        .collect()
}

/// Create a new directory for this run inside `runs_dir`, so the results of
/// two runs never mix.
fn create_run_dir(runs_dir: &Path, binaries: &[Binary]) -> Result<PathBuf> {
    std::fs::create_dir_all(runs_dir)
        .with_context(|| format!("creating {}", runs_dir.display()))?;
    let timestamp = now(format_description!(
        "[year]-[month]-[day]_[hour]-[minute]-[second]"
    ));
    let run_dir = runs_dir.join(run_dir_name(&timestamp, binaries));
    // `create_dir`, unlike `create_dir_all`, fails if the directory exists.
    std::fs::create_dir(&run_dir).with_context(|| format!("creating {}", run_dir.display()))?;
    Ok(run_dir)
}

/// Name of a run directory: `<timestamp>_<label>[_vs_<label>...]`, e.g.
/// `2026-11-27_14-03-12_v0.24.3_vs_my-branch`. Characters that aren't safe in
/// a directory name, such as `/`, become `-`.
fn run_dir_name(timestamp: &str, binaries: &[Binary]) -> String {
    let safe_labels: Vec<String> = labels(binaries)
        .into_iter()
        .map(|label| {
            label
                .chars()
                .map(|c| match c {
                    'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => c,
                    _ => '-',
                })
                .collect()
        })
        .collect();
    format!("{timestamp}_{}", safe_labels.join("_vs_"))
}

/// Order of the binaries in iteration `index`: as given in even iterations,
/// reversed in odd ones.
fn iteration_order(binaries: &[Binary], index: usize) -> Vec<&Binary> {
    if index.is_multiple_of(2) {
        binaries.iter().collect()
    } else {
        binaries.iter().rev().collect()
    }
}

/// Labels of the binaries, in order. Accepts both `&[Binary]` and
/// `&[&Binary]`.
fn labels<B: std::borrow::Borrow<Binary>>(binaries: &[B]) -> Vec<&str> {
    binaries
        .iter()
        .map(|binary| binary.borrow().label.as_str())
        .collect()
}

/// Root of the repository this crate was built from, two levels above
/// `benchmarks/harness`. The default `benchmarks/runs` directory is inside
/// it.
fn repo_root() -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    repo.canonicalize().unwrap_or(repo)
}

/// Current commit of the git repository at `root`, if git is available.
fn git_commit(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Current UTC time in the given format.
fn now(format: &[BorrowedFormatItem<'_>]) -> String {
    OffsetDateTime::now_utc()
        .format(format)
        .expect("formatting the current time never fails")
}

/// Absolute path without `..`. For a missing file it still returns an
/// absolute path, and the caller reports the error.
fn absolute_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary(label: &str) -> Binary {
        Binary {
            label: label.into(),
            path: PathBuf::new(),
        }
    }

    #[test]
    fn order_alternates() {
        let binaries = [binary("a"), binary("b")];
        assert_eq!(labels(&iteration_order(&binaries, 0)), ["a", "b"]);
        assert_eq!(labels(&iteration_order(&binaries, 1)), ["b", "a"]);
        assert_eq!(labels(&iteration_order(&binaries[..1], 1)), ["a"]);
    }

    #[test]
    fn binary_flag() {
        assert_eq!(
            parse_binary("v0.24.3=./old").unwrap(),
            Binary {
                label: "v0.24.3".into(),
                path: PathBuf::from("./old"),
            }
        );
        assert!(parse_binary("./old").is_err());
        assert!(parse_binary("=./old").is_err());
    }

    #[test]
    fn run_dir_name_uses_labels() {
        let ts = "2026-11-27_14-03-12";
        let both = [binary("v0.24.3"), binary("feature/fast")];
        assert_eq!(
            run_dir_name(ts, &both),
            "2026-11-27_14-03-12_v0.24.3_vs_feature-fast"
        );
        assert_eq!(
            run_dir_name(ts, &[binary("main")]),
            "2026-11-27_14-03-12_main"
        );
    }
}
