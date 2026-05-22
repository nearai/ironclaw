use crate::subagent::directions::DirectionId;
use async_trait::async_trait;
use ironclaw_loop_support::{SubagentFlavorPolicy, SubagentFlavorPolicyResolver};
use ironclaw_turns::{RunProfileRequest, TurnRunId, run_profile::AgentLoopHostError};
use serde::{Deserialize, Serialize};

use crate::planned_driver_factory::SUBAGENT_PLANNED_PROFILE_ID;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentToolId {
    Message,
    ReadFile,
    ListFiles,
    Search,
    WebSearch,
}

impl SubagentToolId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::ReadFile => "read_file",
            Self::ListFiles => "list_files",
            Self::Search => "search",
            Self::WebSearch => "web_search",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentFlavor {
    pub id: SubagentFlavorId,
    pub direction: DirectionId,
    pub tool_allowlist: &'static [SubagentToolId],
    pub allow_nesting: bool,
}

const GENERAL_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::Message,
    SubagentToolId::ReadFile,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
];
const RESEARCHER_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::Message,
    SubagentToolId::ReadFile,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
    SubagentToolId::WebSearch,
];

pub const BUILTIN_SUBAGENT_FLAVORS: &[SubagentFlavor] = &[
    SubagentFlavor {
        id: SubagentFlavorId::General,
        direction: DirectionId::General,
        tool_allowlist: GENERAL_TOOLS,
        allow_nesting: false,
    },
    SubagentFlavor {
        id: SubagentFlavorId::Researcher,
        direction: DirectionId::Researcher,
        tool_allowlist: RESEARCHER_TOOLS,
        allow_nesting: false,
    },
];

pub fn lookup_flavor(id: SubagentFlavorId) -> Option<&'static SubagentFlavor> {
    BUILTIN_SUBAGENT_FLAVORS
        .iter()
        .find(|flavor| flavor.id == id)
}

#[derive(Default)]
pub struct StaticSubagentFlavorPolicyResolver;

#[async_trait]
impl SubagentFlavorPolicyResolver for StaticSubagentFlavorPolicyResolver {
    async fn resolve_flavor(
        &self,
        flavor_id: &str,
    ) -> Result<Option<SubagentFlavorPolicy>, AgentLoopHostError> {
        let Some(id) = parse_flavor_id(flavor_id) else {
            return Ok(None);
        };
        let Some(flavor) = lookup_flavor(id) else {
            return Ok(None);
        };
        Ok(Some(SubagentFlavorPolicy {
            flavor_id: flavor.id.as_str().to_string(),
            allow_nesting: flavor.allow_nesting,
            requested_run_profile: RunProfileRequest::new(SUBAGENT_PLANNED_PROFILE_ID).map_err(
                |reason| {
                    AgentLoopHostError::new(
                        ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
                        reason,
                    )
                },
            )?,
        }))
    }

    async fn flavor_of_run(
        &self,
        _run_id: TurnRunId,
    ) -> Result<Option<SubagentFlavorPolicy>, AgentLoopHostError> {
        Ok(None)
    }
}

pub fn parse_flavor_id(value: &str) -> Option<SubagentFlavorId> {
    match value {
        "general" => Some(SubagentFlavorId::General),
        "researcher" => Some(SubagentFlavorId::Researcher),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::subagent::directions::direction_prompt;
    use ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID;

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
                .all(|tool| tool.as_str() != DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
        );
    }

    #[tokio::test]
    async fn static_policy_resolver_binds_subagent_profile() {
        let resolver = StaticSubagentFlavorPolicyResolver;
        let policy = resolver
            .resolve_flavor("researcher")
            .await
            .unwrap()
            .expect("researcher flavor");

        assert_eq!(policy.flavor_id, "researcher");
        assert_eq!(
            policy.requested_run_profile.as_str(),
            SUBAGENT_PLANNED_PROFILE_ID
        );
        assert!(!policy.allow_nesting);
    }
}
