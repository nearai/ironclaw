use super::types::{AgentContext, DelegationPlan, Task};

/// Placeholder planner that decides whether to execute or delegate.
///
/// Future LLM-backed planners can implement the same surface and be injected
/// into [`super::MasterAgent`].
pub trait DelegationPlanner: Send + Sync {
    fn plan(&self, task: &Task, ctx: &AgentContext) -> DelegationPlan;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicDelegationPlanner;

impl DelegationPlanner for HeuristicDelegationPlanner {
    fn plan(&self, task: &Task, ctx: &AgentContext) -> DelegationPlan {
        if !ctx.can_delegate(task) {
            return DelegationPlan::local(format!(
                "depth {} reached max_depth {}",
                task.depth, ctx.max_depth
            ));
        }

        let normalized = task.description.trim();
        if normalized.is_empty() {
            return DelegationPlan::local("empty task");
        }

        if let Some((subtasks, delim)) = split_on_delimiters_with_delim(normalized) {
            if subtasks.len() > 1 {
                return DelegationPlan::split(
                    subtasks.clone(),
                    format!(
                        "{} independent parts split on '{delim}'",
                        subtasks.len()
                    ),
                );
            }
        }

        let words = normalized.split_whitespace().count();
        if words > 8 {
            let midpoint = words / 2;
            let mut iter = normalized.split_whitespace();
            let first: Vec<_> = iter.by_ref().take(midpoint).collect();
            let second: Vec<_> = iter.collect();
            if !first.is_empty() && !second.is_empty() {
                return DelegationPlan::split(
                    vec![first.join(" "), second.join(" ")],
                    format!("long task ({words} words) halved"),
                );
            }
        }

        DelegationPlan::local(format!(
            "simple task ({} words, no independent parts found)",
            normalized.split_whitespace().count()
        ))
    }
}

fn split_on_delimiters_with_delim(description: &str) -> Option<(Vec<String>, &'static str)> {
    let lower = description.to_ascii_lowercase();
    if lower.contains(';') {
        let parts: Vec<String> = description
            .split(';')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        if parts.len() > 1 {
            return Some((parts, ";"));
        }
    }
    if lower.contains(" and ") {
        let parts: Vec<String> = description
            .split(" and ")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        if parts.len() > 1 {
            return Some((parts, "and"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi_agent::types::TaskId;

    #[test]
    fn planner_splits_on_semicolons() {
        let planner = HeuristicDelegationPlanner;
        let task = Task::root("research topic; summarize findings", TaskId::new("t1"));
        let ctx = AgentContext::new(3, 10, std::time::Duration::from_secs(30));
        let plan = planner.plan(&task, &ctx);
        assert!(!plan.execute_directly);
        assert_eq!(plan.subtasks.len(), 2);
    }
}
