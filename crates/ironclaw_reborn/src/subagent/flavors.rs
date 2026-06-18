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
    Explorer,
    Coder,
    Planner,
}

impl SubagentFlavorId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Explorer => "explorer",
            Self::Coder => "coder",
            Self::Planner => "planner",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentToolId {
    ReadFile,
    WriteFile,
    ApplyPatch,
    Shell,
    ListFiles,
    Search,
    Glob,
    Http,
}

impl SubagentToolId {
    /// Capability id string registered in the host runtime first-party
    /// registry. Must remain a valid `CapabilityId`
    /// (`<extension>.<capability>` form) that resolves to a capability the
    /// host runtime actually registers — never advertise an unresolvable id,
    /// or a flavor's declared allowlist would diverge from its effective
    /// runtime capability surface.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadFile => "builtin.read_file",
            Self::WriteFile => "builtin.write_file",
            Self::ApplyPatch => "builtin.apply_patch",
            Self::Shell => "builtin.shell",
            Self::ListFiles => "builtin.list_dir",
            Self::Search => "builtin.grep",
            Self::Glob => "builtin.glob",
            Self::Http => "builtin.http",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentFlavor {
    pub id: SubagentFlavorId,
    pub direction: DirectionId,
    pub tool_allowlist: &'static [SubagentToolId],
    pub allow_nesting: bool,
    pub summary: &'static str,
}

// Subagent result delivery is out-of-band: the completion observer reads the
// child's final assistant thread message and hands it back to the parent. It is
// NOT a capability the subagent invokes, so there is no `builtin.message`
// capability in the first-party registry and it must not appear in any
// allowlist. See `subagent::completion_observer`.
const GENERAL_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::ReadFile,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
];
const EXPLORER_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::ReadFile,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
    SubagentToolId::Glob,
];
const CODER_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::ReadFile,
    SubagentToolId::WriteFile,
    SubagentToolId::ApplyPatch,
    SubagentToolId::Shell,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
    SubagentToolId::Glob,
];
const PLANNER_TOOLS: &[SubagentToolId] = &[
    SubagentToolId::ReadFile,
    SubagentToolId::ListFiles,
    SubagentToolId::Search,
    SubagentToolId::Glob,
    SubagentToolId::Http,
];

pub const BUILTIN_SUBAGENT_FLAVORS: &[SubagentFlavor] = &[
    SubagentFlavor {
        id: SubagentFlavorId::General,
        direction: DirectionId::General,
        tool_allowlist: GENERAL_TOOLS,
        allow_nesting: false,
        summary: "read-only file exploration (read_file, list_dir, grep)",
    },
    SubagentFlavor {
        id: SubagentFlavorId::Explorer,
        direction: DirectionId::Explorer,
        tool_allowlist: EXPLORER_TOOLS,
        allow_nesting: false,
        summary: "read + glob over filesystem (read_file, list_dir, grep, glob)",
    },
    SubagentFlavor {
        id: SubagentFlavorId::Coder,
        direction: DirectionId::Coder,
        tool_allowlist: CODER_TOOLS,
        allow_nesting: false,
        summary: "read + write + shell (read_file, write_file, apply_patch, shell, list_dir, grep, glob)",
    },
    SubagentFlavor {
        id: SubagentFlavorId::Planner,
        direction: DirectionId::Planner,
        tool_allowlist: PLANNER_TOOLS,
        allow_nesting: false,
        summary: "read codebase + web research, returns a structured implementation plan (read_file, list_dir, grep, glob, http)",
    },
];

pub fn lookup_flavor(id: SubagentFlavorId) -> Option<&'static SubagentFlavor> {
    BUILTIN_SUBAGENT_FLAVORS
        .iter()
        .find(|flavor| flavor.id == id)
}

/// Returns one [`ironclaw_loop_support::SpawnSubagentFlavorDescriptor`] per
/// entry in [`BUILTIN_SUBAGENT_FLAVORS`], in registry order. Derived directly
/// from the registry — single source of truth, no drift risk.
pub fn builtin_flavor_catalog() -> Vec<ironclaw_loop_support::SpawnSubagentFlavorDescriptor> {
    BUILTIN_SUBAGENT_FLAVORS
        .iter()
        .map(|f| ironclaw_loop_support::SpawnSubagentFlavorDescriptor {
            id: ironclaw_loop_support::SubagentKindId::new(f.id.as_str())
                .expect("valid SubagentKindId"), // safety: BUILTIN_SUBAGENT_FLAVORS ids are compile-time-constant valid SubagentKindId values
            summary: f.summary.to_string(),
        })
        .collect()
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
        "explorer" => Some(SubagentFlavorId::Explorer),
        "coder" => Some(SubagentFlavorId::Coder),
        "planner" => Some(SubagentFlavorId::Planner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::subagent::directions::direction_prompt;
    use ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID;

    use super::*;

    #[test]
    fn builtin_flavor_catalog_summaries_are_non_empty() {
        for descriptor in builtin_flavor_catalog() {
            assert!(
                !descriptor.summary.trim().is_empty(),
                "flavor `{}` has empty summary — schema description would render as a trailing-blank bullet",
                descriptor.id
            );
        }
    }

    #[test]
    fn builtin_table_has_expected_flavors() {
        assert_eq!(BUILTIN_SUBAGENT_FLAVORS.len(), 4);
        assert!(lookup_flavor(SubagentFlavorId::General).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Explorer).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Coder).is_some());
        assert!(lookup_flavor(SubagentFlavorId::Planner).is_some());
    }

    #[test]
    fn parse_flavor_id_planner_returns_some() {
        assert_eq!(parse_flavor_id("planner"), Some(SubagentFlavorId::Planner));
    }

    #[test]
    fn parse_flavor_id_researcher_returns_none() {
        assert_eq!(parse_flavor_id("researcher"), None);
    }

    #[test]
    fn planner_allowlist_contains_exactly_expected_tools() {
        let flavor = lookup_flavor(SubagentFlavorId::Planner).expect("planner flavor");
        let ids: Vec<&str> = flavor.tool_allowlist.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "builtin.read_file",
                "builtin.list_dir",
                "builtin.grep",
                "builtin.glob",
                "builtin.http",
            ]
        );
    }

    #[test]
    fn builtin_flavor_catalog_returns_four_entries_in_registry_order() {
        let catalog = builtin_flavor_catalog();
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0].id.as_str(), "general");
        assert_eq!(catalog[1].id.as_str(), "explorer");
        assert_eq!(catalog[2].id.as_str(), "coder");
        assert_eq!(catalog[3].id.as_str(), "planner");
    }

    #[test]
    fn explorer_flavor_is_read_only() {
        let flavor = lookup_flavor(SubagentFlavorId::Explorer).expect("explorer flavor");
        let ids: Vec<&str> = flavor.tool_allowlist.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            ids,
            vec![
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
        assert_eq!(parse_flavor_id("unknown"), None);
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
            .resolve_kind(&SubagentKindId::new("planner").unwrap())
            .await
            .unwrap()
            .expect("planner flavor");

        assert_eq!(policy.subagent_kind.as_str(), "planner");
        assert_eq!(
            policy.requested_run_profile.as_str(),
            SUBAGENT_PLANNED_PROFILE_ID
        );
        assert!(!policy.allow_nesting);
    }

    #[tokio::test]
    async fn static_policy_resolver_returns_none_for_researcher() {
        let resolver = StaticSubagentDefinitionResolver;
        let result = resolver
            .resolve_kind(&SubagentKindId::new("researcher").unwrap())
            .await
            .unwrap();
        assert!(result.is_none(), "researcher must no longer resolve");
    }

    // ---------------------------------------------------------------------
    // Caller-level attenuation tests.
    //
    // These drive the SAME production path a real subagent spawn uses:
    //   flavor.tool_allowlist
    //     -> RebornSubagentPromptMaterialSource::material_for_run (production)
    //     -> SubagentCapabilitySurfaceResolver (production)
    //     -> CapabilitySurfaceProfileFilter (the real runtime LoopCapabilityPort
    //        wrapper that enforces visibility + invocation denial).
    //
    // They assert the *effective* runtime surface, not just that the static
    // allowlist strings round-trip into CapabilityId. The string round-trip
    // test (`every_flavor_capability_surface_equals_allowlist`) is kept for
    // data consistency; this is the "test through the caller" coverage.
    // ---------------------------------------------------------------------
    mod caller_level {
        use std::sync::{Arc, Mutex};

        use async_trait::async_trait;
        use ironclaw_agent_loop::test_support::test_run_context;
        use ironclaw_host_api::{CapabilityId, RuntimeKind};
        use ironclaw_loop_support::{
            CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileFilter,
            CapabilitySurfaceProfileResolver, SubagentPromptMaterialSource,
        };
        use ironclaw_turns::run_profile::{
            AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome,
            CapabilityDescriptorView, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
            CapabilityResultMessage, CapabilitySurfaceVersion, ConcurrencyHint, LoopCapabilityPort,
            LoopDriverId, ProviderToolDefinition, VisibleCapabilityRequest,
            VisibleCapabilitySurface,
        };
        use ironclaw_turns::{LoopResultRef, RunProfileId, RunProfileVersion};

        use crate::planned_driver_factory::{
            PLANNED_DRIVER_DEFAULT_VERSION, SUBAGENT_PLANNED_DRIVER_ID, SUBAGENT_PLANNED_PROFILE_ID,
        };
        use crate::subagent::capability_surface::SubagentCapabilitySurfaceResolver;
        use crate::subagent::flavors::{SubagentFlavorId, lookup_flavor};
        use crate::subagent::goal_store::{
            InMemoryBoundedSubagentGoalStore, SubagentGoal, SubagentGoalStore,
        };
        use crate::subagent::prompt_material::RebornSubagentPromptMaterialSource;

        // Full first-party builtin capability surface the host registry exposes.
        // The flavor allowlist must be a subset of these; the filter narrows the
        // host surface down to the flavor's declared allowlist.
        const HOST_SURFACE: &[&str] = &[
            "builtin.read_file",
            "builtin.write_file",
            "builtin.apply_patch",
            "builtin.shell",
            "builtin.list_dir",
            "builtin.grep",
            "builtin.glob",
            "builtin.http",
            "builtin.spawn_subagent",
        ];

        fn cap(value: &str) -> CapabilityId {
            CapabilityId::new(value).expect("valid capability id")
        }

        /// Inner runtime capability port standing in for the host surface. The
        /// production `CapabilitySurfaceProfileFilter` wraps a port like this and
        /// narrows it to the per-flavor allowlist.
        #[derive(Default)]
        struct HostSurfaceSpy {
            invoked: Mutex<Vec<String>>,
        }

        struct StaticProfileResolver(CapabilityAllowSet);

        #[async_trait]
        impl CapabilitySurfaceProfileResolver for StaticProfileResolver {
            async fn resolve(
                &self,
                _run_context: &ironclaw_turns::run_profile::LoopRunContext,
            ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
                Ok(self.0.clone())
            }
        }

        #[async_trait]
        impl LoopCapabilityPort for HostSurfaceSpy {
            fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
                Ok(HOST_SURFACE
                    .iter()
                    .map(|id| ProviderToolDefinition {
                        capability_id: cap(id),
                        name: id.replace('.', "__"),
                        description: format!("{id} description"),
                        parameters: serde_json::json!({"type":"object"}),
                    })
                    .collect())
            }

            async fn visible_capabilities(
                &self,
                _request: VisibleCapabilityRequest,
            ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
                Ok(VisibleCapabilitySurface {
                    version: CapabilitySurfaceVersion::new("surface-v1")
                        .expect("valid surface version"),
                    descriptors: HOST_SURFACE
                        .iter()
                        .map(|id| CapabilityDescriptorView {
                            capability_id: cap(id),
                            provider: None,
                            runtime: RuntimeKind::FirstParty,
                            safe_name: id.to_string(),
                            safe_description: format!("{id} description"),
                            concurrency_hint: ConcurrencyHint::SafeForParallel,
                            parameters_schema: serde_json::json!({"type":"object"}),
                        })
                        .collect(),
                })
            }

            async fn invoke_capability(
                &self,
                request: CapabilityInvocation,
            ) -> Result<CapabilityOutcome, AgentLoopHostError> {
                self.invoked
                    .lock()
                    .expect("invoked lock")
                    .push(request.capability_id.as_str().to_string());
                Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:ok").expect("valid result ref"),
                    safe_summary: "ok".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: false,
                    byte_len: 0,
                    output_digest: None,
                }))
            }

            async fn invoke_capability_batch(
                &self,
                _request: CapabilityBatchInvocation,
            ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
                Ok(CapabilityBatchOutcome {
                    outcomes: Vec::new(),
                    stopped_on_suspension: false,
                })
            }
        }

        /// Build the production `CapabilitySurfaceProfileFilter` for a flavor by
        /// driving the real material source + surface resolver, exactly as a
        /// subagent spawn does, then return both the filter and the inner spy so
        /// callers can assert which capabilities actually reached the host.
        async fn filter_for_flavor(
            flavor: SubagentFlavorId,
        ) -> (
            ironclaw_loop_support::CapabilitySurfaceProfileFilter,
            Arc<HostSurfaceSpy>,
        ) {
            filter_for_flavor_with_base(flavor, CapabilityAllowSet::All).await
        }

        async fn filter_for_flavor_with_base(
            flavor: SubagentFlavorId,
            base_allow_set: CapabilityAllowSet,
        ) -> (
            ironclaw_loop_support::CapabilitySurfaceProfileFilter,
            Arc<HostSurfaceSpy>,
        ) {
            let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
            let mut context = test_run_context("caller-level-attenuation");
            context.resolved_run_profile.profile_id =
                RunProfileId::new(SUBAGENT_PLANNED_PROFILE_ID).expect("subagent profile id");
            context.resolved_run_profile.loop_driver.id =
                LoopDriverId::new(SUBAGENT_PLANNED_DRIVER_ID).expect("subagent driver id");
            context.resolved_run_profile.loop_driver.version =
                RunProfileVersion::new(PLANNED_DRIVER_DEFAULT_VERSION);
            goal_store
                .put_goal(
                    &context.scope,
                    context.run_id,
                    SubagentGoal {
                        task: "task".to_string(),
                        handoff: None,
                    },
                )
                .await
                .expect("seed goal");
            let source: Arc<dyn SubagentPromptMaterialSource> =
                Arc::new(RebornSubagentPromptMaterialSource::new(goal_store, flavor));
            let resolver = SubagentCapabilitySurfaceResolver::new(
                Arc::new(StaticProfileResolver(base_allow_set)),
                source,
            );
            let spy = Arc::new(HostSurfaceSpy::default());
            let allow_set = resolver
                .resolve(&context)
                .await
                .expect("production resolver builds the subagent allowlist");
            let filter = CapabilitySurfaceProfileFilter::new(spy.clone(), Arc::new(allow_set));
            (filter, spy)
        }

        fn invocation(capability: &str) -> CapabilityInvocation {
            CapabilityInvocation {
                surface_version: CapabilitySurfaceVersion::new("surface-v1")
                    .expect("valid surface version"),
                capability_id: cap(capability),
                input_ref: CapabilityInputRef::new("input:test").expect("valid input ref"),
                approval_resume: None,
                auth_resume: None,
            }
        }

        fn is_denied(outcome: &CapabilityOutcome) -> bool {
            matches!(outcome, CapabilityOutcome::Denied(_))
        }

        async fn visible_ids(
            filter: &ironclaw_loop_support::CapabilitySurfaceProfileFilter,
        ) -> Vec<String> {
            let mut ids = filter
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .expect("visible capabilities")
                .descriptors
                .into_iter()
                .map(|descriptor| descriptor.capability_id.as_str().to_string())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        }

        fn definition_ids(
            filter: &ironclaw_loop_support::CapabilitySurfaceProfileFilter,
        ) -> Vec<String> {
            let mut ids = filter
                .tool_definitions()
                .expect("tool definitions")
                .into_iter()
                .map(|definition| definition.capability_id.as_str().to_string())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        }

        fn expected_surface(flavor: SubagentFlavorId) -> Vec<String> {
            let mut ids = lookup_flavor(flavor)
                .expect("flavor")
                .tool_allowlist
                .iter()
                .map(|tool| tool.as_str().to_string())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        }

        #[tokio::test]
        async fn explorer_cannot_see_or_invoke_write_shell_or_spawn() {
            let (filter, spy) = filter_for_flavor(SubagentFlavorId::Explorer).await;

            // Visible surface and provider tool definitions equal the read-only
            // allowlist exactly — write/shell/spawn never appear.
            let expected = expected_surface(SubagentFlavorId::Explorer);
            assert_eq!(visible_ids(&filter).await, expected);
            assert_eq!(definition_ids(&filter), expected);
            for forbidden in [
                "builtin.write_file",
                "builtin.apply_patch",
                "builtin.shell",
                "builtin.spawn_subagent",
            ] {
                assert!(
                    !expected.contains(&forbidden.to_string()),
                    "explorer surface must not list {forbidden}"
                );
            }

            // Invocation of forbidden capabilities is denied and never reaches
            // the inner host port.
            for forbidden in [
                "builtin.write_file",
                "builtin.apply_patch",
                "builtin.shell",
                "builtin.spawn_subagent",
            ] {
                let outcome = filter
                    .invoke_capability(invocation(forbidden))
                    .await
                    .expect("outcome");
                assert!(
                    is_denied(&outcome),
                    "{forbidden} must be denied for explorer"
                );
            }

            // A permitted read capability passes through to the host.
            let outcome = filter
                .invoke_capability(invocation("builtin.read_file"))
                .await
                .expect("outcome");
            assert!(
                !is_denied(&outcome),
                "read_file must be allowed for explorer"
            );

            let invoked = spy.invoked.lock().expect("invoked lock").clone();
            assert_eq!(invoked, vec!["builtin.read_file".to_string()]);
        }

        #[tokio::test]
        async fn coder_gets_exactly_read_write_shell_surface_without_spawn() {
            let (filter, spy) = filter_for_flavor(SubagentFlavorId::Coder).await;

            // Effective surface equals the coder allowlist exactly — read, write,
            // apply_patch, shell, list_dir, grep, glob — and nothing more.
            let expected = expected_surface(SubagentFlavorId::Coder);
            assert_eq!(visible_ids(&filter).await, expected);
            assert_eq!(definition_ids(&filter), expected);
            // Crucially still NOT spawn_subagent.
            assert!(!expected.contains(&"builtin.spawn_subagent".to_string()));
            assert!(!expected.contains(&"builtin.http".to_string()));

            // Every declared capability invokes through to the host.
            for allowed in &expected {
                let outcome = filter
                    .invoke_capability(invocation(allowed))
                    .await
                    .expect("outcome");
                assert!(!is_denied(&outcome), "{allowed} must be allowed for coder");
            }

            // spawn_subagent is denied even though the host registry exposes it.
            let outcome = filter
                .invoke_capability(invocation("builtin.spawn_subagent"))
                .await
                .expect("outcome");
            assert!(
                is_denied(&outcome),
                "spawn_subagent must be denied for coder"
            );

            let invoked = spy.invoked.lock().expect("invoked lock").clone();
            assert_eq!(invoked, expected, "only allowlisted caps reach the host");
        }

        #[tokio::test]
        async fn subagent_surface_intersects_outer_profile_surface() {
            let (filter, spy) = filter_for_flavor_with_base(
                SubagentFlavorId::Coder,
                CapabilityAllowSet::allowlist([
                    cap("builtin.read_file"),
                    cap("builtin.shell"),
                    cap("builtin.http"),
                ]),
            )
            .await;

            let expected = vec!["builtin.read_file".to_string(), "builtin.shell".to_string()];
            assert_eq!(visible_ids(&filter).await, expected);
            assert_eq!(definition_ids(&filter), expected);

            for denied in ["builtin.apply_patch", "builtin.http"] {
                let outcome = filter
                    .invoke_capability(invocation(denied))
                    .await
                    .expect("outcome");
                assert!(
                    is_denied(&outcome),
                    "{denied} must be denied outside the intersected surface"
                );
            }

            let outcome = filter
                .invoke_capability(invocation("builtin.shell"))
                .await
                .expect("outcome");
            assert!(!is_denied(&outcome), "shell must remain allowed");
            assert_eq!(
                spy.invoked.lock().expect("invoked lock").clone(),
                vec!["builtin.shell".to_string()]
            );
        }

        #[tokio::test]
        async fn non_subagent_surface_preserves_outer_profile_surface() {
            let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
            let context = test_run_context("caller-level-non-subagent");
            let base_allow_set =
                CapabilityAllowSet::allowlist([cap("builtin.read_file"), cap("builtin.http")]);
            let source: Arc<dyn SubagentPromptMaterialSource> = Arc::new(
                RebornSubagentPromptMaterialSource::new(goal_store, SubagentFlavorId::Coder),
            );
            let resolver = SubagentCapabilitySurfaceResolver::new(
                Arc::new(StaticProfileResolver(base_allow_set.clone())),
                source,
            );

            let resolved = resolver
                .resolve(&context)
                .await
                .expect("non-subagent should preserve the base capability surface");

            assert_eq!(resolved, base_allow_set);
        }

        #[tokio::test]
        async fn general_and_planner_surfaces_have_no_regression() {
            for flavor in [SubagentFlavorId::General, SubagentFlavorId::Planner] {
                let (filter, _spy) = filter_for_flavor(flavor).await;
                let expected = expected_surface(flavor);
                assert_eq!(
                    visible_ids(&filter).await,
                    expected,
                    "{flavor:?} visible surface"
                );
                assert_eq!(
                    definition_ids(&filter),
                    expected,
                    "{flavor:?} tool definitions"
                );
                // These flavors must never expose write/shell/spawn.
                for forbidden in [
                    "builtin.write_file",
                    "builtin.shell",
                    "builtin.spawn_subagent",
                ] {
                    let outcome = filter
                        .invoke_capability(invocation(forbidden))
                        .await
                        .expect("outcome");
                    assert!(
                        is_denied(&outcome),
                        "{forbidden} must be denied for {flavor:?}"
                    );
                }
            }
        }

        #[tokio::test]
        async fn no_flavor_advertises_unresolvable_message_capability() {
            // Regression for the `builtin.message` finding: a flavor's declared
            // allowlist must equal its effective runtime surface, so no flavor
            // may advertise a capability id the host registry does not expose.
            for flavor in [
                SubagentFlavorId::General,
                SubagentFlavorId::Explorer,
                SubagentFlavorId::Coder,
                SubagentFlavorId::Planner,
            ] {
                let (filter, _spy) = filter_for_flavor(flavor).await;
                let host: std::collections::BTreeSet<String> =
                    HOST_SURFACE.iter().map(|id| id.to_string()).collect();
                for id in expected_surface(flavor) {
                    assert!(
                        host.contains(&id),
                        "{flavor:?} advertises {id} which the host runtime does not register"
                    );
                }
                // And `builtin.message` specifically must be absent + denied.
                assert!(!definition_ids(&filter).contains(&"builtin.message".to_string()));
                let outcome = filter
                    .invoke_capability(invocation("builtin.message"))
                    .await
                    .expect("outcome");
                assert!(
                    is_denied(&outcome),
                    "builtin.message must be denied for {flavor:?}"
                );
            }
        }
    }
}
