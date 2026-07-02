use std::collections::HashMap;
use std::io::IsTerminal;

use super::job_model::{AgentEvent, AgentJob, AgentKind, AgentStatus};
use super::runtime::aggregate_job_summary;
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
    fn yellow(&self, s: &str) -> String {
        self.ansi(s, "33")
    }
    fn cyan(&self, s: &str) -> String {
        self.ansi(s, "36")
    }
    fn magenta(&self, s: &str) -> String {
        self.ansi(s, "35")
    }

    fn ansi(&self, s: &str, code: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

// ─── layout helpers ───────────────────────────────────────────────────────────

const WIDTH: usize = 70;

fn rule(title: &str, c: &Palette) -> String {
    let padded = format!("  {title}  ");
    let fill = WIDTH.saturating_sub(padded.len());
    let left = fill / 2;
    let right = fill - left;
    c.dim(&format!("{}{padded}{}", "─".repeat(left), "─".repeat(right)))
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

fn fmt_ms(ms: i64) -> String {
    if ms < 0 {
        "—".to_string()
    } else if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.2}s", ms as f64 / 1_000.0)
    }
}

fn status_glyph(status: AgentStatus, c: &Palette) -> String {
    match status {
        AgentStatus::Complete => c.green("✔"),
        AgentStatus::Failed | AgentStatus::Cancelled => c.red("✗"),
        AgentStatus::Running | AgentStatus::Claimed => c.yellow("◐"),
        AgentStatus::WaitingForChildren => c.cyan("⌛"),
        AgentStatus::Pending => c.dim("○"),
    }
}

fn kind_tag(kind: AgentKind, c: &Palette) -> String {
    match kind {
        AgentKind::Master => c.magenta("master"),
        AgentKind::SubAgent => c.cyan("agent"),
    }
}

// ─── peak-parallelism from event stream ───────────────────────────────────────

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

// ─── job-tree renderer ────────────────────────────────────────────────────────

fn render_job_tree(
    buf: &mut String,
    by_parent: &HashMap<Option<String>, Vec<&AgentJob>>,
    parent_id: Option<&str>,
    indent: &str,
    c: &Palette,
) {
    let key: Option<String> = parent_id.map(str::to_string);
    let children = match by_parent.get(&key) {
        Some(v) => v,
        None => return,
    };

    for (i, job) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let branch = if is_last { "└─" } else { "├─" };
        let cont = if is_last { "   " } else { "│  " };

        let icon = status_glyph(job.status, c);
        let kind = kind_tag(job.agent_kind, c);
        let short_id = &job.id[..job.id.len().min(8)];
        let elapsed = fmt_ms((job.updated_at - job.created_at).num_milliseconds());

        // Main job line
        buf.push_str(&format!(
            "  {indent}{branch} {icon} {kind}  {}  {}\n",
            c.dim(&format!("[{short_id}]")),
            c.dim(&elapsed),
        ));

        // Task description
        let task = trunc(&job.task, 58);
        buf.push_str(&format!(
            "  {indent}{cont}   {} {}\n",
            c.dim("task:"),
            task,
        ));

        // Result (if any)
        if let Some(result) = &job.result {
            let rshort = trunc(result, 56);
            buf.push_str(&format!(
                "  {indent}{cont}   {} {}\n",
                c.dim("result:"),
                c.green(&rshort),
            ));
        }

        // Error (if any)
        if let Some(error) = &job.error {
            let eshort = trunc(error, 56);
            buf.push_str(&format!(
                "  {indent}{cont}   {} {}\n",
                c.dim("error:"),
                c.red(&eshort),
            ));
        }

        // Show depth/child count badges on the right
        let n_children = by_parent
            .get(&Some(job.id.clone()))
            .map(|v| v.len())
            .unwrap_or(0);
        if n_children > 0 {
            buf.push_str(&format!(
                "  {indent}{cont}   {} delegated {} AgentRun(s)\n",
                c.dim("→"),
                n_children,
            ));
        }

        // Recurse into children
        let child_indent = format!("{indent}{cont}");
        render_job_tree(buf, by_parent, Some(&job.id), &child_indent, c);

        if !is_last {
            buf.push_str(&format!("  {indent}│\n"));
        }
    }
}

// ─── progress event log ───────────────────────────────────────────────────────

fn render_progress_log(buf: &mut String, report: &MultiAgentRunReport, c: &Palette) {
    if report.events.is_empty() {
        buf.push_str(&format!("  {}\n", c.dim("no events recorded")));
        return;
    }

    let origin = report
        .jobs
        .iter()
        .find(|j| j.id == report.root_id)
        .map(|j| j.created_at)
        .unwrap_or_else(|| {
            report
                .events
                .iter()
                .map(|e| e.timestamp)
                .min()
                .unwrap_or_default()
        });

    let mut sorted = report.events.clone();
    sorted.sort_by_key(|e| e.timestamp);

    for ev in &sorted {
        let rel = (ev.timestamp - origin).num_milliseconds();
        let short_id = &ev.job_id[..ev.job_id.len().min(8)];

        let status_str = match ev.status {
            AgentStatus::Running => c.yellow("running"),
            AgentStatus::Complete => c.green("complete"),
            AgentStatus::Failed => c.red("failed"),
            AgentStatus::Cancelled => c.red("cancelled"),
            AgentStatus::WaitingForChildren => c.cyan("waiting"),
            AgentStatus::Claimed => c.yellow("claimed"),
            AgentStatus::Pending => c.dim("pending"),
        };

        buf.push_str(&format!(
            "  {}  {}  {}  {}  {}\n",
            c.dim(&format!("{:>+7}ms", rel)),
            kind_tag(ev.agent_kind, c),
            c.dim(&format!("[{short_id}]")),
            status_str,
            c.dim(&trunc(&ev.message, 48)),
        ));
    }
}

// ─── public API ──────────────────────────────────────────────────────────────

/// Full production-quality dashboard for a completed multi-agent run.
///
/// Always renders:  header · execution plan · runtime stats · agent-run tree
/// · final result.  With `show_progress = true`, also renders the timestamped
/// event log between the stats and the agent tree sections.
pub fn format_run_output(report: &MultiAgentRunReport, show_progress: bool) -> String {
    let c = Palette::detect();
    let mut buf = String::new();

    // ── Header ────────────────────────────────────────────────────────────────
    buf.push('\n');
    buf.push_str(&format!(
        "  {}\n",
        c.bold("IronClaw  ·  Multi-Agent Runtime")
    ));
    buf.push_str(&format!("  {}\n", c.dim(&"═".repeat(WIDTH - 2))));
    buf.push('\n');

    // ── Root task ─────────────────────────────────────────────────────────────
    let task_display = trunc(&report.master_task, 64);
    buf.push_str(&format!(
        "  {}  {}\n",
        c.bold("Root task"),
        c.cyan(&format!("\"{}\"", task_display)),
    ));

    let root_job = report.jobs.iter().find(|j| j.id == report.root_id);
    if let Some(root) = root_job {
        buf.push_str(&format!(
            "  {}    depth ≤ {}  ·  retries {}\n",
            c.dim("Config"),
            root.max_depth,
            root.max_retries,
        ));
    }
    buf.push('\n');

    // ── Execution Plan ────────────────────────────────────────────────────────
    buf.push_str(&format!("{}\n\n", rule("Execution Plan", &c)));

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

    let n_masters = report
        .jobs
        .iter()
        .filter(|j| j.agent_kind == AgentKind::Master)
        .count();
    let n_agents = report
        .jobs
        .iter()
        .filter(|j| j.agent_kind == AgentKind::SubAgent)
        .count();

    if n_agents == 0 {
        buf.push_str(&format!(
            "  {}  master ran the task locally (no delegation needed)\n",
            c.dim("○"),
        ));
    } else {
        buf.push_str(&format!(
            "  {} {} master ·  {} delegated AgentRun(s)\n",
            c.dim("tree:"),
            n_masters,
            n_agents,
        ));
    }
    buf.push('\n');

    render_job_tree(&mut buf, &by_parent, None, "", &c);
    buf.push('\n');

    // ── Runtime Stats ─────────────────────────────────────────────────────────
    buf.push_str(&format!("{}\n\n", rule("Runtime", &c)));

    let completed = report
        .jobs
        .iter()
        .filter(|j| j.status == AgentStatus::Complete)
        .count();
    let failed = report
        .jobs
        .iter()
        .filter(|j| {
            j.status == AgentStatus::Failed || j.status == AgentStatus::Cancelled
        })
        .count();
    let peak = peak_parallelism(&report.events);
    let max_depth_reached = report.jobs.iter().map(|j| j.depth).max().unwrap_or(0);
    let wall_ms = root_job
        .map(|j| (j.updated_at - j.created_at).num_milliseconds())
        .unwrap_or(0);

    buf.push_str(&format!(
        "  {:12}  {}   ({n_masters} master · {n_agents} delegated)\n",
        c.bold("Jobs"),
        c.bold(&report.jobs.len().to_string()),
    ));
    buf.push_str(&format!(
        "  {:12}  {}  {}  ({})\n",
        c.bold("Results"),
        c.green(&format!("{completed} ✔")),
        if failed > 0 {
            c.red(&format!("{failed} ✗"))
        } else {
            c.dim("0 ✗")
        },
        c.dim(&aggregate_job_summary(&report.jobs)),
    ));
    buf.push_str(&format!(
        "  {:12}  peak {} concurrent  ·  max depth reached {}\n",
        c.bold("Parallelism"),
        c.bold(&peak.to_string()),
        c.bold(&max_depth_reached.to_string()),
    ));
    buf.push_str(&format!(
        "  {:12}  {}\n",
        c.bold("Wall time"),
        c.bold(&fmt_ms(wall_ms)),
    ));
    buf.push('\n');

    // ── Progress Log (opt-in) ─────────────────────────────────────────────────
    if show_progress {
        buf.push_str(&format!("{}\n\n", rule("Progress Log", &c)));
        render_progress_log(&mut buf, report, &c);
        buf.push('\n');
    }

    // ── Agent Answers ─────────────────────────────────────────────────────────
    // Collect every job that produced output, sorted depth-first so the master
    // comes first and sub-agents follow in creation order.
    let mut answered: Vec<&AgentJob> = report
        .jobs
        .iter()
        .filter(|j| j.result.is_some() || j.error.is_some())
        .collect();
    answered.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.created_at.cmp(&b.created_at)));

    if !answered.is_empty() {
        buf.push_str(&format!("{}\n\n", rule("Agent Answers", &c)));
        for (idx, job) in answered.iter().enumerate() {
            let short_id = &job.id[..job.id.len().min(8)];
            let label = match job.agent_kind {
                AgentKind::Master => c.magenta(&format!("MasterAgent [{}]", short_id)),
                AgentKind::SubAgent => c.cyan(&format!("AgentRun    [{}]", short_id)),
            };
            let depth_tag = if job.depth > 0 {
                c.dim(&format!("  (depth {})", job.depth))
            } else {
                String::new()
            };

            buf.push_str(&format!("  {} {}{}\n", c.bold("▸"), label, depth_tag));
            buf.push_str(&format!(
                "  {}  {}\n",
                c.dim("task:"),
                c.dim(&job.task),
            ));

            if let Some(result) = &job.result {
                // Wrap long results at ~66 chars so they're readable in any terminal
                let wrapped = word_wrap(result, 66);
                for line in wrapped.lines() {
                    buf.push_str(&format!("  {}  {}\n", c.dim("    "), line));
                }
            }
            if let Some(error) = &job.error {
                buf.push_str(&format!(
                    "  {}  {}\n",
                    c.dim("error:"),
                    c.red(error),
                ));
            }

            if idx < answered.len() - 1 {
                buf.push_str(&format!("  {}\n", c.dim(&"·".repeat(WIDTH - 4))));
            }
            buf.push('\n');
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────────
    buf.push_str(&format!("{}\n\n", rule("Summary", &c)));

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

    buf.push_str(&format!(
        "  {} AgentRun(s) completed  ·  {} failed  ·  wall time {}\n",
        c.green(&completed.to_string()),
        if failed > 0 {
            c.red(&failed.to_string())
        } else {
            c.dim("0")
        },
        c.bold(&fmt_ms(wall_ms)),
    ));
    buf.push('\n');

    buf
}

/// Wrap `text` at `max_cols` characters, preserving existing newlines.
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

/// Legacy verbose format — retained for tests and tooling that import it
/// directly.  New code should prefer [`format_run_output`].
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
    async fn progress_report_includes_tree_and_events() {
        let report = run_multi_agent_jobs(
            "Research X; Plan implementation; Verify result",
            MultiAgentRunConfig::new(3, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("progress report run");
        let rendered = format_job_progress_report(&report);
        assert!(rendered.contains("Root task"));
        assert!(rendered.contains("Execution Plan"));
        assert!(rendered.contains("Runtime"));
        assert!(rendered.contains("Final Result"));
    }

    #[tokio::test]
    async fn format_run_output_shows_parallelism() {
        let report = run_multi_agent_jobs(
            "task one; task two; task three",
            MultiAgentRunConfig::new(2, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("parallel run");
        let rendered = format_run_output(&report, false);
        assert!(rendered.contains("Parallelism"));
        assert!(rendered.contains("Wall time"));
    }

    #[tokio::test]
    async fn single_task_shows_no_delegation_message() {
        let report = run_multi_agent_jobs(
            "just one short task",
            MultiAgentRunConfig::new(1, 10, Duration::from_secs(10), 0),
        )
        .await
        .expect("single task run");
        let rendered = format_run_output(&report, false);
        assert!(rendered.contains("Execution Plan"));
        assert!(rendered.contains("Final Result"));
    }
}
