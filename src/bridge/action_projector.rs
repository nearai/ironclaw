use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::debug;

use ironclaw_engine::{
    ActionDef, CapabilityLease, CapabilityRegistry, CapabilityStatus, EngineError,
    ThreadExecutionContext,
};

use crate::bridge::auth_manager::AuthManager;
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
    pub(crate) async fn project(
        tools: &ToolRegistry,
        auth_manager: Option<&AuthManager>,
        capability_registry: Option<Arc<CapabilityRegistry>>,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
    ) -> Result<Vec<ActionDef>, EngineError> {
        let tool_defs = tools.tool_definitions().await;
        let extension_statuses = if let Some(auth_manager) = auth_manager {
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

        let mut actions = Vec::with_capacity(tool_defs.len());
        for td in tool_defs {
            if crate::bridge::effect_adapter::is_v1_only_tool(&td.name) {
                continue;
            }
            if crate::bridge::effect_adapter::is_v1_auth_tool(&td.name) {
                continue;
            }

            if let Some(provider_extension) = tools.provider_extension_for_tool(&td.name).await {
                let Some(extension_statuses) = extension_statuses.as_ref() else {
                    continue;
                };
                let Some(extension) =
                    provider_extension_status(extension_statuses, &provider_extension)
                else {
                    continue;
                };
                let status = capability_status_for_extension(extension, false);
                let (kind, invocation_mode) = capability_surface_subject_for_extension(extension);
                let assignment = assign_surface(SurfacePolicyInput {
                    kind,
                    status,
                    invocation_mode,
                    approval_gated: false,
                    leased_and_callable: false,
                });
                if !assignment.available_actions {
                    continue;
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
                        approval_gated: action.requires_approval,
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

    use super::provider_extension_status;
    use crate::extensions::{ExtensionKind, InstalledExtension};

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
}
