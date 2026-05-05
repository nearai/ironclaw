use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::debug;

use ironclaw_engine::{
    ActionDef, ActionDiscoveryMetadata, ActionDiscoverySummary, ActionInventory, CapabilityLease,
    CapabilityRegistry, CapabilityStatus, EngineError, ModelToolSurface, ThreadExecutionContext,
};

use crate::auth::extension::AuthManager;
use crate::bridge::capability_projector::{
    capability_status_for_extension, capability_surface_subject_for_extension,
};
use crate::bridge::tool_permissions::ToolPermissionSnapshot;
use crate::bridge::tool_surface::{
    InvocationMode, SurfacePolicyInput, SurfaceSubjectKind, assign_surface,
};
use crate::extensions::naming::extension_name_candidates;
use crate::extensions::{InstalledExtension, LatentProviderAction};
use crate::tools::ToolRegistry;
use crate::tools::permissions::PermissionState;

pub(crate) struct ActionProjector;

struct InventoryInputs {
    tool_defs: Vec<Arc<dyn crate::tools::Tool>>,
    extension_statuses: Option<HashMap<String, InstalledExtension>>,
    latent_actions: Vec<LatentProviderAction>,
    tool_permissions: ToolPermissionSnapshot,
}

impl ActionProjector {
    /// Project the set of available actions from the tool registry and
    /// capability registry.
    ///
    /// When `prefetched_extensions` is `Some`, the projector uses that map
    /// instead of fetching from `auth_manager`. This allows the caller
    /// (typically `EffectBridgeAdapter`) to share a single fetch across
    /// both `ActionProjector` and `CapabilityProjector`.
    pub(crate) async fn project_inventory(
        tools: &ToolRegistry,
        auth_manager: Option<&AuthManager>,
        capability_registry: Option<Arc<CapabilityRegistry>>,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
        prefetched_extensions: Option<&HashMap<String, InstalledExtension>>,
    ) -> Result<ActionInventory, EngineError> {
        let inputs =
            load_inventory_inputs(tools, auth_manager, context, prefetched_extensions).await;
        Ok(classify_projected_actions(
            inputs,
            capability_registry.as_ref(),
            leases,
        ))
    }
}

async fn load_inventory_inputs(
    tools: &ToolRegistry,
    auth_manager: Option<&AuthManager>,
    context: &ThreadExecutionContext,
    prefetched_extensions: Option<&HashMap<String, InstalledExtension>>,
) -> InventoryInputs {
    let tool_defs = tools.all().await;
    let extension_statuses = if let Some(prefetched) = prefetched_extensions {
        Some(prefetched.clone())
    } else if let Some(auth_manager) = auth_manager {
        match auth_manager
            .list_capability_extensions(&context.user_id)
            .await
        {
            Ok(extensions) => Some(
                extensions
                    .into_iter()
                    .map(|extension| (extension.name.clone(), extension))
                    .collect::<HashMap<_, _>>(),
            ),
            Err(error) => {
                debug!(
                    user_id = %context.user_id,
                    error = %error,
                    "failed to load extension inventory for available_actions; omitting extension-backed actions"
                );
                Some(HashMap::new())
            }
        }
    } else {
        None
    };
    let latent_actions = if let Some(auth_manager) = auth_manager {
        auth_manager.latent_provider_actions(&context.user_id).await
    } else {
        Vec::new()
    };
    let tool_permissions = ToolPermissionSnapshot::load(tools, &context.user_id).await;

    InventoryInputs {
        tool_defs,
        extension_statuses,
        latent_actions,
        tool_permissions,
    }
}

fn classify_projected_actions(
    inputs: InventoryInputs,
    capability_registry: Option<&Arc<CapabilityRegistry>>,
    leases: &[CapabilityLease],
) -> ActionInventory {
    let InventoryInputs {
        tool_defs,
        extension_statuses,
        latent_actions,
        tool_permissions,
    } = inputs;
    let mut inline = Vec::with_capacity(tool_defs.len());
    let mut discoverable = Vec::new();

    for tool in tool_defs {
        match classify_registered_tool(
            tool.as_ref(),
            extension_statuses.as_ref(),
            &tool_permissions,
        ) {
            ProjectedAction::Inline(action) => inline.push(action),
            ProjectedAction::Discoverable(action) => discoverable.push(action),
            ProjectedAction::Hidden => {}
        }
    }

    let mut seen_inline: HashSet<String> =
        inline.iter().map(|action| action.name.clone()).collect();

    if let Some(registry) = capability_registry {
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
                if !assignment.available_actions || !seen_inline.insert(action.name.clone()) {
                    continue;
                }
                inline.push(action.clone());
            }
        }
    }

    let mut seen_discoverable: HashSet<String> = seen_inline.clone();
    for latent in latent_actions {
        if tool_permissions
            .resolve_permission(&latent.action_name)
            .effective
            == PermissionState::Disabled
        {
            continue;
        }
        let action = project_latent_action(latent);
        if seen_discoverable.insert(action.name.clone()) {
            discoverable.push(action);
        }
    }
    discoverable.retain(|action| seen_inline.insert(action.name.clone()));

    inline.sort_by(|a, b| a.name.cmp(&b.name));
    discoverable.sort_by(|a, b| a.name.cmp(&b.name));
    ActionInventory {
        inline,
        discoverable,
    }
}

enum ProjectedAction {
    Inline(ActionDef),
    Discoverable(ActionDef),
    Hidden,
}

fn classify_registered_tool(
    tool: &dyn crate::tools::Tool,
    extension_statuses: Option<&HashMap<String, InstalledExtension>>,
    tool_permissions: &ToolPermissionSnapshot,
) -> ProjectedAction {
    if crate::bridge::effect_adapter::is_v1_only_tool(tool.name()) {
        return ProjectedAction::Hidden;
    }
    if crate::bridge::effect_adapter::is_v1_auth_tool(tool.name()) {
        return ProjectedAction::Hidden;
    }
    if hidden_from_model_callable_surface(tool.name()) {
        return ProjectedAction::Hidden;
    }
    if tool_permissions.resolve_permission(tool.name()).effective == PermissionState::Disabled {
        return ProjectedAction::Hidden;
    }

    if let Some(provider_extension) = tool.provider_extension() {
        let Some(extension_statuses) = extension_statuses else {
            return ProjectedAction::Hidden;
        };
        let Some(extension) = provider_extension_status(extension_statuses, provider_extension)
        else {
            return ProjectedAction::Hidden;
        };
        let status = capability_status_for_extension(extension, false);
        let (kind, invocation_mode) = capability_surface_subject_for_extension(extension);
        let assignment = assign_surface(SurfacePolicyInput {
            kind,
            status,
            invocation_mode,
            leased_and_callable: false,
        });
        let action = project_tool_action(tool);
        if assignment.available_actions {
            ProjectedAction::Inline(action)
        } else if supports_pre_activation_discovery(kind, invocation_mode, status) {
            ProjectedAction::Discoverable(action)
        } else {
            ProjectedAction::Hidden
        }
    } else {
        ProjectedAction::Inline(project_tool_action(tool))
    }
}

fn hidden_from_model_callable_surface(tool_name: &str) -> bool {
    matches!(tool_name, "tool_install" | "tool-install")
}

fn supports_pre_activation_discovery(
    kind: SurfaceSubjectKind,
    invocation_mode: InvocationMode,
    status: CapabilityStatus,
) -> bool {
    matches!(
        (kind, invocation_mode, status),
        (
            SurfaceSubjectKind::ExtensionDirectAction
                | SurfaceSubjectKind::AvailableNotInstalledProviderEntry,
            InvocationMode::Direct,
            CapabilityStatus::NeedsAuth
                | CapabilityStatus::NeedsSetup
                | CapabilityStatus::Inactive
                | CapabilityStatus::AvailableNotInstalled
        )
    )
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
    let discovery =
        (summary.is_some() || schema_override.is_some()).then_some(ActionDiscoveryMetadata {
            name: callable_name.clone(),
            summary,
            schema_override,
        });
    let model_tool_surface = default_model_tool_surface(&callable_name);

    ActionDef {
        name: callable_name,
        description: tool.description().to_string(),
        parameters_schema: callable_schema,
        effects: vec![],
        requires_approval: false,
        model_tool_surface,
        discovery,
    }
}

fn project_latent_action(action: LatentProviderAction) -> ActionDef {
    let callable_name = action.action_name.replace('-', "_");
    ActionDef {
        name: callable_name.clone(),
        description: action.description,
        parameters_schema: action.parameters_schema,
        effects: vec![],
        requires_approval: false,
        model_tool_surface: default_model_tool_surface(&callable_name),
        discovery: None,
    }
}

pub(crate) fn default_model_tool_surface(action_name: &str) -> ModelToolSurface {
    if matches!(action_name, "echo" | "http" | "json" | "time")
        || action_name.starts_with("memory_")
        || action_name.starts_with("skill_")
        || action_name.starts_with("tool_")
    {
        ModelToolSurface::FullSchema
    } else {
        ModelToolSurface::CompactToolInfo
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
    use ironclaw_engine::{ActionInventory, ModelToolSurface, ThreadExecutionContext};

    use super::{
        ActionProjector, default_model_tool_surface, project_tool_action, provider_extension_status,
    };
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
    struct PlainTool;
    struct BuiltinTool {
        name: &'static str,
    }

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

    #[async_trait]
    impl crate::tools::Tool for PlainTool {
        fn name(&self) -> &str {
            "plain_helper"
        }

        fn description(&self) -> &str {
            "Plain helper"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"id": {"type": "string"}}})
        }

        async fn execute(
            &self,
            _: serde_json::Value,
            _: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            unreachable!("not needed")
        }
    }

    #[async_trait]
    impl crate::tools::Tool for BuiltinTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "Built-in helper"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _: serde_json::Value,
            _: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            unreachable!("not needed")
        }
    }

    async fn projected_inventory(
        tool_name: &'static str,
        description: &'static str,
        provider_extension: &'static str,
        extension: InstalledExtension,
    ) -> ActionInventory {
        let tools = std::sync::Arc::new(ToolRegistry::new());
        tools
            .register(std::sync::Arc::new(ProviderTool {
                name: tool_name,
                description,
                provider_extension,
            }))
            .await;

        let extension_map = HashMap::from([(extension.name.clone(), extension)]);
        ActionProjector::project_inventory(
            tools.as_ref(),
            None,
            None,
            &[],
            &test_context(),
            Some(&extension_map),
        )
        .await
        .expect("project should succeed")
    }

    async fn inventory_with_tool_permission(
        tool_name: &'static str,
        permission: crate::tools::permissions::PermissionState,
    ) -> ActionInventory {
        let db_path = std::env::temp_dir().join(format!(
            "ironclaw-action-projector-permissions-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = crate::db::connect_from_config(&crate::config::DatabaseConfig::from_libsql_path(
            db_path.to_str().expect("db path"),
            None,
            None,
        ))
        .await
        .expect("db");
        db.set_setting(
            "test_user",
            &format!("tool_permissions.{tool_name}"),
            &serde_json::to_value(permission).expect("serialize permission"),
        )
        .await
        .expect("save tool permission");

        let tools = std::sync::Arc::new(ToolRegistry::new().with_database(db));
        tools
            .register(std::sync::Arc::new(BuiltinTool { name: tool_name }))
            .await;

        ActionProjector::project_inventory(tools.as_ref(), None, None, &[], &test_context(), None)
            .await
            .expect("project should succeed")
    }

    async fn provider_inventory_with_tool_permission(
        tool_name: &'static str,
        provider_extension: &'static str,
        extension: InstalledExtension,
        permission: crate::tools::permissions::PermissionState,
    ) -> ActionInventory {
        let db_path = std::env::temp_dir().join(format!(
            "ironclaw-action-projector-provider-permissions-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = crate::db::connect_from_config(&crate::config::DatabaseConfig::from_libsql_path(
            db_path.to_str().expect("db path"),
            None,
            None,
        ))
        .await
        .expect("db");
        db.set_setting(
            "test_user",
            &format!("tool_permissions.{tool_name}"),
            &serde_json::to_value(permission).expect("serialize permission"),
        )
        .await
        .expect("save tool permission");

        let tools = std::sync::Arc::new(ToolRegistry::new().with_database(db));
        tools
            .register(std::sync::Arc::new(ProviderTool {
                name: tool_name,
                description: "Provider action",
                provider_extension,
            }))
            .await;

        let extension_map = HashMap::from([(extension.name.clone(), extension)]);
        ActionProjector::project_inventory(
            tools.as_ref(),
            None,
            None,
            &[],
            &test_context(),
            Some(&extension_map),
        )
        .await
        .expect("project should succeed")
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
            available_action_inventory_snapshot: None,
            gate_controller: ironclaw_engine::CancellingGateController::arc(),
            call_approval_granted: false,
            conversation_id: None,
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

    #[test]
    fn project_tool_action_omits_empty_discovery_metadata() {
        let tool = std::sync::Arc::new(PlainTool);
        let action = project_tool_action(tool.as_ref());

        assert_eq!(action.description, "Plain helper");
        assert!(action.discovery.is_none());
    }

    #[test]
    fn default_model_tool_surface_matches_allowlist_contract() {
        assert_eq!(
            default_model_tool_surface("echo"),
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            default_model_tool_surface("http"),
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            default_model_tool_surface("json"),
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            default_model_tool_surface("time"),
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            default_model_tool_surface("memory_read"),
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            default_model_tool_surface("skill_install"),
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            default_model_tool_surface("tool_info"),
            ModelToolSurface::FullSchema
        );

        assert_eq!(
            default_model_tool_surface("mission_create"),
            ModelToolSurface::CompactToolInfo
        );
        assert_eq!(
            default_model_tool_surface("gmail_send"),
            ModelToolSurface::CompactToolInfo
        );
        assert_eq!(
            default_model_tool_surface("notion_search"),
            ModelToolSurface::CompactToolInfo
        );
    }

    #[test]
    fn project_tool_action_sets_model_surface_from_allowlist() {
        let full_schema_tool = BuiltinTool {
            name: "memory_read",
        };
        let compact_tool = BuiltinTool {
            name: "mission_create",
        };
        let full_schema_action = project_tool_action(&full_schema_tool);
        let compact_action = project_tool_action(&compact_tool);

        assert_eq!(
            full_schema_action.model_tool_surface,
            ModelToolSurface::FullSchema
        );
        assert_eq!(
            compact_action.model_tool_surface,
            ModelToolSurface::CompactToolInfo
        );
    }

    #[tokio::test]
    async fn needs_auth_provider_tools_omitted_from_available_actions() {
        let inventory = projected_inventory(
            "gmail_send",
            "Send a Gmail message",
            "gmail",
            needs_auth_extension("gmail"),
        )
        .await;

        assert!(
            !inventory
                .inline
                .iter()
                .any(|action| action.name == "gmail_send"),
            "NeedsAuth provider tool should be omitted from available_actions, got: {:?}",
            inventory.inline
        );
        assert!(
            inventory
                .discoverable
                .iter()
                .any(|action| action.name == "gmail_send"),
            "NeedsAuth provider tool should remain discoverable, got: {:?}",
            inventory.discoverable
        );
    }

    #[tokio::test]
    async fn needs_setup_provider_tools_omitted_from_available_actions() {
        let inventory = projected_inventory(
            "notion_search",
            "Search Notion",
            "notion",
            needs_setup_extension("notion"),
        )
        .await;

        assert!(
            !inventory
                .inline
                .iter()
                .any(|action| action.name == "notion_search"),
            "NeedsSetup provider tool should be omitted from available_actions, got: {:?}",
            inventory.inline
        );
        assert!(
            inventory
                .discoverable
                .iter()
                .any(|action| action.name == "notion_search"),
            "NeedsSetup provider tool should remain discoverable, got: {:?}",
            inventory.discoverable
        );
    }

    #[tokio::test]
    async fn inactive_provider_tools_omitted_from_available_actions() {
        let inventory = projected_inventory(
            "github_search",
            "Search GitHub",
            "github",
            inactive_extension("github"),
        )
        .await;

        assert!(
            !inventory
                .inline
                .iter()
                .any(|action| action.name == "github_search"),
            "Inactive provider tool should be omitted from available_actions, got: {:?}",
            inventory.inline
        );
        assert!(
            inventory
                .discoverable
                .iter()
                .any(|action| action.name == "github_search"),
            "Inactive provider tool should remain discoverable, got: {:?}",
            inventory.discoverable
        );
    }

    #[tokio::test]
    async fn routed_channel_tools_omitted_from_available_actions() {
        let inventory = projected_inventory(
            "telegram_send",
            "Send a Telegram message",
            "telegram",
            channel_extension("telegram"),
        )
        .await;

        assert!(
            !inventory
                .inline
                .iter()
                .any(|action| action.name == "telegram_send"),
            "Routed-only channel tool should be omitted from available_actions, got: {:?}",
            inventory.inline
        );
        assert!(
            !inventory
                .discoverable
                .iter()
                .any(|action| action.name == "telegram_send"),
            "Routed-only channel tool should stay out of discoverable inventory, got: {:?}",
            inventory.discoverable
        );
    }

    #[tokio::test]
    async fn latent_provider_tools_omitted_from_available_actions() {
        let inventory = projected_inventory(
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
            !inventory
                .inline
                .iter()
                .any(|action| action.name == "latent_send"),
            "Not-installed provider tool should be omitted from available_actions, got: {:?}",
            inventory.inline
        );
        assert!(
            inventory
                .discoverable
                .iter()
                .any(|action| action.name == "latent_send"),
            "Not-installed provider tool should remain discoverable, got: {:?}",
            inventory.discoverable
        );
    }

    #[tokio::test]
    async fn tool_install_hidden_from_model_callable_surface() {
        let tools = std::sync::Arc::new(ToolRegistry::new());
        tools
            .register(std::sync::Arc::new(BuiltinTool {
                name: "tool_install",
            }))
            .await;
        tools
            .register(std::sync::Arc::new(BuiltinTool {
                name: "tool_activate",
            }))
            .await;

        let inventory = ActionProjector::project_inventory(
            tools.as_ref(),
            None,
            None,
            &[],
            &test_context(),
            None,
        )
        .await
        .expect("project should succeed");
        let action_names = inventory
            .inline
            .into_iter()
            .map(|action| action.name)
            .collect::<Vec<_>>();

        assert!(!action_names.iter().any(|name| name == "tool_install"));
        assert!(action_names.iter().any(|name| name == "tool_activate"));
    }

    #[tokio::test]
    async fn disabled_tools_are_omitted_from_inventory() {
        let inventory = inventory_with_tool_permission(
            "message",
            crate::tools::permissions::PermissionState::Disabled,
        )
        .await;

        assert!(
            inventory
                .inline
                .iter()
                .all(|action| action.name != "message"),
            "disabled tool should be hidden from available_actions, got: {:?}",
            inventory.inline
        );
    }

    #[tokio::test]
    async fn ask_each_time_tools_remain_visible_in_inventory() {
        let inventory = inventory_with_tool_permission(
            "message",
            crate::tools::permissions::PermissionState::AskEachTime,
        )
        .await;

        assert!(
            inventory
                .inline
                .iter()
                .any(|action| action.name == "message"),
            "ask_each_time tool should remain visible in available_actions, got: {:?}",
            inventory.inline
        );
    }

    #[tokio::test]
    async fn disabled_provider_tools_are_omitted_from_discoverable_inventory() {
        let inventory = provider_inventory_with_tool_permission(
            "gmail_send",
            "gmail",
            needs_auth_extension("gmail"),
            crate::tools::permissions::PermissionState::Disabled,
        )
        .await;

        assert!(
            inventory
                .discoverable
                .iter()
                .all(|action| action.name != "gmail_send"),
            "disabled provider tool should be hidden from discoverable inventory, got: {:?}",
            inventory.discoverable
        );
    }
}
