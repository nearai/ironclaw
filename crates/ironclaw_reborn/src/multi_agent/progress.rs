use super::runtime::aggregate_job_summary;
use super::types::MultiAgentRunReport;

pub fn format_job_progress_report(report: &MultiAgentRunReport) -> String {
    let mut output = String::new();
    output.push_str("Master task:\n");
    output.push_str(&report.master_task);
    output.push('\n');
    output.push_str("\nJob tree:\n");
    append_job_tree(&mut output, report, None, 0);
    output.push_str("\nProgress log:\n");
    append_progress_log(&mut output, report);
    output.push_str("\nFinal aggregated result:\n");
    output.push_str(&report.final_summary);
    output.push('\n');
    output.push_str(&format!(
        "\nJob totals: {}\n",
        aggregate_job_summary(&report.jobs)
    ));
    output
}

fn append_job_tree(
    output: &mut String,
    report: &MultiAgentRunReport,
    parent_id: Option<&str>,
    indent: usize,
) {
    let prefix = "  ".repeat(indent);
    let branch = if indent == 0 { "└── " } else { "├── " };
    for job in report
        .jobs
        .iter()
        .filter(|job| job.parent_id.as_deref() == parent_id)
    {
        output.push_str(&format!(
            "{prefix}{branch}[{}] {} status={:?}\n",
            job.agent_kind.as_str(),
            job.task,
            job.status
        ));
        if let Some(result) = &job.result {
            output.push_str(&format!("{prefix}    result: {result}\n"));
        }
        if let Some(error) = &job.error {
            output.push_str(&format!("{prefix}    error: {error}\n"));
        }
        append_job_tree(output, report, Some(&job.id), indent + 1);
    }
}

fn append_progress_log(output: &mut String, report: &MultiAgentRunReport) {
    let mut events = report.events.clone();
    events.sort_by_key(|event| event.timestamp);
    for event in events {
        output.push_str(&format!(
            "  [{}] {} job={} status={:?}: {}\n",
            event.timestamp.format("%H:%M:%S"),
            event.agent_kind.as_str(),
            event.job_id,
            event.status,
            event.message
        ));
    }
}

pub fn format_run_output(report: &MultiAgentRunReport, show_progress: bool) -> String {
    if show_progress {
        format_job_progress_report(report)
    } else {
        super::report::format_run_report(
            &report.master_task,
            &report.root_result,
            &report.final_summary,
        )
    }
}

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
        assert!(rendered.contains("Job tree:"));
        assert!(rendered.contains("Progress log:"));
        assert!(rendered.contains("master"));
    }
}
