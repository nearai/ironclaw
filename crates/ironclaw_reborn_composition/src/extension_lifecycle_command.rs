use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_workflow::{
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
    LifecyclePhase, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleProductSurfaceContext,
    ProductWorkflowError,
};
use thiserror::Error;

use crate::factory::RebornServices;
use crate::lifecycle::RebornLocalLifecycleFacade;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornExtensionLifecycleCommand {
    Search { query: String },
    Install { id: String },
    Activate { id: String },
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
    services: &RebornServices,
    command: RebornExtensionLifecycleCommand,
) -> Result<LifecycleProductResponse, RebornExtensionLifecycleCommandError> {
    let local_runtime = services
        .local_runtime
        .as_ref()
        .ok_or(RebornExtensionLifecycleCommandError::LocalRuntimeUnavailable)?;
    let mut facade = RebornLocalLifecycleFacade::new(local_runtime.skill_management.clone());
    if let Some(extension_management) = &local_runtime.extension_management {
        facade = facade.with_extension_management(extension_management.clone());
    }
    if let Some(runtime_http_egress) = &local_runtime.runtime_http_egress {
        facade = facade.with_runtime_http_egress(runtime_http_egress.clone());
    }
    if let Some(product_auth) = &services.product_auth {
        facade = facade.with_runtime_credential_accounts(
            product_auth.runtime_credential_account_selection_service(),
        );
    }
    Ok(facade
        .execute(
            extension_lifecycle_surface_context()?,
            command.into_action()?,
        )
        .await?)
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
        format_args!("phase: {}", phase_label(response.phase)),
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
    fn into_action(self) -> Result<LifecycleProductAction, ProductWorkflowError> {
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

fn extension_lifecycle_surface_context() -> Result<LifecycleProductContext, ProductWorkflowError> {
    Ok(LifecycleProductContext::Surface(
        LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("reborn-cli").map_err(invalid_surface_context)?,
            user_id: UserId::new("reborn-cli").map_err(invalid_surface_context)?,
            agent_id: None,
            project_id: None,
        },
    ))
}

fn invalid_surface_context(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

fn extension_package_ref(
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)
}

fn render_search_payload(
    output: &mut String,
    extensions: &[LifecycleExtensionSummary],
    count: usize,
) {
    push_line(output, format_args!("count: {count}"));
    for extension in extensions {
        push_line(
            output,
            format_args!(
                "- {}: {} {} ({})",
                extension.package_ref.id.as_str(),
                terminal_safe(&extension.name),
                terminal_safe(&extension.version),
                extension_source_label(extension.source)
            ),
        );
        if !extension.description.is_empty() {
            push_line(
                output,
                format_args!("  description: {}", terminal_safe(&extension.description)),
            );
        }
        render_string_array(output, &extension.visible_capability_ids, "  capability");
    }
}

fn render_string_array(output: &mut String, items: &[String], label: &str) {
    for item in items {
        push_line(output, format_args!("{label}: {}", terminal_safe(item)));
    }
}

fn phase_label(phase: LifecyclePhase) -> &'static str {
    match phase {
        LifecyclePhase::Discovered => "discovered",
        LifecyclePhase::Installing => "installing",
        LifecyclePhase::Installed => "installed",
        LifecyclePhase::Configured => "configured",
        LifecyclePhase::Activating => "activating",
        LifecyclePhase::Active => "active",
        LifecyclePhase::Disabled => "disabled",
        LifecyclePhase::UpgradeRequired => "upgrade_required",
        LifecyclePhase::Failed => "failed",
        LifecyclePhase::Removing => "removing",
        LifecyclePhase::Removed => "removed",
        LifecyclePhase::UnsupportedOrLegacy => "unsupported_or_legacy",
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
    let _ = output.write_fmt(args);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use ironclaw_auth::{
        AuthContinuationRef, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId};
    use secrecy::SecretString;

    use super::*;
    use crate::{
        RebornBuildInput, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
        RebornServices, build_reborn_services,
    };

    #[tokio::test]
    async fn extension_lifecycle_command_rejects_services_without_local_runtime() {
        let error = execute_reborn_extension_lifecycle_command(
            &RebornServices::disabled(),
            RebornExtensionLifecycleCommand::Search {
                query: String::new(),
            },
        )
        .await
        .expect_err("disabled services should not expose local extension lifecycle");

        assert!(matches!(
            error,
            RebornExtensionLifecycleCommandError::LocalRuntimeUnavailable
        ));
    }

    #[tokio::test]
    async fn extension_lifecycle_command_activates_credentialed_extension_with_product_auth() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "reborn-cli",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let product_auth = services
            .product_auth
            .as_ref()
            .expect("local-dev composes product auth");
        let scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new("reborn-cli").expect("tenant"),
                user_id: UserId::new("reborn-cli").expect("user"),
                agent_id: None,
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

        execute_reborn_extension_lifecycle_command(
            &services,
            RebornExtensionLifecycleCommand::Install {
                id: "github".to_string(),
            },
        )
        .await
        .expect("install credentialed extension");
        let activate = execute_reborn_extension_lifecycle_command(
            &services,
            RebornExtensionLifecycleCommand::Activate {
                id: "github".to_string(),
            },
        )
        .await
        .expect("activate uses product-auth credentials");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        let Some(LifecycleProductPayload::ExtensionActivate {
            activated,
            visible_capability_ids,
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
            phase: LifecyclePhase::Discovered,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionSearch {
                count: 1,
                extensions: vec![LifecycleExtensionSummary {
                    package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "evil")
                        .expect("package ref"),
                    name: "bad\u{1b}[31mname".to_string(),
                    version: "0.1.0".to_string(),
                    description: "line\rrewrite".to_string(),
                    source: LifecycleExtensionSource::HostBundled,
                    runtime_kind:
                        ironclaw_product_workflow::LifecycleExtensionRuntimeKind::WasmTool,
                    visible_capability_ids: Vec::new(),
                    visible_read_only_capability_ids: Vec::new(),
                    credential_requirements: Vec::new(),
                    onboarding: None,
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
