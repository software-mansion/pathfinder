//! RPC load scenario. For now a mock that returns fixed numbers without
//! starting the binaries, so the whole flow (run, compare, report) can be
//! tried end to end.
//!
//! TODO: start each binary on a prepared database, replay a fixed mix of
//! requests and measure latency percentiles and throughput.

use anyhow::Result;

use super::{Binary, Iteration};
use crate::results::Measurement;
use crate::stats::Better;

pub struct RpcScenario;

impl RpcScenario {
    /// Every binary gets the same made-up latency, so Δ is always 0.
    pub fn run_iteration(&mut self, order: &[&Binary]) -> Result<Iteration> {
        // TODO: Change this to actually measure the binaries.
        let measurements = order
            .iter()
            .map(|binary| Measurement {
                label: binary.label.clone(),
                metric: "latency.p50".into(),
                value: 0.010,
                unit: "s".into(),
                better: Better::Lower,
            })
            .collect();
        Ok(Iteration {
            measurements,
            failures: vec![],
        })
    }
}
