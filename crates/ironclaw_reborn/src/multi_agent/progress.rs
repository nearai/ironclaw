use std::collections::HashMap;
use std::io::IsTerminal;

use super::job_model::{AgentEvent, AgentJob, AgentKind, AgentStatus};
use super::types::MultiAgentRunReport;

// ─── colour palette ───────────────────────────────────────────────────────────

struct Palette {
    on: bool,
}

impl Palette {
    fn detect() -> Self {
        let on = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none();
        Self { on }
    }

    fn bold(&self, s: &str) -> String {
        self.ansi(s, "1")
    }
    fn dim(&self, s: &str) -> String {
        self.ansi(s, "2")
    }
    fn green(&self, s: &str) -> String {
        self.ansi(s, "32")
    }
    fn red(&self, s: &str) -> String {
        self.ansi(s, "31")
    }
    fn cyan(&self, s: &str) -> String {
        self.ansi(s, "36")
    }

    fn ansi(&self, s: &str, code: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

const RULE_WIDTH: usize = 44;

fn section(title: &str, c: &Palette, buf: &mut String) {
    buf.push_str(&format!("\n{}\n", c.bold(title)));
    buf.push_str(&c.dim(&"─".repeat(RULE_WIDTH)));
    buf.push('\n');
}

fn fmt_ms(ms: i64) -> String {
    if ms < 0 {
        "—".to_string()
    } else if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1_000.0)
    }
}

fn status_mark(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Complete => "✓",
        AgentStatus::Failed | AgentStatus::Cancelled => "✗",
        _ => "…",
    }
}

fn status_colored(mark: &str, status: AgentStatus, c: &Palette) -> String {
    match status {
        AgentStatus::Complete => c.green(mark),
        AgentStatus::Failed | AgentStatus::Cancelled => c.red(mark),
        _ => c.dim(mark),
    }
}

/// Display name for a job in the execution tree:  "AgentRun-N" for sub-agents,
/// "MasterAgent" for the root.
fn display_name(job: &AgentJob) -> String {
    match job.agent_kind {
        AgentKind::Master => "MasterAgent".to_string(),
        AgentKind::SubAgent => {
            // ID is "job-N" — render as "AgentRun-N"
            let suffix = job
                .id
                .strip_prefix("job-")
                .unwrap_or(&job.id)
                .to_string();
            format!("AgentRun-{suffix}")
        }
    }
}

// ─── peak parallelism ────────────────────────────────────────────────────────

fn peak_parallelism(events: &[AgentEvent]) -> usize {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|e| e.timestamp);
    let mut active: std::collections::HashSet<String> = Default::default();
    let mut peak = 0usize;
    for ev in &sorted {
        match ev.status {
            AgentStatus::Running | AgentStatus::Claimed => {
                active.insert(ev.job_id.clone());
            }
            AgentStatus::Complete
            | AgentStatus::Failed
            | AgentStatus::Cancelled
            | AgentStatus::WaitingForChildren => {
                active.remove(&ev.job_id);
            }
            AgentStatus::Pending => {}
        }
        peak = peak.max(active.len());
    }
    peak.max(1)
}

// ─── execution plan tree ─────────────────────────────────────────────────────

fn render_execution_plan(
    buf: &mut String,
    report: &MultiAgentRunReport,
    c: &Palette,
) {
    let mut by_parent: HashMap<Option<String>, Vec<&AgentJob>> = HashMap::new();
    for job in &report.jobs {
        by_parent
            .entry(job.parent_id.clone())
            .or_default()
            .push(job);
    }
    for v in by_parent.values_mut() {
        v.sort_by_key(|j| j.created_at);
    }
    render_job_node(buf, &by_parent, None, "", c);
}

fn render_job_node(
    buf: &mut String,
    by_parent: &HashMap<Option<String>, Vec<&AgentJob>>,
    parent_id: Option<&str>,
    prefix: &str,
    c: &Palette,
) {
    let key: Option<String> = parent_id.map(str::to_string);
    let children = match by_parent.get(&key) {
        Some(v) => v,
        None => return,
    };
    let root_level = parent_id.is_none();

    for (i, job) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let branch = if root_level {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };

        let mark = status_mark(job.status);
        let colored_mark = status_colored(mark, job.status, c);
        let name = display_name(job);
        let task_short = truncate(&job.task, 50);
        let n_children = by_parent
            .get(&Some(job.id.clone()))
            .map(|v| v.len())
            .unwrap_or(0);

        // Decision annotation (what the planner decided for this job)
        let decision_tag = job
            .plan_decision
            .as_deref()
            .map(|d| format!("  {}", c.dim(&format!("({d})"))))
            .unwrap_or_default();

        buf.push_str(&format!(
            "{prefix}{branch}{} {colored_mark}{decision_tag}\n",
            c.bold(&name),
        ));

        // Show the task on the indent continuation line
        let cont = if root_level {
            "    "
        } else if is_last {
            "    "
        } else {
            "│   "
        };
        buf.push_str(&format!(
            "{prefix}{cont}{}  {}\n",
            c.dim("task:"),
            task_short,
        ));

        // If this job delegated, note how many concurrent tasks were spawned
        if n_children > 0 {
            buf.push_str(&format!(
                "{prefix}{cont}{}  {} concurrent runs  (depth {})\n",
                c.dim("↳"),
                1 + n_children, // local + delegated
                job.depth + 1,
            ));
        }

        // Recurse into children
        let child_prefix = format!(
            "{prefix}{}",
            if root_level {
                ""
            } else if is_last {
                "    "
            } else {
                "│   "
            }
        );
        render_job_node(buf, by_parent, Some(&job.id), &child_prefix, c);
    }
}

// ─── combined final answer ────────────────────────────────────────────────────

/// Build a clean combined answer from all job results. Master's local work
/// first, then each sub-agent in creation order, separated by blank lines.
fn combined_answer(report: &MultiAgentRunReport) -> String {
    let mut jobs: Vec<&AgentJob> = report
        .jobs
        .iter()
        .filter(|j| j.result.is_some())
        .collect();
    jobs.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.created_at.cmp(&b.created_at)));

    // For a single job (no delegation), just return its result directly.
    if jobs.len() == 1 {
        return jobs[0].result.clone().unwrap_or_default();
    }

    let mut parts: Vec<String> = Vec::new();
    for job in &jobs {
        if let Some(result) = &job.result {
            let name = display_name(job);
            let label = format!("[{name}]");
            let text = word_wrap(result, 66);
            parts.push(format!("{label}\n{text}"));
        }
    }
    parts.join("\n")
}

// ─── verbose: per-agent details ───────────────────────────────────────────────

fn render_verbose(buf: &mut String, report: &MultiAgentRunReport, c: &Palette) {
    let mut jobs: Vec<&AgentJob> = report
        .jobs
        .iter()
        .filter(|j| j.result.is_some() || j.error.is_some())
        .collect();
    jobs.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.created_at.cmp(&b.created_at)));

    for (idx, job) in jobs.iter().enumerate() {
        let elapsed = fmt_ms((job.updated_at - job.created_at).num_milliseconds());
        let name = display_name(job);
        let status_m = status_mark(job.status);
        let colored_m = status_colored(status_m, job.status, c);

        buf.push_str(&format!(
            "{}  {}  {}\n",
            c.bold(&name),
            colored_m,
            c.dim(&elapsed),
        ));
        buf.push_str(&format!(
            "  {}  {}\n",
            c.dim("Task:"),
            job.task,
        ));

        if let Some(result) = &job.result {
            let wrapped = word_wrap(result, 66);
            buf.push_str(&format!("  {}\n", c.dim("Output:")));
            for line in wrapped.lines() {
                buf.push_str(&format!("    {line}\n"));
            }
        }
        if let Some(error) = &job.error {
            buf.push_str(&format!("  {}  {}\n", c.dim("Error:"), c.red(error)));
        }

        if idx < jobs.len() - 1 {
            buf.push_str(&format!("\n  {}\n\n", c.dim(&"─".repeat(40))));
        }
    }

    // Progress event log
    if !report.events.is_empty() {
        buf.push_str(&format!("\n  {}\n", c.dim("Events:")));
        let origin = report
            .jobs
            .iter()
            .find(|j| j.id == report.root_id)
            .map(|j| j.created_at)
            .unwrap_or_default();
        let mut sorted = report.events.clone();
        sorted.sort_by_key(|e| e.timestamp);
        for ev in &sorted {
            let rel = (ev.timestamp - origin).num_milliseconds();
            let short_id = &ev.job_id[..ev.job_id.len().min(8)];
            let status_str = match ev.status {
                AgentStatus::Running => c.cyan("running"),
                AgentStatus::Complete => c.green("complete"),
                AgentStatus::Failed | AgentStatus::Cancelled => c.red("failed"),
                AgentStatus::WaitingForChildren => c.dim("waiting"),
                _ => c.dim("—"),
            };
            buf.push_str(&format!(
                "  {}  {}  {}  {}\n",
                c.dim(&format!("{:>+6}ms", rel)),
                c.dim(&format!("[{short_id}]")),
                status_str,
                c.dim(&truncate(&ev.message, 50)),
            ));
        }
    }
}

// ─── string utilities ────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

fn word_wrap(text: &str, max_cols: usize) -> String {
    let mut out = String::new();
    for para in text.split('\n') {
        if para.len() <= max_cols {
            out.push_str(para);
            out.push('\n');
            continue;
        }
        let mut line_len = 0usize;
        for word in para.split_whitespace() {
            if line_len == 0 {
                out.push_str(word);
                line_len = word.len();
            } else if line_len + 1 + word.len() > max_cols {
                out.push('\n');
                out.push_str(word);
                line_len = word.len();
            } else {
                out.push(' ');
                out.push_str(word);
                line_len += 1 + word.len();
            }
        }
        out.push('\n');
    }
    out
}

// ─── public API ──────────────────────────────────────────────────────────────

/// Standard output: root task, execution plan, runtime stats, combined
/// final result.
///
/// With `verbose = true`, also renders a detailed per-agent breakdown with
/// full outputs and the event log.
pub fn format_run_output(report: &MultiAgentRunReport, verbose: bool) -> String {
    let c = Palette::detect();
    let mut buf = String::new();

    buf.push_str(&format!("\n{}\n", c.bold("IronClaw Multi-Agent Runtime")));

    // ── Root Task ─────────────────────────────────────────────────────────────
    section("Root Task", &c, &mut buf);
    buf.push_str(&report.master_task);
    buf.push('\n');

    // ── Execution Plan ────────────────────────────────────────────────────────
    section("Execution Plan", &c, &mut buf);
    render_execution_plan(&mut buf, report, &c);

    // ── Runtime ───────────────────────────────────────────────────────────────
    section("Runtime", &c, &mut buf);

    let root_job = report.jobs.iter().find(|j| j.id == report.root_id);
    let wall_ms = root_job
        .map(|j| (j.updated_at - j.created_at).num_milliseconds())
        .unwrap_or(0);
    let peak = peak_parallelism(&report.events);
    let max_depth = report.jobs.iter().map(|j| j.depth).max().unwrap_or(0);
    let completed = report
        .jobs
        .iter()
        .filter(|j| j.status == AgentStatus::Complete)
        .count();
    let failed = report
        .jobs
        .iter()
        .filter(|j| j.status == AgentStatus::Failed || j.status == AgentStatus::Cancelled)
        .count();

    buf.push_str(&format!("Jobs:              {}\n", report.jobs.len()));
    buf.push_str(&format!("Peak Parallelism:  {peak}\n"));
    buf.push_str(&format!("Max Depth:         {max_depth}\n"));
    buf.push_str(&format!("Wall Time:         {}\n", fmt_ms(wall_ms)));
    if failed > 0 {
        buf.push_str(&format!(
            "Results:           {}  {}\n",
            c.green(&format!("{completed} ✓")),
            c.red(&format!("{failed} ✗")),
        ));
    } else {
        buf.push_str(&format!(
            "Results:           {} ✓\n",
            c.green(&completed.to_string()),
        ));
    }

    // ── Verbose: per-agent details ────────────────────────────────────────────
    if verbose {
        section("Agent Outputs", &c, &mut buf);
        render_verbose(&mut buf, report, &c);
    }

    // ── Final Result ──────────────────────────────────────────────────────────
    section("Final Result", &c, &mut buf);
    let answer = combined_answer(report);
    buf.push_str(&answer);

    if !verbose {
        buf.push_str(&format!(
            "\n{}\n",
            c.dim("Use --verbose to inspect every individual AgentRun output."),
        ));
    }

    buf.push('\n');
    buf
}

/// Retained for tests and tooling that import this name directly.
pub fn format_job_progress_report(report: &MultiAgentRunReport) -> String {
    format_run_output(report, true)
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi_agent::runtime::{MultiAgentRunConfig, run_multi_agent_jobs};
    use std::time::Duration;

    #[tokio::test]
    async fn standard_output_has_all_sections() {
        let report = run_multi_agent_jobs(
            "Research X; Plan implementation; Verify result",
            MultiAgentRunConfig::new(3, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("run");
        let out = format_run_output(&report, false);
        assert!(out.contains("Root Task"));
        assert!(out.contains("Execution Plan"));
        assert!(out.contains("MasterAgent"));
        assert!(out.contains("Runtime"));
        assert!(out.contains("Final Result"));
        assert!(out.contains("--verbose"));
    }

    #[tokio::test]
    async fn verbose_output_has_agent_outputs_section() {
        let report = run_multi_agent_jobs(
            "task one; task two; task three",
            MultiAgentRunConfig::new(2, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("run");
        let out = format_run_output(&report, true);
        assert!(out.contains("Agent Outputs"));
        assert!(!out.contains("--verbose"));
    }

    #[tokio::test]
    async fn execution_plan_shows_agentrun_names() {
        let report = run_multi_agent_jobs(
            "part one; part two",
            MultiAgentRunConfig::new(2, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("run");
        let out = format_run_output(&report, false);
        assert!(out.contains("AgentRun-"));
    }
}
