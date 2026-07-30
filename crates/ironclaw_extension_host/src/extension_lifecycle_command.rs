use std::sync::Arc;

use ironclaw_auth::RuntimeCredentialAccountSelectionService;
use ironclaw_host_api::{InstallationState, ProductSurfaceError, RuntimeHttpEgress};
use ironclaw_product::{
    LifecycleExtensionSource, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleProductService, LifecycleProductSurfaceContext, LifecycleSearchExtensionSummary,
    ProductSurfaceFailure,
};
use thiserror::Error;

use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::lifecycle_product_service::ExtensionHostLifecycleProductService;
use ironclaw_skills::ScopedSkillManagementPort;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornExtensionLifecycleCommand {
    Search { query: String },
    Install { id: String },
    Activate { id: String },
    Remove { id: String },
}

#[derive(Debug, Error)]
pub enum RebornExtensionLifecycleCommandError {
    #[error("extension lifecycle is available only for standalone Reborn services")]
    LocalRuntimeUnavailable,
    #[error("extension lifecycle command is invalid: {0}")]
    ProductCommand(#[from] ProductSurfaceFailure),
    #[error("extension lifecycle failed: {0}")]
    ProductSurface(#[from] ProductSurfaceError),
}

pub trait RebornExtensionLifecycleRuntime {
    fn skill_management(&self) -> Arc<ScopedSkillManagementPort>;
    fn extension_management(&self) -> Arc<RebornLocalExtensionManagementPort>;
    fn runtime_http_egress(&self) -> Option<Arc<dyn RuntimeHttpEgress>>;
    fn runtime_credential_accounts(&self) -> Arc<dyn RuntimeCredentialAccountSelectionService>;
    fn extension_lifecycle_surface_context(&self) -> LifecycleProductSurfaceContext;
}

pub async fn execute_reborn_extension_lifecycle_command(
    runtime: &impl RebornExtensionLifecycleRuntime,
    command: RebornExtensionLifecycleCommand,
) -> Result<LifecycleProductResponse, RebornExtensionLifecycleCommandError> {
    let mut service = ExtensionHostLifecycleProductService::new(runtime.skill_management())
        .with_extension_management(runtime.extension_management());
    if let Some(runtime_http_egress) = runtime.runtime_http_egress() {
        service = service.with_runtime_http_egress(runtime_http_egress);
    }
    service = service.with_runtime_credential_accounts(runtime.runtime_credential_accounts());
    let context = LifecycleProductContext::Surface(runtime.extension_lifecycle_surface_context());
    execute_reborn_extension_lifecycle_service_command(&service, context, command).await
}

pub async fn execute_reborn_extension_lifecycle_service_command(
    service: &ExtensionHostLifecycleProductService,
    context: LifecycleProductContext,
    command: RebornExtensionLifecycleCommand,
) -> Result<LifecycleProductResponse, RebornExtensionLifecycleCommandError> {
    Ok(match command {
        RebornExtensionLifecycleCommand::Install { id } => {
            execute_install_with_activation(service, context, extension_package_ref(id)?).await?
        }
        command => service.execute(context, command.into_action()?).await?,
    })
}

async fn execute_install_with_activation(
    service: &ExtensionHostLifecycleProductService,
    context: LifecycleProductContext,
    package_ref: LifecyclePackageRef,
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
    let mut install_response = service
        .execute(
            context.clone(),
            LifecycleProductAction::ExtensionInstall {
                package_ref: package_ref.clone(),
            },
        )
        .await?;
    let activation_response = service
        .execute(
            context,
            LifecycleProductAction::ExtensionActivate { package_ref },
        )
        .await;
    let Ok(activation_response) = activation_response else {
        return Ok(install_response);
    };
    install_response.phase = activation_response.phase;
    install_response.blockers = activation_response.blockers;
    install_response.message = activation_response.message;
    let activation_visible_capability_ids = match activation_response.payload {
        Some(LifecycleProductPayload::ExtensionActivate {
            visible_capability_ids,
            ..
        }) => Some(visible_capability_ids),
        _ => None,
    };
    if let Some(LifecycleProductPayload::ExtensionInstall {
        visible_capability_ids,
        next_step,
        ..
    }) = install_response.payload.as_mut()
    {
        if let Some(activation_visible_capability_ids) = activation_visible_capability_ids {
            *visible_capability_ids = activation_visible_capability_ids;
        }
        *next_step = if install_response.phase == InstallationState::Active {
            "Activation completed; model-visible extension tools are ready.".to_string()
        } else {
            "Activation did not complete; inspect the lifecycle phase and blockers.".to_string()
        };
    }
    Ok(install_response)
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
        }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            render_string_array(&mut output, visible_capability_ids, "visible_capability");
            push_line(&mut output, format_args!("next_step: {next_step}"));
        }
        Some(LifecycleProductPayload::ExtensionActivate {
            activated,
            visible_capability_ids,
            ..
        }) => {
            push_line(&mut output, format_args!("activated: {activated}"));
            render_string_array(&mut output, visible_capability_ids, "visible_capability");
        }
        Some(LifecycleProductPayload::ExtensionRemove { removed }) => {
            push_line(&mut output, format_args!("removed: {removed}"));
        }
        _ => {}
    }
    output
}

impl RebornExtensionLifecycleCommand {
    fn into_action(self) -> Result<LifecycleProductAction, ProductSurfaceFailure> {
        Ok(match self {
            Self::Search { query } => LifecycleProductAction::ExtensionSearch { query },
            Self::Install { id } => LifecycleProductAction::ExtensionInstall {
                package_ref: extension_package_ref(id)?,
            },
            Self::Activate { id } => LifecycleProductAction::ExtensionActivate {
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
) -> Result<LifecyclePackageRef, ProductSurfaceFailure> {
    Ok(LifecyclePackageRef::new(
        LifecyclePackageKind::Extension,
        id,
    )?)
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
    use ironclaw_host_api::{
        AgentId, InstallationState, InvocationId, ResourceScope, TenantId, UserId,
    };
    use ironclaw_product::LifecycleExtensionSummary;
    use secrecy::SecretString;

    use super::*;
    use crate::lifecycle_test_support::{build_lifecycle_test_services, lifecycle_product_context};
    use ironclaw_auth::{RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest};

    #[tokio::test]
    async fn extension_lifecycle_command_activates_credentialed_extension_with_product_auth() {
        let owner = "extension-lifecycle-command-owner";
        let tenant = "extension-lifecycle-command-tenant";
        let agent = "extension-lifecycle-command-agent";
        let services = build_lifecycle_test_services(owner, None, false).await;
        let product_auth = &services.product_auth;
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
                scope.clone(),
                challenge.interaction_id,
                SecretString::from("github-token".to_string()),
            ))
            .await
            .expect("manual-token submit");

        execute_reborn_extension_lifecycle_service_command(
            &services.lifecycle_service,
            lifecycle_product_context(scope.resource.clone()),
            RebornExtensionLifecycleCommand::Install {
                id: "github".to_string(),
            },
        )
        .await
        .expect("install credentialed extension");
        let activate = execute_reborn_extension_lifecycle_service_command(
            &services.lifecycle_service,
            lifecycle_product_context(scope.resource),
            RebornExtensionLifecycleCommand::Activate {
                id: "github".to_string(),
            },
        )
        .await
        .expect("activate uses product-auth credentials");

        assert_eq!(activate.phase, InstallationState::Active);
        let Some(LifecycleProductPayload::ExtensionActivate {
            activated,
            visible_capability_ids,
            ..
        }) = activate.payload
        else {
            panic!("expected extension activation payload");
        };
        assert!(activated);
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
            phase: InstallationState::Installed,
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
