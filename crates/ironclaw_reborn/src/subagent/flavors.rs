use crate::subagent::directions::DirectionId;
use async_trait::async_trait;
use ironclaw_loop_support::{SubagentDefinition, SubagentDefinitionResolver, SubagentKindId};
use ironclaw_turns::{RunProfileRequest, TurnRunId, run_profile::AgentLoopHostError};
use serde::{Deserialize, Serialize};

use crate::planned_driver_factory::SUBAGENT_PLANNED_PROFILE_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentFlavorId {
    General,
    Researcher,
    Explorer,
    Coder,
}

impl SubagentFlavorId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Researcher => "researcher",
            Self::Explorer => "explorer",
            Self::Coder => "coder",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentToolId {
    Message,
    ReadFile,
    WriteFile,
    ApplyPatch,
    Shell,
    ListFiles,
    Search,
    Glob,
    WebSearch,
}

impl SubagentToolId {
    /// Capability id string registered in the host runtime first-party
    /// registry. Must remain a valid `CapabilityId`
    /// (`<extension>.<capability>` form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "builtin.message",
            Self::ReadFile => "builtin.read_file",
            Self::WriteFile => "builtin.write_file",
            Self::ApplyPatch => "builtin.apply_patch",
            Self::Shell => "builtin.shell",
            Self::ListFiles => "builtin.list_dir",
            Self::Search => "builtin.grep",
            Self::Glob => "builtin.glob",
            Self::WebSearch => "builtin.http",
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
const EXPLORER_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::Message,
    SubagentToolId::ReadFile,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
    SubagentToolId::Glob,
];
const CODER_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::Message,
    SubagentToolId::ReadFile,
    SubagentToolId::WriteFile,
    SubagentToolId::ApplyPatch,
    SubagentToolId::Shell,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
    SubagentToolId::Glob,
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
    SubagentFlavor {
        id: SubagentFlavorId::Explorer,
        direction: DirectionId::Explorer,
        tool_allowlist: EXPLORER_TOOLS,
        allow_nesting: false,
    },
    SubagentFlavor {
        id: SubagentFlavorId::Coder,
        direction: DirectionId::Coder,
        tool_allowlist: CODER_TOOLS,
        allow_nesting: false,
    },
];

pub fn lookup_flavor(id: SubagentFlavorId) -> Option<&'static SubagentFlavor> {
    BUILTIN_SUBAGENT_FLAVORS
        .iter()
        .find(|flavor| flavor.id == id)
}

#[derive(Default)]
pub struct StaticSubagentDefinitionResolver;

#[async_trait]
impl SubagentDefinitionResolver for StaticSubagentDefinitionResolver {
    async fn resolve_kind(
        &self,
        kind: &SubagentKindId,
    ) -> Result<Option<SubagentDefinition>, AgentLoopHostError> {
        let Some(id) = parse_flavor_id(kind.as_str()) else {
            return Ok(None);
        };
        let Some(flavor) = lookup_flavor(id) else {
            return Ok(None);
        };
        Ok(Some(SubagentDefinition {
            subagent_kind: kind.clone(),
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

    async fn definition_of_run(
        &self,
        _run_id: TurnRunId,
    ) -> Result<Option<SubagentDefinition>, AgentLoopHostError> {
        Ok(None)
    }
}

pub fn parse_flavor_id(value: &str) -> Option<SubagentFlavorId> {
    match value {
        "general" => Some(SubagentFlavorId::General),
        "researcher" => Some(SubagentFlavorId::Researcher),
        "explorer" => Some(SubagentFlavorId::Explorer),
        "coder" => Some(SubagentFlavorId::Coder),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::subagent::directions::direction_prompt;
    use ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID;

    use super::*;

    #[test]
    fn builtin_table_has_expected_flavors() {
        assert_eq!(BUILTIN_SUBAGENT_FLAVORS.len(), 4);
        assert!(lookup_flavor(SubagentFlavorId::General).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Researcher).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Explorer).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Coder).is_some());
    }

    #[test]
    fn explorer_flavor_is_read_only() {
        let flavor = lookup_flavor(SubagentFlavorId::Explorer).expect("explorer flavor");
        let ids: Vec<&str> = flavor.tool_allowlist.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "builtin.message",
                "builtin.read_file",
                "builtin.list_dir",
                "builtin.grep",
                "builtin.glob",
            ]
        );
        // No write/shell/web surface for explorer.
        assert!(!ids.contains(&"builtin.write_file"));
        assert!(!ids.contains(&"builtin.apply_patch"));
        assert!(!ids.contains(&"builtin.shell"));
        assert!(!ids.contains(&"builtin.http"));
        assert!(!flavor.allow_nesting);
    }

    #[test]
    fn coder_flavor_surface_matches_allowlist_exactly() {
        let flavor = lookup_flavor(SubagentFlavorId::Coder).expect("coder flavor");
        let ids: Vec<&str> = flavor.tool_allowlist.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "builtin.message",
                "builtin.read_file",
                "builtin.write_file",
                "builtin.apply_patch",
                "builtin.shell",
                "builtin.list_dir",
                "builtin.grep",
                "builtin.glob",
            ]
        );
        assert!(!flavor.allow_nesting);
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

    #[test]
    fn parse_flavor_id_round_trips_all_flavors() {
        for flavor in BUILTIN_SUBAGENT_FLAVORS {
            assert_eq!(parse_flavor_id(flavor.id.as_str()), Some(flavor.id));
        }
        assert_eq!(
            parse_flavor_id("explorer"),
            Some(SubagentFlavorId::Explorer)
        );
        assert_eq!(parse_flavor_id("coder"), Some(SubagentFlavorId::Coder));
        assert_eq!(parse_flavor_id("nope"), None);
    }

    #[test]
    fn every_flavor_capability_surface_equals_allowlist() {
        use ironclaw_host_api::CapabilityId;
        use std::collections::BTreeSet;

        // Attenuation invariant: the capability surface derived for each flavor
        // is exactly the flavor's static allowlist — no leakage, no narrowing.
        for flavor in BUILTIN_SUBAGENT_FLAVORS {
            let expected: BTreeSet<String> = flavor
                .tool_allowlist
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            let resolved: BTreeSet<String> = flavor
                .tool_allowlist
                .iter()
                .map(|t| {
                    CapabilityId::new(t.as_str())
                        .expect("flavor capability id must be valid")
                        .as_str()
                        .to_string()
                })
                .collect();
            assert_eq!(
                resolved,
                expected,
                "flavor {} capability surface must match its allowlist exactly",
                flavor.id.as_str()
            );
        }
    }

    #[tokio::test]
    async fn static_policy_resolver_binds_subagent_profile() {
        let resolver = StaticSubagentDefinitionResolver;
        let policy = resolver
            .resolve_kind(&SubagentKindId::new("researcher").unwrap())
            .await
            .unwrap()
            .expect("researcher flavor");

        assert_eq!(policy.subagent_kind.as_str(), "researcher");
        assert_eq!(
            policy.requested_run_profile.as_str(),
            SUBAGENT_PLANNED_PROFILE_ID
        );
        assert!(!policy.allow_nesting);
    }
}
