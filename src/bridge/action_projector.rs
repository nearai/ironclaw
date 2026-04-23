use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::debug;

use ironclaw_engine::{
    ActionDef, ActionDiscoveryMetadata, ActionDiscoverySummary, CapabilityLease,
    CapabilityRegistry, CapabilityStatus, EngineError, ThreadExecutionContext,
};

use crate::auth::extension::AuthManager;
use crate::bridge::capability_projector::{
    capability_status_for_extension, capability_surface_subject_for_extension,
};
use crate::bridge::tool_surface::{
    InvocationMode, SurfacePolicyInput, SurfaceSubjectKind, assign_surface,
};
use crate::extensions::InstalledExtension;
use crate::extensions::naming::extension_name_candidates;
use crate::tools::ToolRegistry;

pub(crate) struct ActionProjector;

impl ActionProjector {
    /// Project the set of available actions from the tool registry and
    /// capability registry.
    ///
    /// When `prefetched_extensions` is `Some`, the projector uses that map
    /// instead of fetching from `auth_manager`. This allows the caller
    /// (typically `EffectBridgeAdapter`) to share a single fetch across
    /// both `ActionProjector` and `CapabilityProjector`.
    pub(crate) async fn project(
        tools: &ToolRegistry,
        auth_manager: Option<&AuthManager>,
        capability_registry: Option<Arc<CapabilityRegistry>>,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
        prefetched_extensions: Option<&HashMap<String, InstalledExtension>>,
    ) -> Result<Vec<ActionDef>, EngineError> {
        let tool_defs = tools.all().await;
        let owned_statuses;
        let extension_statuses: Option<&HashMap<String, InstalledExtension>> = if let Some(
            prefetched,
        ) =
            prefetched_extensions
        {
            Some(prefetched)
        } else if let Some(auth_manager) = auth_manager {
            match auth_manager
                .list_capability_extensions(&context.user_id)
                .await
            {
                Ok(extensions) => {
                    owned_statuses = extensions
                        .into_iter()
                        .map(|extension| (extension.name.clone(), extension))
                        .collect::<HashMap<_, _>>();
                    Some(&owned_statuses)
                }
                Err(error) => {
                    debug!(
                        user_id = %context.user_id,
                        error = %error,
                        "failed to load extension inventory for available_actions; omitting extension-backed actions"
                    );
                    owned_statuses = HashMap::new();
                    Some(&owned_statuses)
                }
            }
        } else {
            None
        };

        let mut actions = Vec::with_capacity(tool_defs.len());
        for tool in tool_defs {
            if crate::bridge::effect_adapter::is_v1_only_tool(tool.name()) {
                continue;
            }
            if crate::bridge::effect_adapter::is_v1_auth_tool(tool.name()) {
                continue;
            }

            if let Some(provider_extension) = tool.provider_extension() {
                let Some(extension_statuses) = extension_statuses else {
                    continue;
                };
                let Some(extension) =
                    provider_extension_status(extension_statuses, provider_extension)
                else {
                    continue;
                };
                let status = capability_status_for_extension(extension, false);
                let (kind, invocation_mode) = capability_surface_subject_for_extension(extension);
                let assignment = assign_surface(SurfacePolicyInput {
                    kind,
                    status,
                    invocation_mode,
                    leased_and_callable: false,
                });
                if !assignment.available_actions {
                    continue;
                }
            }

            actions.push(project_tool_action(tool.as_ref()));
        }

        if let Some(registry) = capability_registry.as_ref() {
            let mut seen: HashSet<String> = actions.iter().map(|a| a.name.clone()).collect();
            for lease in leases {
                if lease.capability_name == "tools" {
                    continue;
                }
                let Some(cap) = registry.get(&lease.capability_name) else {
                    continue;
                };
                for action in &cap.actions {
                    if !lease.granted_actions.covers(&action.name) {
                        continue;
                    }
                    if crate::bridge::effect_adapter::is_v1_only_tool(&action.name)
                        || crate::bridge::effect_adapter::is_v1_auth_tool(&action.name)
                    {
                        continue;
                    }
                    let assignment = assign_surface(SurfacePolicyInput {
                        kind: SurfaceSubjectKind::EngineNativeDirectAction,
                        status: CapabilityStatus::Ready,
                        invocation_mode: InvocationMode::Direct,
                        leased_and_callable: true,
                    });
                    if !assignment.available_actions || !seen.insert(action.name.clone()) {
                        continue;
                    }
                    actions.push(action.clone());
                }
            }
        }

        actions.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(actions)
    }
}

fn project_tool_action(tool: &dyn crate::tools::Tool) -> ActionDef {
    let callable_name = tool.name().replace('-', "_");
    let callable_schema = tool.parameters_schema();
    let discovery_schema = tool.discovery_schema();
    let summary = tool
        .discovery_summary()
        .map(|summary| ActionDiscoverySummary {
            always_required: summary.always_required,
            conditional_requirements: summary.conditional_requirements,
            notes: summary.notes,
            examples: summary.examples,
        });
    let schema_override = (discovery_schema != callable_schema).then_some(discovery_schema);

    ActionDef {
        name: callable_name.clone(),
        description: tool.description().to_string(),
        parameters_schema: callable_schema,
        effects: vec![],
        requires_approval: false,
        discovery: Some(ActionDiscoveryMetadata {
            name: callable_name,
            summary,
            schema_override,
        }),
    }
}

fn provider_extension_status<'a>(
    extension_statuses: &'a HashMap<String, InstalledExtension>,
    provider_extension: &str,
) -> Option<&'a InstalledExtension> {
    extension_name_candidates(provider_extension)
        .into_iter()
        .filter_map(|candidate| extension_statuses.get(&candidate))
        .max_by_key(|extension| provider_extension_rank(extension))
}

fn provider_extension_rank(extension: &InstalledExtension) -> u8 {
    match capability_status_for_extension(extension, false) {
        CapabilityStatus::Ready => 5,
        CapabilityStatus::Inactive => 4,
        CapabilityStatus::NeedsAuth => 3,
        CapabilityStatus::NeedsSetup => 2,
        CapabilityStatus::Error => 1,
        CapabilityStatus::AvailableNotInstalled => 0,
        CapabilityStatus::ReadyScoped | CapabilityStatus::Latent => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use ironclaw_engine::ThreadExecutionContext;

    use super::{ActionProjector, project_tool_action, provider_extension_status};
    use crate::extensions::{ExtensionKind, InstalledExtension};
    use crate::tools::ToolRegistry;

    fn installed_extension(name: &str) -> InstalledExtension {
        InstalledExtension {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            display_name: Some(name.to_string()),
            description: Some(format!("{name} description")),
            url: None,
            authenticated: true,
            active: true,
            tools: vec![format!("{name}_search")],
            needs_setup: false,
            has_auth: true,
            installed: true,
            activation_error: None,
            version: None,
        }
    }

    fn needs_auth_extension(name: &str) -> InstalledExtension {
        InstalledExtension {
            authenticated: false,
            ..installed_extension(name)
        }
    }

    fn needs_setup_extension(name: &str) -> InstalledExtension {
        InstalledExtension {
            needs_setup: true,
            ..installed_extension(name)
        }
    }

    fn inactive_extension(name: &str) -> InstalledExtension {
        InstalledExtension {
            active: false,
            ..installed_extension(name)
        }
    }

    fn channel_extension(name: &str) -> InstalledExtension {
        InstalledExtension {
            kind: ExtensionKind::WasmChannel,
            tools: Vec::new(),
            ..installed_extension(name)
        }
    }

    struct ProviderTool {
        name: &'static str,
        description: &'static str,
        provider_extension: &'static str,
    }

    #[async_trait]
    impl crate::tools::Tool for ProviderTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            self.description
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _: serde_json::Value,
            _: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            Ok(crate::tools::ToolOutput::success(
                serde_json::json!({}),
                std::time::Duration::from_millis(1),
            ))
        }

        fn provider_extension(&self) -> Option<&str> {
            Some(self.provider_extension)
        }
    }

    struct DiscoveryTool;

    #[async_trait]
    impl crate::tools::Tool for DiscoveryTool {
        fn name(&self) -> &str {
            "mission_helper"
        }

        fn description(&self) -> &str {
            "Mission helper"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"id": {"type": "string"}}})
        }

        fn discovery_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "mode": {"type": "string"}
                },
                "required": ["id"]
            })
        }

        fn discovery_summary(&self) -> Option<crate::tools::ToolDiscoverySummary> {
            Some(crate::tools::ToolDiscoverySummary {
                always_required: vec!["id".to_string()],
                conditional_requirements: vec!["mode is needed when updating".to_string()],
                notes: vec!["Use for mission inspection".to_string()],
                examples: vec![],
            })
        }

        async fn execute(
            &self,
            _: serde_json::Value,
            _: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            unreachable!("not needed")
        }
    }

    async fn projected_action_names(
        tool_name: &'static str,
        description: &'static str,
        provider_extension: &'static str,
        extension: InstalledExtension,
    ) -> Vec<String> {
        let tools = std::sync::Arc::new(ToolRegistry::new());
        tools
            .register(std::sync::Arc::new(ProviderTool {
                name: tool_name,
                description,
                provider_extension,
            }))
            .await;

        let extension_map = HashMap::from([(extension.name.clone(), extension)]);
        let actions = ActionProjector::project(
            tools.as_ref(),
            None,
            None,
            &[],
            &test_context(),
            Some(&extension_map),
        )
        .await
        .expect("project should succeed");

        actions.into_iter().map(|a| a.name).collect()
    }

    fn test_context() -> ThreadExecutionContext {
        ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
            thread_goal: None,
            available_actions_snapshot: None,
        }
    }

    #[test]
    fn provider_extension_lookup_accepts_legacy_hyphen_alias() {
        let extension = installed_extension("linear-server");
        let statuses = HashMap::from([(extension.name.clone(), extension)]);

        let resolved = provider_extension_status(&statuses, "linear_server")
            .expect("legacy hyphen alias should resolve");

        assert_eq!(resolved.name, "linear-server");
    }

    #[test]
    fn provider_extension_lookup_prefers_installed_alias_over_registry_only_entry() {
        let installed = installed_extension("linear-server");
        let registry_only = InstalledExtension {
            installed: false,
            active: false,
            authenticated: false,
            has_auth: true,
            tools: Vec::new(),
            ..installed_extension("linear_server")
        };
        let statuses = HashMap::from([
            (installed.name.clone(), installed),
            (registry_only.name.clone(), registry_only),
        ]);

        let resolved = provider_extension_status(&statuses, "linear_server")
            .expect("installed alias should win over registry-only canonical entry");

        assert_eq!(resolved.name, "linear-server");
        assert!(resolved.installed);
    }

    #[test]
    fn project_tool_action_preserves_discovery_metadata() {
        let tool = std::sync::Arc::new(DiscoveryTool);
        let action = project_tool_action(tool.as_ref());

        assert_eq!(action.description, "Mission helper");
        assert!(
            action.parameters_schema["properties"].get("mode").is_none(),
            "callable schema should stay on the executable surface only"
        );

        let discovery = action.discovery.expect("discovery metadata");
        assert_eq!(discovery.name, "mission_helper");
        assert!(discovery.summary.is_some());
        let schema_override = discovery
            .schema_override
            .expect("discovery schema override");
        assert!(schema_override["properties"].get("mode").is_some());
    }

    #[tokio::test]
    async fn needs_auth_provider_tools_omitted_from_available_actions() {
        let actions = projected_action_names(
            "gmail_send",
            "Send a Gmail message",
            "gmail",
            needs_auth_extension("gmail"),
        )
        .await;

        assert!(
            !actions.iter().any(|a| a == "gmail_send"),
            "NeedsAuth provider tool should be omitted from available_actions, got: {actions:?}"
        );
    }

    #[tokio::test]
    async fn needs_setup_provider_tools_omitted_from_available_actions() {
        let actions = projected_action_names(
            "notion_search",
            "Search Notion",
            "notion",
            needs_setup_extension("notion"),
        )
        .await;

        assert!(
            !actions.iter().any(|a| a == "notion_search"),
            "NeedsSetup provider tool should be omitted from available_actions, got: {actions:?}"
        );
    }

    #[tokio::test]
    async fn inactive_provider_tools_omitted_from_available_actions() {
        let actions = projected_action_names(
            "github_search",
            "Search GitHub",
            "github",
            inactive_extension("github"),
        )
        .await;

        assert!(
            !actions.iter().any(|a| a == "github_search"),
            "Inactive provider tool should be omitted from available_actions, got: {actions:?}"
        );
    }

    #[tokio::test]
    async fn routed_channel_tools_omitted_from_available_actions() {
        let actions = projected_action_names(
            "telegram_send",
            "Send a Telegram message",
            "telegram",
            channel_extension("telegram"),
        )
        .await;

        assert!(
            !actions.iter().any(|a| a == "telegram_send"),
            "Routed-only channel tool should be omitted from available_actions, got: {actions:?}"
        );
    }

    #[tokio::test]
    async fn latent_provider_tools_omitted_from_available_actions() {
        let actions = projected_action_names(
            "latent_send",
            "Send via latent provider",
            "latent_provider",
            InstalledExtension {
                installed: false,
                active: false,
                authenticated: false,
                ..installed_extension("latent_provider")
            },
        )
        .await;

        assert!(
            !actions.iter().any(|a| a == "latent_send"),
            "Not-installed provider tool should be omitted from available_actions, got: {actions:?}"
        );
    }
}
