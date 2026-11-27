//! Benchmark harness for pathfinder.
//!
//! It only runs pathfinder binaries and never links pathfinder code, so it
//! can compare any versions.
//!
//! There are two commands: `bench run` measures binaries and saves raw
//! results ([`run`]), and `bench compare` compares two of them ([`compare`])
//! and writes a Markdown report ([`report`]).

pub mod compare;
pub mod report;
pub mod results;
pub mod run;
pub mod scenarios;
pub mod stats;
