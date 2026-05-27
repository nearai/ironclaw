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
        }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            render_string_array(&mut output, visible_capability_ids, "visible_capability");
        }
        Some(LifecycleProductPayload::ExtensionActivate { activated }) => {
            push_line(&mut output, format_args!("activated: {activated}"));
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
        render_string_array(
            output,
            &extension.visible_read_only_capability_ids,
            "  capability",
        );
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
    use super::*;
    use crate::RebornServices;

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
                    visible_read_only_capability_ids: Vec::new(),
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
