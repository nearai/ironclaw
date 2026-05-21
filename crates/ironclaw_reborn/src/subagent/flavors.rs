use crate::directions::DirectionId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentFlavorId {
    General,
    Researcher,
}

impl SubagentFlavorId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Researcher => "researcher",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentBudget {
    pub iteration_limit: u32,
    pub max_total_tokens: Option<u64>,
    pub max_cost_micro_usd: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentFlavor {
    pub id: SubagentFlavorId,
    pub direction: DirectionId,
    pub tool_allowlist: &'static [&'static str],
    pub model: &'static str,
    pub budget: SubagentBudget,
    pub allow_nesting: bool,
}

const GENERAL_TOOLS: &[&str] = &["message", "read_file", "list_files", "search"];
const RESEARCHER_TOOLS: &[&str] = &["message", "read_file", "list_files", "search", "web_search"];

pub const BUILTIN_SUBAGENT_FLAVORS: &[SubagentFlavor] = &[
    SubagentFlavor {
        id: SubagentFlavorId::General,
        direction: DirectionId::General,
        tool_allowlist: GENERAL_TOOLS,
        model: "default",
        budget: SubagentBudget {
            iteration_limit: 16,
            max_total_tokens: Some(200_000),
            max_cost_micro_usd: Some(500_000),
        },
        allow_nesting: false,
    },
    SubagentFlavor {
        id: SubagentFlavorId::Researcher,
        direction: DirectionId::Researcher,
        tool_allowlist: RESEARCHER_TOOLS,
        model: "default",
        budget: SubagentBudget {
            iteration_limit: 12,
            max_total_tokens: Some(150_000),
            max_cost_micro_usd: Some(300_000),
        },
        allow_nesting: false,
    },
];

pub fn lookup_flavor(id: SubagentFlavorId) -> Option<&'static SubagentFlavor> {
    BUILTIN_SUBAGENT_FLAVORS
        .iter()
        .find(|flavor| flavor.id == id)
}

#[cfg(test)]
mod tests {
    use crate::directions::direction_prompt;

    use super::*;

    #[test]
    fn builtin_table_has_general_and_researcher() {
        assert_eq!(BUILTIN_SUBAGENT_FLAVORS.len(), 2);
        assert!(lookup_flavor(SubagentFlavorId::General).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Researcher).is_some());
    }

    #[test]
    fn every_flavor_direction_resolves() {
        for flavor in BUILTIN_SUBAGENT_FLAVORS {
            assert!(!direction_prompt(flavor.direction).trim().is_empty());
        }
    }

    #[test]
    fn v1_flavors_disallow_nesting() {
        assert!(
            BUILTIN_SUBAGENT_FLAVORS
                .iter()
                .all(|flavor| !flavor.allow_nesting)
        );
    }

    #[test]
    fn flavor_tool_allowlists_exclude_spawn_subagent() {
        assert!(
            BUILTIN_SUBAGENT_FLAVORS
                .iter()
                .flat_map(|flavor| flavor.tool_allowlist.iter())
                .all(|tool| *tool != "spawn_subagent")
        );
    }
}
