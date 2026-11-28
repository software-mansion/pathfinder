# Benchmarks

`harness/` is `bench`, a tool that measures pathfinder binaries on one
machine and compares them. It runs every scenario it has. For now that is a
mock `rpc` scenario, which shows the pipeline end to end but measures nothing
real.

Every iteration measures all binaries back to back, in alternating order, so
machine noise affects them equally, and the values of one iteration are
compared as a pair.

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
