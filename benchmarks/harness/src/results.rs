//! Files of a run directory and their formats.
//!
//! - `results.jsonl`: one JSON row per line. Everything else is computed from
//!   it.
//! - `metadata.json`: binaries, machine and harness commit.
//! - `summary.json` and `summary.md`: the comparison and its Markdown report,
//!   written by `bench compare`.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::stats::Better;

/// Version of the `summary.json` format. Increase it on incompatible changes.
pub const SCHEMA_VERSION: u32 = 1;
pub const RESULTS_FILE: &str = "results.jsonl";
pub const METADATA_FILE: &str = "metadata.json";
pub const SUMMARY_JSON: &str = "summary.json";
pub const SUMMARY_MD: &str = "summary.md";

/// Contents of `metadata.json`: which binaries were measured and on which
/// machine. Scenario settings and inputs are fixed in the code, so the
/// harness commit identifies them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    /// Path of each binary, by label (from `--binary LABEL=PATH`).
    pub binaries: BTreeMap<String, PathBuf>,
    pub machine: Machine,
    /// Commit of the repository the harness ran from.
    pub harness_commit: Option<String>,
}

/// The machine a run happened on. Fields that couldn't be read are `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Machine {
    /// OS and kernel, e.g. `Linux 24.04 Ubuntu, kernel 6.8.0`.
    pub system: Option<String>,
    /// E.g. `arm64`.
    pub arch: String,
    /// Number of logical CPUs.
    pub cpus: Option<usize>,
    /// CPU model, e.g. `Apple M3 Pro`.
    pub cpu: Option<String>,
    /// Total memory.
    pub memory_bytes: Option<u64>,
}

impl Machine {
    /// Read the CPU, memory and OS of this machine.
    pub fn detect() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let os = match (System::long_os_version(), System::kernel_version()) {
            (Some(os), Some(kernel)) => Some(format!("{os}, kernel {kernel}")),
            (os, kernel) => os.or(kernel),
        };
        Self {
            system: os,
            arch: System::cpu_arch(),
            cpus: Some(system.cpus().len()).filter(|&n| n > 0),
            cpu: system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().trim().to_string())
                .filter(|brand| !brand.is_empty()),
            memory_bytes: Some(system.total_memory()).filter(|&bytes| bytes > 0),
        }
    }
}

/// One value measured for one binary in one iteration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    /// Label of the measured binary.
    pub label: String,
    /// Metric name, unique within the scenario, e.g. `latency.p99`.
    pub metric: String,
    pub value: f64,
    /// Unit of `value`: `s`, `bytes`, `%` or any other short name. The report
    /// formats `s` and `bytes` in a readable way.
    pub unit: String,
    /// Whether a lower or a higher value is better. `bench compare` uses it
    /// to tell a regression from an improvement.
    pub better: Better,
}

/// A measurement that failed, e.g. a request that returned an error. Any
/// failure marks the scenario invalid, so its results never count as
/// regressions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    /// Label of the binary that failed.
    pub label: String,
    pub reason: String,
}

/// One row of `results.jsonl`: a measurement or a failure, with the scenario
/// and iteration it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Row {
    Metric {
        scenario: String,
        iteration: usize,
        #[serde(flatten)]
        measurement: Measurement,
    },
    Invalid {
        scenario: String,
        iteration: usize,
        #[serde(flatten)]
        failure: Failure,
    },
}

/// Appends rows to `results.jsonl`. It flushes after every row, so a run
/// that crashes still leaves the rows written so far.
pub struct ResultsWriter {
    file: File,
}

impl ResultsWriter {
    /// Open the file for appending, creating it if needed.
    pub fn create(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self { file })
    }

    /// Append one row and flush.
    pub fn write(&mut self, row: &Row) -> Result<()> {
        let mut line = serde_json::to_string(row)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        Ok(())
    }
}

/// Read all rows of `results.jsonl`, skipping empty lines.
pub fn read_results(path: &Path) -> Result<Vec<Row>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut rows = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str(&line)
            .with_context(|| format!("{}:{}: invalid row", path.display(), i + 1))?;
        rows.push(row);
    }
    Ok(rows)
}

/// Write `value` as pretty JSON. It writes a temporary file in the same
/// directory and renames it, so a reader never sees a half-written file.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.persist(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Read and parse a JSON file.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RESULTS_FILE);
        let rows = [
            Row::Metric {
                scenario: "rpc".into(),
                iteration: 0,
                measurement: Measurement {
                    label: "target".into(),
                    metric: "latency.p50".into(),
                    value: 1.5,
                    unit: "s".into(),
                    better: Better::Lower,
                },
            },
            Row::Invalid {
                scenario: "rpc".into(),
                iteration: 0,
                failure: Failure {
                    label: "target".into(),
                    reason: "boom".into(),
                },
            },
        ];
        let mut writer = ResultsWriter::create(&path).unwrap();
        for row in &rows {
            writer.write(row).unwrap();
        }
        drop(writer);

        assert_eq!(read_results(&path).unwrap(), rows);
        // The measurement's fields sit next to the row's own, not nested.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with(r#"{"kind":"metric","scenario":"rpc","iteration":0,"label":"target""#),
            "{text}"
        );
    }

    #[test]
    fn detects_machine() {
        let machine = Machine::detect();
        assert!(machine.cpus.unwrap() >= 1);
        assert!(machine.memory_bytes.unwrap() > 0);
    }
}
