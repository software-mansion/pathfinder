//! Command line of `bench`. The work happens in the library.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use pathfinder_bench::compare::{compare_run_dir, MetricsConfig};
use pathfinder_bench::report;
use pathfinder_bench::run::{run, RunArgs};

/// Command line arguments.
#[derive(Parser)]
#[command(name = "bench", about = "Pathfinder benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

/// Subcommands.
#[derive(Subcommand)]
enum Cmd {
    /// Measure one or more binaries and save the raw results.
    Run {
        #[command(flatten)]
        args: RunArgs,
    },
    /// Compare two binaries of a run, write summary.json and summary.md, and
    /// print the report.
    Compare {
        /// Run directory created by `bench run`.
        run_dir: PathBuf,
        /// Label of the binary to compare against. Leave out for absolute
        /// numbers only.
        #[arg(long, value_name = "LABEL")]
        baseline: Option<String>,
        /// Label of the binary being judged (default: the only binary in the
        /// run).
        #[arg(long, value_name = "LABEL")]
        target: Option<String>,
    },
}

/// Prints the error chain to stderr and exits with code 2 if a command
/// fails.
fn main() -> ExitCode {
    match _main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// Run the chosen subcommand.
fn _main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Run { args } => {
            run(&args)?;
        }
        Cmd::Compare {
            run_dir,
            baseline,
            target,
        } => {
            let summary = compare_run_dir(&run_dir, &MetricsConfig::default(), baseline, target)?;
            print!("{}", report::render(&summary));
        }
    }
    Ok(())
}
