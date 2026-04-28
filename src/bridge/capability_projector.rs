use std::collections::{BTreeMap, HashMap, HashSet};

use ironclaw_engine::{
    CapabilityLease, CapabilityStatus, CapabilitySummary, CapabilitySummaryKind, EngineError,
    ThreadExecutionContext,
};

use crate::auth::extension::AuthManager;
use crate::bridge::tool_surface::{
    InvocationMode, SurfacePolicyInput, SurfaceSubjectKind, assign_surface,
};
use crate::extensions::naming::extension_name_candidates;
use crate::extensions::{ExtensionKind, InstalledExtension, LatentProviderAction};

pub(crate) struct CapabilityProjector;

struct CapabilityRuntimeSnapshot {
    extensions: Vec<InstalledExtension>,
    latent_actions: Vec<LatentProviderAction>,
    channel_routes: HashMap<String, String>,
}

impl CapabilityProjector {
    /// Project the set of capability summaries from the runtime extension state.
    ///
    /// When `prefetched_extensions` is `Some`, the projector uses that list
    /// instead of fetching from `auth_manager`. This allows the caller
    /// (typically `EffectBridgeAdapter`) to share a single fetch across
    /// both `ActionProjector` and `CapabilityProjector`.
    pub(crate) async fn project(
        auth_manager: Option<&AuthManager>,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
        prefetched_extensions: Option<Vec<InstalledExtension>>,
    ) -> Result<Vec<CapabilitySummary>, EngineError> {
        let Some(auth_manager) = auth_manager else {
            return Ok(Vec::new());
        };

        let extensions = if let Some(prefetched) = prefetched_extensions {
            prefetched
        } else {
            auth_manager
                .list_capability_extensions(&context.user_id)
                .await
                .map_err(|error| EngineError::Effect {
                    reason: format!("Failed to list extensions for capability projection: {error}"),
                })?
        };

        let latent_actions = auth_manager.latent_provider_actions(&context.user_id).await;
        let mut channel_routes = HashMap::new();
        let channel_route_lookups = extensions
            .iter()
            .filter(|extension| is_channel_extension_kind(extension.kind))
            .map(|extension| {
                let name = extension.name.clone();
                async {
                    let target = auth_manager.notification_target_for_channel(&name).await;
                    (name, target)
                }
            });

        for (name, target) in futures::future::join_all(channel_route_lookups).await {
            if let Some(target) = target {
                channel_routes.insert(name, target);
            }
        }

        Ok(Self::project_snapshot(
            CapabilityRuntimeSnapshot {
                extensions,
                latent_actions,
                channel_routes,
            },
            leases,
        ))
    }

    fn project_snapshot(
        snapshot: CapabilityRuntimeSnapshot,
        _leases: &[CapabilityLease],
    ) -> Vec<CapabilitySummary> {
        let mut summaries = BTreeMap::<String, PrioritizedSummary>::new();
        let mut registry_only = Vec::new();
        let mut installed_keys = HashSet::new();
        let latent_action_previews = latent_action_previews(&snapshot.latent_actions);

        for extension in snapshot.extensions {
            let normalized_key = normalized_capability_key(&extension.name);
            if extension.installed {
                installed_keys.insert(normalized_key.clone());
                if let Some(summary) = summarize_extension(
                    &extension,
                    &snapshot.channel_routes,
                    latent_action_previews.get(&normalized_key),
                ) {
                    upsert_summary(&mut summaries, summary, ProjectionSource::InstalledRuntime);
                }
            } else {
                registry_only.push(extension);
            }
        }

        for extension in registry_only {
            let normalized_key = normalized_capability_key(&extension.name);
            if installed_keys.contains(&normalized_key) || summaries.contains_key(&normalized_key) {
                continue;
            }

            if let Some(summary) = summarize_extension(
                &extension,
                &snapshot.channel_routes,
                latent_action_previews.get(&normalized_key),
            ) {
                upsert_summary(&mut summaries, summary, ProjectionSource::RegistryOnly);
            }
        }

        for summary in summarize_latent_only_providers(&snapshot.latent_actions) {
            let normalized_key = normalized_capability_key(&summary.name);
            if installed_keys.contains(&normalized_key) || summaries.contains_key(&normalized_key) {
                continue;
            }
            upsert_summary(&mut summaries, summary, ProjectionSource::RegistryOnly);
        }

        summaries
            .into_values()
            .map(|summary| summary.summary)
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProjectionSource {
    RegistryOnly,
    InstalledRuntime,
}

struct PrioritizedSummary {
    source: ProjectionSource,
    summary: CapabilitySummary,
}

fn upsert_summary(
    summaries: &mut BTreeMap<String, PrioritizedSummary>,
    summary: CapabilitySummary,
    source: ProjectionSource,
) {
    let key = normalized_capability_key(&summary.name);
    match summaries.get(&key) {
        Some(existing) if existing.source >= source => {}
        _ => {
            summaries.insert(key, PrioritizedSummary { source, summary });
        }
    }
}

fn normalized_capability_key(name: &str) -> String {
    extension_name_candidates(name)
        .into_iter()
        .next()
        .unwrap_or_else(|| name.to_string())
}

fn summarize_extension(
    extension: &InstalledExtension,
    channel_routes: &HashMap<String, String>,
    latent_action_preview: Option<&Vec<String>>,
) -> Option<CapabilitySummary> {
    let status =
        capability_status_for_extension(extension, channel_routes.contains_key(&extension.name));
    let (subject_kind, invocation_mode) = capability_surface_subject_for_extension(extension);
    let assignment = assign_surface(SurfacePolicyInput {
        kind: subject_kind,
        status,
        invocation_mode,
        leased_and_callable: false,
    });
    if !assignment.available_capabilities {
        return None;
    }

    let routing_hint = if matches!(status, CapabilityStatus::ReadyScoped) {
        Some("Usable through message".to_string())
    } else {
        None
    };
    let action_preview = action_preview_for_extension(extension, latent_action_preview, status);

    Some(CapabilitySummary {
        name: extension.name.clone(),
        display_name: extension.display_name.clone(),
        kind: if is_channel_extension_kind(extension.kind) {
            CapabilitySummaryKind::Channel
        } else {
            CapabilitySummaryKind::Provider
        },
        status,
        description: extension.description.clone(),
        action_preview,
        routing_hint,
    })
}

pub(crate) fn capability_surface_subject_for_extension(
    extension: &InstalledExtension,
) -> (SurfaceSubjectKind, InvocationMode) {
    if is_channel_extension_kind(extension.kind) {
        return (SurfaceSubjectKind::Channel, InvocationMode::RoutedOnly);
    }

    if extension.installed {
        (
            SurfaceSubjectKind::ExtensionDirectAction,
            InvocationMode::Direct,
        )
    } else {
        (
            SurfaceSubjectKind::AvailableNotInstalledProviderEntry,
            InvocationMode::Direct,
        )
    }
}

pub(crate) fn capability_status_for_extension(
    extension: &InstalledExtension,
    route_exists: bool,
) -> CapabilityStatus {
    if !extension.installed {
        return CapabilityStatus::AvailableNotInstalled;
    }
    if extension.activation_error.is_some() {
        return CapabilityStatus::Error;
    }
    if extension.needs_setup {
        return CapabilityStatus::NeedsSetup;
    }
    if extension.has_auth && !extension.authenticated {
        return CapabilityStatus::NeedsAuth;
    }
    if !extension.active {
        return CapabilityStatus::Inactive;
    }
    if is_channel_extension_kind(extension.kind) {
        return if route_exists {
            CapabilityStatus::ReadyScoped
        } else {
            CapabilityStatus::Inactive
        };
    }

    CapabilityStatus::Ready
}

fn action_preview_for_extension(
    extension: &InstalledExtension,
    latent_action_preview: Option<&Vec<String>>,
    status: CapabilityStatus,
) -> Vec<String> {
    const MAX_ACTION_PREVIEW: usize = 3;

    if !matches!(
        status,
        CapabilityStatus::NeedsAuth
            | CapabilityStatus::NeedsSetup
            | CapabilityStatus::Inactive
            | CapabilityStatus::Latent
            | CapabilityStatus::AvailableNotInstalled
    ) {
        return Vec::new();
    }

    let mut preview = if !extension.tools.is_empty() {
        extension.tools.clone()
    } else {
        latent_action_preview.cloned().unwrap_or_default()
    };
    preview.truncate(MAX_ACTION_PREVIEW);
    preview
}

fn latent_action_previews(latent_actions: &[LatentProviderAction]) -> HashMap<String, Vec<String>> {
    let mut previews = HashMap::<String, Vec<String>>::new();
    for latent in latent_actions {
        let key = normalized_capability_key(&latent.provider_extension);
        let preview = previews.entry(key).or_default();
        let canonical_name = latent.action_name.replace('-', "_");
        if !preview.contains(&canonical_name) {
            preview.push(canonical_name);
        }
    }
    previews
}

fn summarize_latent_only_providers(
    latent_actions: &[LatentProviderAction],
) -> Vec<CapabilitySummary> {
    const MAX_ACTION_PREVIEW: usize = 3;
    let mut summaries = BTreeMap::<String, CapabilitySummary>::new();

    for latent in latent_actions {
        let key = normalized_capability_key(&latent.provider_extension);
        let summary = summaries.entry(key).or_insert_with(|| CapabilitySummary {
            name: latent.provider_extension.clone(),
            display_name: None,
            kind: CapabilitySummaryKind::Provider,
            status: CapabilityStatus::Latent,
            description: Some(latent.description.clone()),
            action_preview: Vec::new(),
            routing_hint: None,
        });
        let canonical_name = latent.action_name.replace('-', "_");
        if summary.action_preview.len() < MAX_ACTION_PREVIEW
            && !summary.action_preview.contains(&canonical_name)
        {
            summary.action_preview.push(canonical_name);
        }
    }

    summaries.into_values().collect()
}

pub(crate) const fn is_channel_extension_kind(kind: ExtensionKind) -> bool {
    matches!(
        kind,
        ExtensionKind::WasmChannel | ExtensionKind::ChannelRelay
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironclaw_engine::{
        CapabilityLease, CapabilityStatus, CapabilitySummaryKind, GrantedActions,
    };

    use super::{CapabilityProjector, CapabilityRuntimeSnapshot};
    use crate::extensions::{ExtensionKind, InstalledExtension, LatentProviderAction};

    fn make_lease() -> CapabilityLease {
        CapabilityLease {
            id: ironclaw_engine::LeaseId::new(),
            thread_id: ironclaw_engine::ThreadId::new(),
            capability_name: "tools".to_string(),
            granted_actions: GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        }
    }

    fn installed_extension(name: &str, kind: ExtensionKind) -> InstalledExtension {
        InstalledExtension {
            name: name.to_string(),
            kind,
            display_name: Some(name.to_string()),
            description: Some(format!("{name} description")),
            url: None,
            authenticated: true,
            active: true,
            tools: vec![format!("{name}_send")],
            needs_setup: false,
            has_auth: true,
            installed: true,
            activation_error: None,
            version: None,
        }
    }

    fn available_extension(name: &str) -> InstalledExtension {
        InstalledExtension {
            installed: false,
            active: false,
            authenticated: false,
            has_auth: true,
            tools: Vec::new(),
            ..installed_extension(name, ExtensionKind::WasmTool)
        }
    }

    fn latent_action(provider_extension: &str) -> LatentProviderAction {
        LatentProviderAction {
            action_name: format!("{provider_extension}_send"),
            provider_extension: provider_extension.to_string(),
            description: format!("{provider_extension} latent action"),
            parameters_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn projects_normalized_capability_statuses() {
        let mut telegram = installed_extension("telegram", ExtensionKind::WasmChannel);
        telegram.tools.clear();

        let mut slack = installed_extension("slack", ExtensionKind::ChannelRelay);
        slack.authenticated = false;

        let mut github = installed_extension("github", ExtensionKind::McpServer);
        github.needs_setup = true;
        github.authenticated = false;

        let mut notion = installed_extension("notion", ExtensionKind::WasmTool);
        notion.active = false;

        let mut broken = installed_extension("broken", ExtensionKind::WasmChannel);
        broken.active = false;
        broken.activation_error = Some("activation failed".to_string());

        let unpaired = installed_extension("discord", ExtensionKind::WasmChannel);

        let snapshot = CapabilityRuntimeSnapshot {
            extensions: vec![
                telegram,
                slack,
                github,
                notion,
                broken,
                unpaired,
                available_extension("linear"),
            ],
            latent_actions: vec![latent_action("gmail")],
            channel_routes: HashMap::from([("telegram".to_string(), "actor".to_string())]),
        };

        let projected = CapabilityProjector::project_snapshot(snapshot, &[make_lease()]);
        let by_name = projected
            .into_iter()
            .map(|summary| (summary.name.clone(), summary))
            .collect::<HashMap<_, _>>();

        assert_eq!(by_name["telegram"].kind, CapabilitySummaryKind::Channel);
        assert_eq!(by_name["telegram"].status, CapabilityStatus::ReadyScoped);
        assert_eq!(
            by_name["telegram"].routing_hint.as_deref(),
            Some("Usable through message")
        );
        assert_eq!(by_name["slack"].status, CapabilityStatus::NeedsAuth);
        assert_eq!(by_name["github"].status, CapabilityStatus::NeedsSetup);
        assert_eq!(by_name["notion"].status, CapabilityStatus::Inactive);
        assert_eq!(by_name["gmail"].status, CapabilityStatus::Latent);
        assert_eq!(
            by_name["gmail"].action_preview,
            vec!["gmail_send".to_string()]
        );
        assert_eq!(
            by_name["linear"].status,
            CapabilityStatus::AvailableNotInstalled
        );
        // "broken" has Error status which is excluded from all surfaces
        // (early return in assign_surface), so it should not appear.
        assert!(!by_name.contains_key("broken"));
        assert_eq!(by_name["discord"].status, CapabilityStatus::Inactive);
        assert_eq!(by_name["discord"].routing_hint, None);
    }

    #[test]
    fn installed_latent_entries_surface_once_with_action_preview() {
        let mut inactive = installed_extension("gmail", ExtensionKind::WasmTool);
        inactive.active = false;

        let snapshot = CapabilityRuntimeSnapshot {
            extensions: vec![inactive, available_extension("gmail")],
            latent_actions: vec![latent_action("gmail")],
            channel_routes: HashMap::new(),
        };

        let projected = CapabilityProjector::project_snapshot(snapshot, &[]);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].name, "gmail");
        assert_eq!(projected[0].status, CapabilityStatus::Inactive);
        assert_eq!(projected[0].action_preview, vec!["gmail_send".to_string()]);
    }

    #[test]
    fn ready_scoped_channels_do_not_clone_tool_lists_into_capability_payload() {
        let mut telegram = installed_extension("telegram", ExtensionKind::WasmChannel);
        telegram.tools = vec![
            "telegram_send".into(),
            "telegram_history".into(),
            "telegram_delete".into(),
        ];

        let snapshot = CapabilityRuntimeSnapshot {
            extensions: vec![telegram],
            latent_actions: Vec::new(),
            channel_routes: HashMap::from([("telegram".to_string(), "actor".to_string())]),
        };

        let projected = CapabilityProjector::project_snapshot(snapshot, &[]);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].status, CapabilityStatus::ReadyScoped);
        assert!(projected[0].action_preview.is_empty());
    }

    #[test]
    fn activatable_previews_are_capped_to_prompt_slice() {
        let mut gmail = available_extension("gmail");
        gmail.tools = vec![
            "gmail_send".into(),
            "gmail_read".into(),
            "gmail_archive".into(),
            "gmail_delete".into(),
        ];

        let snapshot = CapabilityRuntimeSnapshot {
            extensions: vec![gmail],
            latent_actions: Vec::new(),
            channel_routes: HashMap::new(),
        };

        let projected = CapabilityProjector::project_snapshot(snapshot, &[]);
        assert_eq!(projected.len(), 1);
        assert_eq!(
            projected[0].action_preview,
            vec![
                "gmail_send".to_string(),
                "gmail_read".to_string(),
                "gmail_archive".to_string(),
            ]
        );
    }

    #[test]
    fn installed_alias_suppresses_registry_only_canonical_duplicate() {
        let installed = installed_extension("linear-server", ExtensionKind::McpServer);
        let registry_only = InstalledExtension {
            installed: false,
            active: false,
            authenticated: false,
            has_auth: true,
            tools: Vec::new(),
            ..installed_extension("linear_server", ExtensionKind::McpServer)
        };

        let snapshot = CapabilityRuntimeSnapshot {
            extensions: vec![installed, registry_only],
            latent_actions: Vec::new(),
            channel_routes: HashMap::new(),
        };

        let projected = CapabilityProjector::project_snapshot(snapshot, &[]);
        assert!(projected.is_empty());
    }

    #[test]
    fn ready_direct_provider_actions_stay_out_of_capabilities() {
        let snapshot = CapabilityRuntimeSnapshot {
            extensions: vec![installed_extension("drive", ExtensionKind::WasmTool)],
            latent_actions: Vec::new(),
            channel_routes: HashMap::new(),
        };

        let projected = CapabilityProjector::project_snapshot(snapshot, &[]);
        assert!(projected.is_empty());
    }
}
