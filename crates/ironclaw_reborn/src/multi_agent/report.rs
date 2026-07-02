use super::types::TaskResult;

pub fn format_run_report(master_task: &str, root_result: &TaskResult, final_summary: &str) -> String {
    let mut output = String::new();
    output.push_str("Master task:\n");
    output.push_str(master_task);
    output.push('\n');
    output.push_str("\nDelegation tree:\n");
    append_tree(&mut output, root_result, 0);
    output.push_str("\nAgent work:\n");
    append_agent_work(&mut output, root_result);
    output.push_str("\nFinal aggregated result:\n");
    output.push_str(final_summary);
    output.push('\n');
    output
}

fn append_tree(output: &mut String, result: &TaskResult, indent: usize) {
    let prefix = "  ".repeat(indent);
    output.push_str(&format!(
        "{prefix}[{}] {} ({:?})\n",
        result.agent_id, result.summary, result.status
    ));
    for child in &result.child_results {
        append_tree(output, child, indent + 1);
    }
}

fn append_agent_work(output: &mut String, result: &TaskResult) {
    output.push_str(&format!(
        "  {}: {} ({:?})\n",
        result.agent_id, result.summary, result.status
    ));
    if let Some(error) = &result.error {
        output.push_str(&format!("    error: {error}\n"));
    }
    for child in &result.child_results {
        append_agent_work(output, child);
    }
}

pub fn aggregate_final_summary(root_result: &TaskResult) -> String {
    let mut completed = 0usize;
    let mut failed = 0usize;
    collect_counts(root_result, &mut completed, &mut failed);
    format!(
        "Master completed with {completed} successful node(s) and {failed} failed node(s). Root summary: {}",
        root_result.summary
    )
}

fn collect_counts(result: &TaskResult, completed: &mut usize, failed: &mut usize) {
    match result.status {
        super::types::TaskStatus::Completed | super::types::TaskStatus::Delegated => {
            *completed = completed.saturating_add(1);
        }
        super::types::TaskStatus::Failed => {
            *failed = failed.saturating_add(1);
        }
    }
    for child in &result.child_results {
        collect_counts(child, completed, failed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi_agent::types::{TaskId, TaskStatus};

    #[test]
    fn format_includes_master_and_final_summary() {
        let root = TaskResult::delegated(
            TaskId::new("root"),
            "master",
            "Delegated 2 subtask(s): 2 completed, 0 failed",
            vec![
                TaskResult::completed(TaskId::new("a"), "sub-a", "done a"),
                TaskResult::completed(TaskId::new("b"), "sub-b", "done b"),
            ],
        );
        let report = format_run_report(
            "research and summarize",
            &root,
            "final",
        );
        assert!(report.contains("Master task:"));
        assert!(report.contains("Delegation tree:"));
        assert!(report.contains("[master]"));
        assert!(report.contains("Final aggregated result:"));
        assert!(report.contains("final"));
    }

    #[test]
    fn aggregate_counts_failed_nodes() {
        let root = TaskResult::delegated(
            TaskId::new("root"),
            "master",
            "Delegated",
            vec![TaskResult::failed(
                TaskId::new("bad"),
                "sub-bad",
                "boom",
            )],
        );
        let summary = aggregate_final_summary(&root);
        assert!(summary.contains("failed node(s)"));
        assert!(root.child_results[0].status == TaskStatus::Failed);
    }
}
