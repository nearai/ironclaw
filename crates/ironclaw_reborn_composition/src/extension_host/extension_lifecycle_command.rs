use ironclaw_product::{
    LifecycleExtensionSource, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductFacade, LifecycleProductPayload,
    LifecycleProductResponse, LifecyclePublicState, LifecycleSearchExtensionSummary,
    ProductWorkflowError,
};
use std::sync::Arc;
use thiserror::Error;

use crate::extension_host::lifecycle::LifecycleFacade;
use crate::runtime::RebornRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornExtensionLifecycleCommand {
    Search { query: String },
    Install { id: String },
    Remove { id: String },
}

#[derive(Debug, Error)]
pub enum RebornExtensionLifecycleCommandError {
    #[error("extension lifecycle is available only for local-dev Reborn services")]
    LocalRuntimeUnavailable,
    #[error("extension lifecycle failed: {0}")]
    Product(#[from] ProductWorkflowError),
}

pub async fn execute_reborn_extension_lifecycle_command(
    runtime: &RebornRuntime,
    command: RebornExtensionLifecycleCommand,
) -> Result<LifecycleProductResponse, RebornExtensionLifecycleCommandError> {
    let mut facade = LifecycleFacade::new(Arc::clone(&runtime.skill_management));
    facade = facade.with_extension_management(runtime.extension_management.clone());
    if let Some(runtime_http_egress) = &runtime.runtime_http_egress {
        facade = facade.with_runtime_http_egress(runtime_http_egress.clone());
    }
    facade = facade.with_runtime_credential_accounts(
        runtime
            .product_auth
            .runtime_credential_account_selection_service(),
    );
    let context =
        LifecycleProductContext::Surface(runtime.extension_lifecycle_surface_context.clone());
    Ok(facade.execute(context, command.into_action()?).await?)
}

pub fn render_reborn_extension_lifecycle_response(
    label: &str,
    response: &LifecycleProductResponse,
) -> String {
    let mut output = String::new();
    push_line(
        &mut output,
        format_args!("IronClaw Reborn extension {label}"),
    );
    push_line(
        &mut output,
        format_args!("phase: {}", response.phase.as_str()),
    );
    if let Some(package_ref) = &response.package_ref {
        push_line(
            &mut output,
            format_args!("extension: {}", package_ref.id.as_str()),
        );
    }

    match response.payload.as_ref() {
        Some(LifecycleProductPayload::ExtensionSearch { extensions, count }) => {
            render_search_payload(&mut output, extensions, *count);
        }
        Some(LifecycleProductPayload::ExtensionInstall {
            installed,
            visible_capability_ids,
            next_step,
            ..
        }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            push_line(
                &mut output,
                format_args!("active: {}", response.phase == LifecyclePublicState::Active),
            );
            render_string_array(&mut output, visible_capability_ids, "visible_capability");
            push_line(&mut output, format_args!("next_step: {next_step}"));
        }
        Some(LifecycleProductPayload::ExtensionRemove { removed }) => {
            push_line(&mut output, format_args!("removed: {removed}"));
        }
        _ => {}
    }
    output
}

impl RebornExtensionLifecycleCommand {
    fn into_action(self) -> Result<LifecycleProductAction, ProductWorkflowError> {
        Ok(match self {
            Self::Search { query } => LifecycleProductAction::ExtensionSearch { query },
            Self::Install { id } => LifecycleProductAction::ExtensionInstall {
                package_ref: extension_package_ref(id)?,
            },
            Self::Remove { id } => LifecycleProductAction::ExtensionRemove {
                package_ref: extension_package_ref(id)?,
            },
        })
    }
}

fn extension_package_ref(
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)
}

fn render_search_payload(
    output: &mut String,
    extensions: &[LifecycleSearchExtensionSummary],
    count: usize,
) {
    push_line(output, format_args!("count: {count}"));
    for extension in extensions {
        let summary = &extension.summary;
        push_line(
            output,
            format_args!(
                "- {}: {} {} ({})",
                summary.package_ref.id.as_str(),
                terminal_safe(&summary.name),
                terminal_safe(&summary.version),
                extension_source_label(summary.source)
            ),
        );
        if !summary.description.is_empty() {
            push_line(
                output,
                format_args!("  description: {}", terminal_safe(&summary.description)),
            );
        }
        render_string_array(output, &summary.visible_capability_ids, "  capability");
    }
}

fn render_string_array(output: &mut String, items: &[String], label: &str) {
    for item in items {
        push_line(output, format_args!("{label}: {}", terminal_safe(item)));
    }
}

fn extension_source_label(source: LifecycleExtensionSource) -> &'static str {
    match source {
        LifecycleExtensionSource::HostBundled => "host_bundled",
    }
}

fn terminal_safe(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

fn push_line(output: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    #[allow(clippy::let_underscore_must_use)] // writing to a String is infallible
    let _ = output.write_fmt(args);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use ironclaw_auth::{
        AuthContinuationRef, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    };
    use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, TenantId, UserId};
    use ironclaw_product::LifecycleExtensionSummary;
    use secrecy::SecretString;

    use super::*;
    use crate::{
        RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornRuntimeInput,
        build_reborn_runtime,
    };

    #[tokio::test]
    async fn extension_lifecycle_command_activates_credentialed_extension_with_product_auth() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = "extension-lifecycle-command-owner";
        let tenant = "extension-lifecycle-command-tenant";
        let agent = "extension-lifecycle-command-agent";
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_build_input(
                crate::deployment::local_dev_build_input(owner, dir.path().join("local-dev"))
                    .with_runtime_policy(
                        crate::local_dev_runtime_policy().expect("local-dev policy resolves"),
                    ),
            )
            .with_identity(crate::RebornRuntimeIdentity {
                tenant_id: tenant.to_string(),
                agent_id: agent.to_string(),
                source_binding_id: "extension-lifecycle-command-source".to_string(),
                reply_target_binding_id: "extension-lifecycle-command-reply".to_string(),
            }),
        )
        .await
        .expect("local-dev runtime builds");
        let product_auth = &runtime.product_auth;
        let scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new(tenant).expect("tenant"),
                user_id: UserId::new(owner).expect("user"),
                agent_id: Some(AgentId::new(agent).expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Api,
        );
        let provider = AuthProviderId::new("github").expect("provider");
        let challenge = product_auth
            .request_manual_token_setup(RebornManualTokenSetupRequest {
                scope: scope.clone(),
                provider: provider.clone(),
                label: CredentialAccountLabel::new("work github").expect("label"),
                continuation: AuthContinuationRef::SetupOnly,
                update_binding: None,
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            })
            .await
            .expect("manual-token setup challenge");
        product_auth
            .submit_manual_token(RebornManualTokenSubmitRequest::new(
                scope,
                challenge.interaction_id,
                SecretString::from("github-token".to_string()),
            ))
            .await
            .expect("manual-token submit");

        let install = execute_reborn_extension_lifecycle_command(
            &runtime,
            RebornExtensionLifecycleCommand::Install {
                id: "github".to_string(),
            },
        )
        .await
        .expect("install credentialed extension");

        assert_eq!(install.phase, LifecyclePublicState::Active);
        let Some(LifecycleProductPayload::ExtensionInstall {
            visible_capability_ids,
            ..
        }) = install.payload
        else {
            panic!("expected extension install payload");
        };
        assert!(
            visible_capability_ids
                .iter()
                .any(|id| id == "github.search_issues")
        );
        assert!(
            visible_capability_ids
                .iter()
                .any(|id| id == "github.get_issue")
        );
    }

    #[test]
    fn human_renderer_escapes_terminal_control_characters() {
        let response = LifecycleProductResponse {
            package_ref: None,
            phase: LifecyclePublicState::SetupNeeded,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionSearch {
                count: 1,
                extensions: vec![LifecycleSearchExtensionSummary {
                    summary: LifecycleExtensionSummary {
                        package_ref: LifecyclePackageRef::new(
                            LifecyclePackageKind::Extension,
                            "evil",
                        )
                        .expect("package ref"),
                        name: "bad\u{1b}[31mname".to_string(),
                        version: "0.1.0".to_string(),
                        description: "line\rrewrite".to_string(),
                        source: LifecycleExtensionSource::HostBundled,
                        runtime_kind: ironclaw_product::LifecycleExtensionRuntimeKind::WasmTool,
                        surface_kinds: Vec::new(),
                        channel_directions: None,
                        channel_connection: None,
                        channel_presentation: None,
                        visible_capability_ids: Vec::new(),
                        visible_read_only_capability_ids: Vec::new(),
                        credential_requirements: Vec::new(),
                        onboarding: None,
                    },
                    installation_phase: None,
                }],
            }),
        };

        let output = render_reborn_extension_lifecycle_response("search", &response);

        assert!(!output.contains('\u{1b}'), "output: {output:?}");
        assert!(!output.contains('\r'), "output: {output:?}");
        assert!(output.contains("\\u{1b}"));
        assert!(output.contains("\\r"));
    }
}
