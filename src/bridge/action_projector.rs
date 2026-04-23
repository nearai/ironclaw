use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::debug;

use ironclaw_engine::{
    ActionDef, CapabilityLease, CapabilityRegistry, CapabilityStatus, EngineError,
    ThreadExecutionContext,
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
        let tool_defs = tools.tool_definitions().await;
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
        for td in tool_defs {
            if crate::bridge::effect_adapter::is_v1_only_tool(&td.name) {
                continue;
            }
            if crate::bridge::effect_adapter::is_v1_auth_tool(&td.name) {
                continue;
            }

            if let Some(provider_extension) = tools.provider_extension_for_tool(&td.name).await {
                let Some(extension_statuses) = extension_statuses else {
                    continue;
                };
                let Some(extension) =
                    provider_extension_status(extension_statuses, &provider_extension)
                else {
                    continue;
                };
                let status = capability_status_for_extension(extension, false);
                // NeedsAuth tools remain in available_actions so the LLM can
                // trigger the auth-on-first-call gate by attempting to call them.
                if status == CapabilityStatus::NeedsAuth {
                    // Keep in actions — execution will trigger the auth gate.
                } else {
                    let (kind, invocation_mode) =
                        capability_surface_subject_for_extension(extension);
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
            }

            actions.push(ActionDef {
                name: td.name.replace('-', "_"),
                description: td.description,
                parameters_schema: td.parameters,
                effects: vec![],
                requires_approval: false,
            });
        }

        let mut seen: HashSet<String> = actions.iter().map(|a| a.name.clone()).collect();

        if let Some(registry) = capability_registry.as_ref() {
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

        // Surface installed-but-not-yet-ready provider tools (primarily
        // WASM tools pending OAuth) as callable actions. Without this,
        // unauthenticated WASM tools are invisible to the LLM — they're
        // registered in `tool_registry` only after activation, which
        // requires auth first — so the LLM never calls them and the
        // auth-on-first-call gate never fires. See issue #2883.
        if let Some(auth_manager) = auth_manager {
            for latent in auth_manager.latent_provider_actions(&context.user_id).await {
                // Normalize hyphen→underscore to match the first loop's
                // `td.name.replace('-', '_')`. This keeps the action name
                // stable for the LLM before and after auth (registered
                // tools surface as underscores) and ensures the `seen`
                // dedup correctly suppresses overlap with tools already
                // added above.
                let name = latent.action_name.replace('-', "_");
                if !seen.insert(name.clone()) {
                    continue;
                }
                actions.push(ActionDef {
                    name,
                    description: latent.description,
                    parameters_schema: latent.parameters_schema,
                    effects: vec![],
                    requires_approval: false,
                });
            }
        }

        actions.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(actions)
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

    use super::{ActionProjector, provider_extension_status};
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

    /// NeedsAuth provider tools must remain in available_actions so the LLM
    /// can trigger the auth-on-first-call gate by attempting to call them.
    #[tokio::test]
    async fn needs_auth_provider_tools_remain_in_available_actions() {
        use async_trait::async_trait;

        struct GmailSendTool;

        #[async_trait]
        impl crate::tools::Tool for GmailSendTool {
            fn name(&self) -> &str {
                "gmail_send"
            }
            fn description(&self) -> &str {
                "Send a Gmail message"
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
                Some("gmail")
            }
        }

        let tools = std::sync::Arc::new(ToolRegistry::new());
        tools.register(std::sync::Arc::new(GmailSendTool)).await;

        let ext = needs_auth_extension("gmail");
        let extension_map = HashMap::from([(ext.name.clone(), ext)]);

        let context = ironclaw_engine::ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
            thread_goal: None,
        };

        let actions = ActionProjector::project(
            tools.as_ref(),
            None,
            None,
            &[],
            &context,
            Some(&extension_map),
        )
        .await
        .expect("project should succeed");

        assert!(
            actions.iter().any(|a| a.name == "gmail_send"),
            "NeedsAuth provider tool should remain in available_actions, got: {:?}",
            actions.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
    }

    /// Latent/not-installed provider tools should still be omitted.
    #[tokio::test]
    async fn latent_provider_tools_omitted_from_available_actions() {
        use async_trait::async_trait;

        struct LatentTool;

        #[async_trait]
        impl crate::tools::Tool for LatentTool {
            fn name(&self) -> &str {
                "latent_send"
            }
            fn description(&self) -> &str {
                "Send via latent provider"
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
                Some("latent_provider")
            }
        }

        let tools = std::sync::Arc::new(ToolRegistry::new());
        tools.register(std::sync::Arc::new(LatentTool)).await;

        // Extension is not installed — only in registry.
        let ext = InstalledExtension {
            installed: false,
            active: false,
            authenticated: false,
            ..installed_extension("latent_provider")
        };
        let extension_map = HashMap::from([(ext.name.clone(), ext)]);

        let context = ironclaw_engine::ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
            thread_goal: None,
        };

        let actions = ActionProjector::project(
            tools.as_ref(),
            None,
            None,
            &[],
            &context,
            Some(&extension_map),
        )
        .await
        .expect("project should succeed");

        assert!(
            !actions.iter().any(|a| a.name == "latent_send"),
            "Not-installed provider tool should be omitted from available_actions"
        );
    }
}
