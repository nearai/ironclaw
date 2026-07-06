use std::{collections::BTreeMap, fmt::Write, fs, path::Path};

use serde_json::{Value, json};

use crate::sweep::RunMetrics;

pub(crate) fn render_comparison_report(
    baseline_path: &Path,
    current: &Value,
) -> Result<String, String> {
    let baseline = load_value(baseline_path)?;
    let baseline_runs = extract_runs(&baseline)?;
    let current_runs = extract_runs(current)?;
    if baseline_runs.is_empty() {
        return Err(format!(
            "comparison baseline {} did not contain comparable metrics",
            baseline_path.display()
        ));
    }
    if current_runs.is_empty() {
        return Err("current run did not contain comparable metrics".to_string());
    }

    let mut output = String::new();
    let _ = writeln!(output, "\nComparison report");
    let _ = writeln!(
        output,
        "{:<22} {:<14} {:>12} {:>12} {:>12}",
        "point", "metric", "baseline", "current", "delta"
    );
    let _ = writeln!(
        output,
        "{:-<22} {:-<14} {:->12} {:->12} {:->12}",
        "", "", "", "", ""
    );

    let mut missing_baseline = Vec::new();
    let mut missing_current = Vec::new();
    for pair in comparison_pairs(&baseline_runs, &current_runs) {
        match (pair.baseline, pair.current) {
            (Some(baseline), Some(current)) => {
                push_metric_rows(&mut output, &pair.label, baseline, current);
            }
            (None, Some(_)) => missing_baseline.push(pair.label),
            (Some(_), None) => missing_current.push(pair.label),
            (None, None) => {}
        }
    }

    if !missing_baseline.is_empty() || !missing_current.is_empty() {
        let _ = writeln!(output, "\nComparison gaps");
        if !missing_baseline.is_empty() {
            let _ = writeln!(
                output,
                "missing_in_baseline {}",
                missing_baseline.join(", ")
            );
        }
        if !missing_current.is_empty() {
            let _ = writeln!(output, "missing_in_current {}", missing_current.join(", "));
        }
    }

    let _ = writeln!(output, "\nComparison signals");
    let signals = comparison_signals(&baseline_runs, &current_runs);
    if signals.is_empty() {
        let _ = writeln!(output, "- no threshold-level regression signal");
    } else {
        for signal in signals {
            let _ = writeln!(output, "- {signal}");
        }
    }
    Ok(output)
}

#[derive(Debug, Clone, Copy)]
struct ComparableRun {
    metrics: RunMetrics,
}

fn load_value(path: &Path) -> Result<Value, String> {
    let contents =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if let Ok(value) = serde_json::from_str::<Value>(&contents) {
        return Ok(value);
    }

    let mut runs = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|error| {
            format!(
                "parse {} line {} as JSON or JSONL: {error}",
                path.display(),
                line_index + 1
            )
        })?;
        runs.push(value);
    }
    Ok(json!({ "runs": runs }))
}

fn extract_runs(value: &Value) -> Result<BTreeMap<String, ComparableRun>, String> {
    let mut runs = BTreeMap::new();
    if let Some(values) = value.get("runs").and_then(Value::as_array) {
        for (index, run) in values.iter().enumerate() {
            if let Some(metrics) = extract_metrics(run) {
                let label = run_label(run).unwrap_or_else(|| format!("run-{index}"));
                runs.insert(label, ComparableRun { metrics });
            }
        }
        return Ok(runs);
    }

    if let Some(metrics) = extract_metrics(value) {
        let label = run_label(value).unwrap_or_else(|| "run".to_string());
        runs.insert(label, ComparableRun { metrics });
    }
    Ok(runs)
}

fn extract_metrics(value: &Value) -> Option<RunMetrics> {
    if let Some(metrics) = value.get("metrics") {
        return Some(RunMetrics {
            attempted: u64_field(metrics, "attempted")?,
            failed: u64_field(metrics, "failed").unwrap_or(0),
            throughput_ops_sec: f64_field(metrics, "throughput_ops_sec")?,
            cpu_ms: u128_field(metrics, "cpu_ms"),
            peak_rss_kb: u64_field(metrics, "peak_rss_kb"),
            p95_us: u128_field(metrics, "p95_us").unwrap_or(0),
            p99_us: u128_field(metrics, "p99_us").unwrap_or(0),
            max_us: u128_field(metrics, "max_us").unwrap_or(0),
        });
    }

    Some(RunMetrics {
        attempted: u64_field(value, "attempted")?,
        failed: u64_field(value, "failed").unwrap_or(0),
        throughput_ops_sec: f64_field(value, "throughput_ops_sec")?,
        cpu_ms: value.pointer("/process/delta_cpu_ms").and_then(u128_value),
        peak_rss_kb: value
            .pointer("/process/peak_rss_kb")
            .and_then(Value::as_u64),
        p95_us: value
            .pointer("/latency/p95_us")
            .and_then(u128_value)
            .or_else(|| value.get("worst_child_p99_us").and_then(u128_value))
            .unwrap_or(0),
        p99_us: value
            .pointer("/latency/p99_us")
            .and_then(u128_value)
            .or_else(|| value.get("worst_child_p99_us").and_then(u128_value))
            .unwrap_or(0),
        max_us: value
            .pointer("/latency/max_us")
            .and_then(u128_value)
            .or_else(|| value.get("worst_child_max_us").and_then(u128_value))
            .unwrap_or(0),
    })
}

fn run_label(value: &Value) -> Option<String> {
    if let Some(label) = value.get("label").and_then(Value::as_str) {
        return Some(label.to_string());
    }
    if let (Some(axis), Some(value)) = (
        value.get("axis").and_then(Value::as_str),
        value.get("value").and_then(Value::as_u64),
    ) {
        return Some(format!("{axis}={value}"));
    }
    value
        .get("run_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

#[derive(Debug, Clone, Copy)]
struct ComparisonPair {
    baseline: Option<ComparableRun>,
    current: Option<ComparableRun>,
}

#[derive(Debug)]
struct LabeledComparisonPair {
    label: String,
    baseline: Option<ComparableRun>,
    current: Option<ComparableRun>,
}

fn comparison_pairs(
    baseline_runs: &BTreeMap<String, ComparableRun>,
    current_runs: &BTreeMap<String, ComparableRun>,
) -> Vec<LabeledComparisonPair> {
    if baseline_runs.len() == 1 && current_runs.len() == 1 {
        let baseline_label = baseline_runs.keys().next().expect("baseline label");
        let current_label = current_runs.keys().next().expect("current label");
        let label = if baseline_label == current_label {
            baseline_label.clone()
        } else {
            format!("{baseline_label} -> {current_label}")
        };
        return vec![LabeledComparisonPair {
            label,
            baseline: baseline_runs.get(baseline_label).copied(),
            current: current_runs.get(current_label).copied(),
        }];
    }

    let mut pairs = BTreeMap::<String, ComparisonPair>::new();
    for (label, baseline) in baseline_runs {
        pairs
            .entry(label.clone())
            .or_insert(ComparisonPair {
                baseline: None,
                current: None,
            })
            .baseline = Some(*baseline);
    }
    for (label, current) in current_runs {
        pairs
            .entry(label.clone())
            .or_insert(ComparisonPair {
                baseline: None,
                current: None,
            })
            .current = Some(*current);
    }
    pairs
        .into_iter()
        .map(|(label, pair)| LabeledComparisonPair {
            label,
            baseline: pair.baseline,
            current: pair.current,
        })
        .collect()
}

fn push_metric_rows(
    output: &mut String,
    label: &str,
    baseline: ComparableRun,
    current: ComparableRun,
) {
    push_row(
        output,
        label,
        "throughput",
        format!("{:.2}", baseline.metrics.throughput_ops_sec),
        format!("{:.2}", current.metrics.throughput_ops_sec),
        percent_delta(
            current.metrics.throughput_ops_sec,
            baseline.metrics.throughput_ops_sec,
        ),
    );
    push_row(
        output,
        label,
        "p95",
        format_latency_us(baseline.metrics.p95_us),
        format_latency_us(current.metrics.p95_us),
        percent_delta_u128(current.metrics.p95_us, baseline.metrics.p95_us),
    );
    push_row(
        output,
        label,
        "fail_rate",
        format!("{:.2}%", failure_rate(baseline.metrics) * 100.0),
        format!("{:.2}%", failure_rate(current.metrics) * 100.0),
        percentage_point_delta(
            failure_rate(current.metrics),
            failure_rate(baseline.metrics),
        ),
    );
    if baseline.metrics.cpu_ms.is_some() || current.metrics.cpu_ms.is_some() {
        push_row(
            output,
            label,
            "cpu",
            format_optional_ms(baseline.metrics.cpu_ms),
            format_optional_ms(current.metrics.cpu_ms),
            option_percent_delta(current.metrics.cpu_ms, baseline.metrics.cpu_ms),
        );
    }
    if baseline.metrics.peak_rss_kb.is_some() || current.metrics.peak_rss_kb.is_some() {
        push_row(
            output,
            label,
            "rss",
            format_optional_kb(baseline.metrics.peak_rss_kb),
            format_optional_kb(current.metrics.peak_rss_kb),
            option_percent_delta_u64(current.metrics.peak_rss_kb, baseline.metrics.peak_rss_kb),
        );
    }
}

fn push_row(
    output: &mut String,
    label: &str,
    metric: &str,
    baseline: String,
    current: String,
    delta: String,
) {
    let _ = writeln!(
        output,
        "{:<22} {:<14} {:>12} {:>12} {:>12}",
        truncate(label, 22),
        metric,
        baseline,
        current,
        delta
    );
}

fn comparison_signals(
    baseline_runs: &BTreeMap<String, ComparableRun>,
    current_runs: &BTreeMap<String, ComparableRun>,
) -> Vec<String> {
    let mut signals = Vec::new();
    for pair in comparison_pairs(baseline_runs, current_runs) {
        let (Some(baseline), Some(current)) = (pair.baseline, pair.current) else {
            continue;
        };
        let throughput_delta = ratio_delta(
            current.metrics.throughput_ops_sec,
            baseline.metrics.throughput_ops_sec,
        );
        if let Some(delta) = throughput_delta
            && delta <= -0.20
        {
            signals.push(format!(
                "{} throughput regression {}",
                pair.label,
                percent_delta(
                    current.metrics.throughput_ops_sec,
                    baseline.metrics.throughput_ops_sec
                )
            ));
        }
        let p95_delta = ratio_delta_u128(current.metrics.p95_us, baseline.metrics.p95_us);
        if let Some(delta) = p95_delta
            && delta >= 0.20
        {
            signals.push(format!(
                "{} p95 regression {}",
                pair.label,
                percent_delta_u128(current.metrics.p95_us, baseline.metrics.p95_us)
            ));
        }
        let failure_delta = failure_rate(current.metrics) - failure_rate(baseline.metrics);
        if failure_delta >= 0.01 {
            signals.push(format!(
                "{} failure-rate regression {}",
                pair.label,
                percentage_point_delta(
                    failure_rate(current.metrics),
                    failure_rate(baseline.metrics)
                )
            ));
        }
        if let Some(delta) = option_ratio_delta(current.metrics.cpu_ms, baseline.metrics.cpu_ms)
            && delta >= 0.20
        {
            signals.push(format!(
                "{} CPU regression {}",
                pair.label,
                option_percent_delta(current.metrics.cpu_ms, baseline.metrics.cpu_ms)
            ));
        }
        if let Some(delta) =
            option_ratio_delta_u64(current.metrics.peak_rss_kb, baseline.metrics.peak_rss_kb)
            && delta >= 0.20
        {
            signals.push(format!(
                "{} RSS regression {}",
                pair.label,
                option_percent_delta_u64(current.metrics.peak_rss_kb, baseline.metrics.peak_rss_kb)
            ));
        }
    }
    signals
}

fn u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn u128_field(value: &Value, field: &str) -> Option<u128> {
    value.get(field).and_then(u128_value)
}

fn u128_value(value: &Value) -> Option<u128> {
    value.as_u64().map(u128::from)
}

fn f64_field(value: &Value, field: &str) -> Option<f64> {
    value.get(field).and_then(Value::as_f64)
}

fn failure_rate(metrics: RunMetrics) -> f64 {
    if metrics.attempted == 0 {
        0.0
    } else {
        metrics.failed as f64 / metrics.attempted as f64
    }
}

fn percent_delta(current: f64, baseline: f64) -> String {
    ratio_delta(current, baseline)
        .map(|delta| format!("{:+.1}%", delta * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

fn percent_delta_u128(current: u128, baseline: u128) -> String {
    ratio_delta_u128(current, baseline)
        .map(|delta| format!("{:+.1}%", delta * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

fn option_percent_delta(current: Option<u128>, baseline: Option<u128>) -> String {
    option_ratio_delta(current, baseline)
        .map(|delta| format!("{:+.1}%", delta * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

fn option_percent_delta_u64(current: Option<u64>, baseline: Option<u64>) -> String {
    option_ratio_delta_u64(current, baseline)
        .map(|delta| format!("{:+.1}%", delta * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

fn percentage_point_delta(current: f64, baseline: f64) -> String {
    format!("{:+.2}pp", (current - baseline) * 100.0)
}

fn ratio_delta(current: f64, baseline: f64) -> Option<f64> {
    if baseline == 0.0 {
        None
    } else {
        Some((current - baseline) / baseline)
    }
}

fn ratio_delta_u128(current: u128, baseline: u128) -> Option<f64> {
    if baseline == 0 {
        None
    } else {
        Some((current as f64 - baseline as f64) / baseline as f64)
    }
}

fn option_ratio_delta(current: Option<u128>, baseline: Option<u128>) -> Option<f64> {
    ratio_delta_u128(current?, baseline?)
}

fn option_ratio_delta_u64(current: Option<u64>, baseline: Option<u64>) -> Option<f64> {
    ratio_delta_u128(u128::from(current?), u128::from(baseline?))
}

fn format_latency_us(us: u128) -> String {
    if us >= 1_000_000 {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    } else if us >= 1_000 {
        format!("{:.1}ms", us as f64 / 1_000.0)
    } else {
        format!("{us}us")
    }
}

fn format_duration_ms(ms: u128) -> String {
    if ms >= 1_000 {
        format!("{:.2}s", ms as f64 / 1_000.0)
    } else {
        format!("{ms}ms")
    }
}

fn format_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.2}GB", kb as f64 / 1024.0 / 1024.0)
    } else if kb >= 1024 {
        format!("{:.1}MB", kb as f64 / 1024.0)
    } else {
        format!("{kb}KB")
    }
}

fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(format_duration_ms)
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_kb(value: Option<u64>) -> String {
    value.map(format_kb).unwrap_or_else(|| "-".to_string())
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}
