//! Scenarios: what `bench run` measures.
//!
//! A scenario gets all binaries at once and chooses how finely to interleave
//! them: per input, per load stage or per whole run. The runner only gives
//! it the order of the binaries for each iteration.
//!
//! Only `rpc` exists, as a mock. The TODOs in [`Scenario::new`] describe what
//! the other scenarios will measure.

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::results::{Failure, Measurement};

pub mod rpc;

/// A binary to measure, from `--binary LABEL=PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
    /// Name of the binary in the results and in `bench compare`, e.g. a
    /// version or a branch.
    pub label: String,
    /// Absolute path to the executable.
    pub path: PathBuf,
}

/// Everything one iteration of a scenario produced. The runner writes it to
/// `results.jsonl`.
#[derive(Debug, Default)]
pub struct Iteration {
    pub measurements: Vec<Measurement>,
    pub failures: Vec<Failure>,
}

/// A scenario set up and ready to measure. Each implemented scenario is one
/// variant holding its own state, so calls are dispatched with a `match`,
/// without trait objects.
///
/// To add a scenario: add a variant with its struct, and an arm for it in
/// [`Scenario::new`] and in [`Scenario::run_iteration`].
pub enum Scenario {
    Rpc(rpc::RpcScenario),
}

impl Scenario {
    /// Set up a scenario. Fails for a scenario that isn't implemented, so
    /// `bench run` can stop before measuring anything.
    pub fn new(kind: ScenarioKind) -> Result<Self> {
        match kind {
            // A mock for now, see `rpc.rs`.
            ScenarioKind::Rpc => Ok(Self::Rpc(rpc::RpcScenario)),
            // TODO: sync a fixed block range from a fixed starting database
            // and measure the wall time and blocks per second.
            ScenarioKind::Sync => bail!("the sync scenario is not implemented yet"),
            // TODO: measure the database size after the sync scenario's block
            // range, per table if pathfinder can report it.
            ScenarioKind::DbSize => bail!("the db-size scenario is not implemented yet"),
            // TODO: measure compile time. A working version (9 fixture
            // classes grouped by size, interleaved per class) is in
            // `benchmarks/archive/harness-with-compile.patch`.
            ScenarioKind::Compile => bail!("the compile scenario is not implemented yet"),
        }
    }

    /// Measure every binary once, in the given order.
    pub fn run_iteration(&mut self, order: &[&Binary]) -> Result<Iteration> {
        match self {
            Self::Rpc(scenario) => scenario.run_iteration(order),
        }
    }
}

/// The scenarios `bench run` knows. The value is also the scenario's name on
/// the command line (`--scenarios rpc,sync`) and in the results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum ScenarioKind {
    /// JSON-RPC load against a running node.
    Rpc,
    /// Time to sync a range of blocks.
    Sync,
    /// Size of the database after syncing.
    DbSize,
    /// Sierra to CASM compilation through `pathfinder compile`.
    Compile,
}

impl ScenarioKind {
    /// Name of the scenario in the results, the same as on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Sync => "sync",
            Self::DbSize => "db-size",
            Self::Compile => "compile",
        }
    }
}
