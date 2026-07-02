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
            return DelegationPlan::execute_directly();
        }

        let normalized = task.description.trim();
        if normalized.is_empty() {
            return DelegationPlan::execute_directly();
        }

        if let Some(subtasks) = split_on_delimiters(normalized) {
            if subtasks.len() > 1 {
                return DelegationPlan::delegate(subtasks);
            }
        }

        let words = normalized.split_whitespace().count();
        if words > 8 {
            let midpoint = words / 2;
            let mut split = normalized.split_whitespace();
            let first: Vec<_> = split.by_ref().take(midpoint).collect();
            let second: Vec<_> = split.collect();
            if !first.is_empty() && !second.is_empty() {
                return DelegationPlan::delegate(vec![
                    first.join(" "),
                    second.join(" "),
                ]);
            }
        }

        DelegationPlan::execute_directly()
    }
}

fn split_on_delimiters(description: &str) -> Option<Vec<String>> {
    let lower = description.to_ascii_lowercase();
    let delimiter = if lower.contains(';') {
        ';'
    } else if lower.contains(" and ") {
        return split_on_and(description);
    } else {
        return None;
    };

    let parts = description
        .split(delimiter)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        Some(parts)
    } else {
        None
    }
}

fn split_on_and(description: &str) -> Option<Vec<String>> {
    let parts = description
        .split(" and ")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        Some(parts)
    } else {
        None
    }
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
