# Benchmarks

`harness/` is `bench`, a tool that measures pathfinder binaries on one
machine and compares them. It runs every scenario it has. For now that is a
mock `rpc` scenario, which shows the pipeline end to end but measures nothing
real.

Every iteration measures all binaries back to back, in alternating order, so
machine noise affects them equally, and the values of one iteration are
compared as a pair.

## Running in CI

Start the Benchmark workflow from the Actions tab:

- **Use workflow from** picks the version of the workflow and the harness,
  usually `main`. It is not what gets measured.
- **target** is the ref to measure: a branch, tag or commit.
- **baseline** is the ref to compare it with, e.g. `main` or the latest
  release. Leave it empty to measure the target alone.

The workflow builds both refs, runs them on a new VM and deletes the VM at
the end. While it runs, the "Wait for the VM" step shows the VM's log. The
report is on the run's summary page, with links to both commits, and the
results are also an artifact of the run. A regression shows as a warning but
doesn't fail the run.

The GCP setup is in `infra/`.

## Reading the report

A report with a baseline ends with a legend. In short: ❌ is a regression and
✅ an improvement, both only when the whole confidence interval of the change
lies beyond the metric's threshold. `~` means no clear change.

Next to the report, a run directory holds `summary.json` with the same data,
`metadata.json` with the binaries and the machine, and `results.jsonl` with
the raw measurements. `bench compare` can compare a run again at any time.

## Tests

```
cargo test --manifest-path benchmarks/harness/Cargo.toml
```
