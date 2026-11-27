//! The Markdown report (`summary.md`) of a [`Summary`]. It only formats:
//! all statuses and counts come from `compare`.

use std::fmt::Write as _;

use humansize::{format_size, BINARY};
use tabled::builder::Builder;
use tabled::settings::Style;

use crate::compare::{MetricSummary, ScenarioSummary, Summary};
use crate::stats::Status;

/// Symbol of a status in the report.
fn icon(status: Option<Status>) -> &'static str {
    match status {
        Some(Status::Regression) => "❌",
        Some(Status::Improvement) => "✅",
        Some(Status::Neutral) => "~",
        Some(Status::Invalid) => "⚠️",
        None => "",
    }
}

/// Three significant digits without an exponent, like `%.3g` in C, e.g.
/// `12.3` or `110`.
fn sig3(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let digits = (2 - x.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{x:.digits$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A value in its unit, readable: seconds as ms, s or minutes, bytes as KiB,
/// MiB or GiB. A missing value is `–`.
pub fn fmt_value(value: Option<f64>, unit: &str) -> String {
    let Some(v) = value else { return "–".into() };
    match unit {
        "s" if v < 1.0 => format!("{} ms", sig3(v * 1000.0)),
        "s" if v < 60.0 => format!("{v:.2} s"),
        "s" => {
            let total = v.round() as u64;
            format!("{}m{:02}s", total / 60, total % 60)
        }
        "bytes" => {
            let size = format_size(v.abs().round() as u64, BINARY.decimal_places(1));
            if v < 0.0 {
                format!("−{size}")
            } else {
                size
            }
        }
        "%" => format!("{v:.2}%"),
        _ => format!("{} {unit}", sig3(v)).trim().to_string(),
    }
}

/// A number with a sign and a typographic minus, e.g. `+1.5` or `−2.0`. A
/// value that rounds to zero is `+0.0`, never `−0.0`.
fn signed(x: f64, digits: usize) -> String {
    let factor = 10f64.powi(digits as i32);
    let x = if (x * factor).round() == 0.0 { 0.0 } else { x };
    format!("{x:+.digits$}").replace('-', "−")
}

/// A relative change (fraction) as a signed percentage, e.g. `+12.5%`.
fn fmt_percent(x: f64) -> String {
    format!("{}%", signed(x * 100.0, 1))
}

/// The metric's change as a percentage, or `–`.
fn fmt_delta(m: &MetricSummary) -> String {
    m.delta.map_or("–".into(), fmt_percent)
}

/// The confidence interval as percentages, e.g. `−1.2…+3.4%`, or `–`.
fn fmt_ci(m: &MetricSummary) -> String {
    m.ci.map_or("–".into(), |(lo, hi)| {
        format!("{}…{}", signed(lo * 100.0, 1), fmt_percent(hi))
    })
}

/// The threshold as a percentage. Empty for informational metrics.
fn fmt_threshold(m: &MetricSummary) -> String {
    m.threshold
        .map_or(String::new(), |t| format!("{}%", sig3(t * 100.0)))
}

/// A Markdown table with columns padded to equal width.
fn markdown_table(head: &[&str], rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let mut builder = Builder::default();
    builder.push_record(head.iter().copied());
    for row in rows {
        builder.push_record(row);
    }
    let mut table = builder.build();
    table.with(Style::markdown());
    format!("{table}\n")
}

/// Append a Markdown table of `metrics`. Primary metrics get the threshold
/// and status columns. Without a baseline, the table has only values.
fn table(out: &mut String, metrics: &[&MetricSummary], has_baseline: bool) {
    let with_status = metrics.iter().any(|m| m.is_primary());
    let head: &[&str] = match (has_baseline, with_status) {
        (false, _) => &["Metric", "Value"],
        (true, false) => &["Metric", "Baseline", "Target", "Δ (paired)", "95% CI"],
        (true, true) => &[
            "Metric",
            "Baseline",
            "Target",
            "Δ (paired)",
            "95% CI",
            "Threshold",
            "Status",
        ],
    };
    let rows = metrics.iter().map(|m| {
        let mut cells = vec![m.metric.clone()];
        if !has_baseline {
            cells.push(fmt_value(m.target, &m.unit));
            return cells;
        }
        cells.extend([
            fmt_value(m.baseline, &m.unit),
            fmt_value(m.target, &m.unit),
            fmt_delta(m),
            fmt_ci(m),
        ]);
        if with_status {
            let mut status = icon(m.status).to_string();
            if let (Some(Status::Invalid), Some(note)) = (m.status, &m.note) {
                status = format!("{status} {note}");
            }
            cells.extend([fmt_threshold(m), status]);
        }
        cells
    });
    out.push_str(&markdown_table(head, rows));
}

/// One row per scenario: its status and how many primary metrics regressed
/// or improved. Without a baseline, only the number of metrics.
fn overview(out: &mut String, summary: &Summary) {
    let link = |name: &str| format!("[{name}](#{name})");
    let text = if summary.has_baseline() {
        markdown_table(
            &["Scenario", "Status", "Regressions", "Improvements"],
            summary.scenarios.iter().map(|sc| {
                vec![
                    link(&sc.name),
                    icon(Some(sc.status())).to_string(),
                    sc.count(Status::Regression).to_string(),
                    sc.count(Status::Improvement).to_string(),
                ]
            }),
        )
    } else {
        markdown_table(
            &["Scenario", "Metrics"],
            summary
                .scenarios
                .iter()
                .map(|sc| vec![link(&sc.name), sc.metrics.len().to_string()]),
        )
    };
    out.push_str(&text);
    out.push('\n');
}

/// Section of one scenario: why it is invalid, if it is, then its primary
/// metrics and a collapsed table of informational metrics. Without primary
/// metrics, the informational table is shown open.
fn scenario_section(out: &mut String, scenario: &ScenarioSummary, has_baseline: bool) {
    let _ = writeln!(out, "### {}\n", scenario.name);

    for reason in &scenario.invalid {
        let _ = writeln!(out, "- ⚠️ invalid: {reason}");
    }
    if !scenario.invalid.is_empty() {
        out.push('\n');
    }

    let (primary, info): (Vec<&MetricSummary>, Vec<&MetricSummary>) =
        scenario.metrics.iter().partition(|m| m.is_primary());

    if !primary.is_empty() {
        table(out, &primary, has_baseline);
        out.push('\n');
    }
    if info.is_empty() {
        return;
    }
    if primary.is_empty() {
        table(out, &info, has_baseline);
        out.push('\n');
    } else {
        let _ = writeln!(
            out,
            "<details>\n<summary>Informational metrics ({})</summary>\n",
            info.len()
        );
        table(out, &info, has_baseline);
        out.push_str("\n</details>\n\n");
    }
}

/// The whole Markdown report: binaries, machine, result line, an overview of
/// the scenarios and one section per scenario.
pub fn render(summary: &Summary) -> String {
    let metadata = &summary.metadata;
    let mut out = String::from("## Benchmark results\n\n");

    if !metadata.binaries.is_empty() {
        let sel = &summary.selection;
        let rows = metadata.binaries.iter().map(|(label, path)| {
            let role = if sel.baseline.as_ref() == Some(label) {
                "baseline"
            } else if &sel.target == label {
                "target"
            } else {
                "–"
            };
            vec![
                label.clone(),
                role.to_string(),
                format!("`{}`", path.display()),
            ]
        });
        out.push_str(&markdown_table(&["Binary", "Role", "Path"], rows));
        out.push('\n');
    }

    let m = &metadata.machine;
    if let Some(cpu) = &m.cpu {
        let cpus = m.cpus.map_or("?".into(), |n| n.to_string());
        let system = m.system.as_deref().unwrap_or("?");
        let _ = writeln!(out, "Machine: {cpu}, {cpus} CPUs, {system}\n");
    }

    if summary.has_baseline() {
        let c = summary.counts();
        let parts: Vec<String> = [
            (c.regression, "regression"),
            (c.improvement, "improvement"),
            (c.invalid, "invalid"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}{}", if n == 1 || w == "invalid" { "" } else { "s" }))
        .collect();
        let text = if parts.is_empty() {
            "no change detected".to_string()
        } else {
            parts.join(", ")
        };
        let _ = writeln!(out, "**Result: {text}**\n");
    } else {
        out.push_str("**No baseline: absolute values only, no status.**\n\n");
    }

    overview(&mut out, summary);
    for scenario in &summary.scenarios {
        scenario_section(&mut out, scenario, summary.has_baseline());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::tests::{cfg, rows, selection};
    use crate::compare::{compare, Selection};
    use crate::results::Metadata;

    /// The report with runs of spaces (column padding) squeezed to one.
    fn squeeze(text: &str) -> String {
        text.split(' ')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn values() {
        assert_eq!(fmt_value(Some(0.0123), "s"), "12.3 ms");
        assert_eq!(fmt_value(Some(0.0761), "s"), "76.1 ms");
        assert_eq!(fmt_value(Some(0.110), "s"), "110 ms");
        assert_eq!(fmt_value(Some(6.2), "s"), "6.20 s");
        assert_eq!(fmt_value(Some(842.0), "s"), "14m02s");
        assert_eq!(fmt_value(Some(3.0 * 1024.0 * 1024.0), "bytes"), "3 MiB");
        assert_eq!(fmt_value(Some(3.5 * 1024.0 * 1024.0), "bytes"), "3.5 MiB");
        assert_eq!(fmt_value(Some(512.0), "bytes"), "512 B");
        assert_eq!(fmt_value(Some(99.98), "%"), "99.98%");
        assert_eq!(fmt_value(Some(20.1), "classes/s"), "20.1 classes/s");
        assert_eq!(fmt_value(None, "s"), "–");
    }

    #[test]
    fn signed_avoids_negative_zero() {
        assert_eq!(signed(-0.0001, 1), "+0.0");
        assert_eq!(signed(-2.55, 1), "−2.5");
        assert_eq!(signed(12.5, 1), "+12.5");
    }

    #[test]
    fn renders_regression_row() {
        let data = rows("latency.p50", &[1.0; 5], &[1.2; 5]);
        let text = squeeze(&render(
            &compare(&data, &cfg(), Metadata::default(), selection()).unwrap(),
        ));
        assert!(text.contains("**Result: 1 regression**"), "{text}");
        assert!(text.contains("| [rpc](#rpc) | ❌ | 1 | 0 |"), "{text}");
        assert!(text.contains("### rpc"), "{text}");
        assert!(
            text.contains("| latency.p50 | 1.00 s | 1.20 s | +20.0% | +20.0…+20.0% | 5% | ❌ |"),
            "{text}"
        );
    }

    #[test]
    fn renders_no_baseline() {
        let selection = Selection {
            baseline: None,
            target: "target".into(),
        };
        let data = rows("latency.p50", &[], &[1.0]);
        let text = render(&compare(&data, &cfg(), Metadata::default(), selection).unwrap());
        assert!(text.contains("No baseline"));
    }
}
