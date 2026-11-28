//! Scenarios: what `bench run` measures.
//!
//! A scenario gets all binaries at once and chooses how finely to interleave
//! them: per input, per load stage or per whole run. The runner only gives
//! it the order of the binaries for each iteration.
//!
//! `bench run` runs every scenario. Only `rpc` exists, as a mock. The TODOs in
//! [`Scenario::all`] describe the scenarios to come.

use std::path::PathBuf;

use anyhow::Result;

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
/// [`Scenario::all`], [`Scenario::name`] and [`Scenario::run_iteration`].
pub enum Scenario {
    Rpc(rpc::RpcScenario),
}

impl Scenario {
    /// Set up every implemented scenario, in the order `bench run` runs them.
    ///
    /// TODO: more scenarios.
    /// - `sync`: sync a fixed block range from a fixed starting database and
    ///   measure the wall time and blocks per second.
    /// - `db-size`: the database size after that block range, per table if
    ///   pathfinder can report it.
    /// - `compile`: compile time. A working version (9 fixture classes grouped
    ///   by size, interleaved per class) is in
    ///   `benchmarks/archive/harness-with-compile.patch`.
    pub fn all() -> Result<Vec<Self>> {
        // A mock for now, see `rpc.rs`.
        Ok(vec![Self::Rpc(rpc::RpcScenario)])
    }

    /// Name of the scenario in the results and the report.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Rpc(_) => "rpc",
        }
    }

    /// Measure every binary once, in the given order.
    pub fn run_iteration(&mut self, order: &[&Binary]) -> Result<Iteration> {
        match self {
            Self::Rpc(scenario) => scenario.run_iteration(order),
        }
    }
}
