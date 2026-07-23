use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::{StreamExt, TryStreamExt, stream};
use ironclaw_auth::{
    AuthAccountLastError, AuthAccountState, CredentialAccountStatus, project_auth_account_state,
};
use ironclaw_host_api::{
    ActivityId, Blocked, CapabilitySurfaceKind, ExtensionId, FailureKind, ProductSurfaceCaller,
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    ProductSurfaceValidationCode, Resolution,
};

use crate::{
    ChannelAuthAccountState, ChannelConnectionFacade, LifecycleExtensionSummary,
    LifecycleInstalledExtensionSummary, LifecyclePackageRef, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductFacade, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleProductSurfaceContext, LifecyclePublicState, ProductView,
    RebornAccountBindingSource, RebornAuthAccount, RebornExtensionActionResponse,
    RebornExtensionInfo, RebornExtensionListResponse, RebornExtensionRegistryEntry,
    RebornExtensionRegistryResponse, RebornExtensionSurface, RebornVendorAuthAccounts,
};

use super::{
    EXTENSION_INSTALL_CAPABILITY, ExtensionCredentialSetupService,
    extension_credentials::{
        ExtensionCredentialReadiness, credential_scope, readiness_for_requirements,
    },
    extension_onboarding,
    lifecycle_setup::{map_lifecycle_error, validation_error},
};

const EXTENSION_READINESS_CONCURRENCY: usize = 8;

pub const EXTENSIONS_VIEW: ProductView<serde_json::Value, RebornExtensionListResponse> =
    ProductView::unpaginated("extensions");

pub const EXTENSION_REGISTRY_VIEW: ProductView<serde_json::Value, RebornExtensionRegistryResponse> =
    ProductView::unpaginated("extension_registry");

/// Executes the caller-scoped extension install operation through the generic
/// product surface and verifies the resulting membership through the
/// authoritative extension projection before reporting success.
///
/// A blocked auth resolution is the expected `setup needed` result after
/// membership is installed. It is accepted only when the readback proves that
/// exact package is now visible to the same caller.
pub async fn install_extension_on_surface(
    surface: &ironclaw_host_api::BoundProductSurface,
    package_ref: LifecyclePackageRef,
    activity_id: ActivityId,
) -> Result<RebornExtensionActionResponse, ProductSurfaceError> {
    let resolution = EXTENSION_INSTALL_CAPABILITY
        .invoke_on(
            surface,
            serde_json::json!({ "extension_id": package_ref.id.as_str() }),
            activity_id,
        )
        .await?;
    install_mutation_succeeded(resolution)?;

    let installed: RebornExtensionListResponse = EXTENSIONS_VIEW
        .query_on(surface, serde_json::json!({}), None)
        .await?;
    let membership_is_visible = installed
        .extensions
        .iter()
        .any(|extension| extension.package_ref == package_ref);
    if !membership_is_visible {
        return Err(extension_install_unavailable(true));
    }

    Ok(RebornExtensionActionResponse {
        success: true,
        message: "Extension installed.".to_string(),
    })
}

fn install_mutation_succeeded(resolution: Resolution) -> Result<(), ProductSurfaceError> {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => Ok(()),
        Resolution::Blocked(Blocked::Auth(_)) => Ok(()),
        Resolution::Done(outcome) => match outcome.verdict.error_kind() {
            Some(FailureKind::InvalidInput) => Err(ProductSurfaceError {
                code: ProductSurfaceErrorCode::InvalidRequest,
                kind: ProductSurfaceErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: None,
                validation_code: Some(ProductSurfaceValidationCode::InvalidValue),
            }),
            Some(FailureKind::OperationFailed) => Err(ProductSurfaceError {
                code: ProductSurfaceErrorCode::InvalidRequest,
                kind: ProductSurfaceErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: None,
                validation_code: None,
            }),
            Some(FailureKind::Authorization | FailureKind::PolicyDenied) => {
                Err(extension_install_forbidden())
            }
            Some(FailureKind::Backend | FailureKind::Transient | FailureKind::Unavailable) => {
                Err(extension_install_unavailable(true))
            }
            _ => Err(ProductSurfaceError::internal_from(
                "extension install capability did not complete successfully",
            )),
        },
        Resolution::Denied(_) => Err(extension_install_forbidden()),
        Resolution::Blocked(_) | Resolution::Suspended(_) => {
            Err(extension_install_unavailable(true))
        }
    }
}

fn extension_install_forbidden() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Forbidden,
        kind: ProductSurfaceErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn extension_install_unavailable(retryable: bool) -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Unavailable,
        kind: ProductSurfaceErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

pub(super) async fn list_extensions(
    facade: Arc<dyn LifecycleProductFacade>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    channel_connection_facade: Arc<dyn ChannelConnectionFacade>,
    caller: ProductSurfaceCaller,
) -> Result<RebornExtensionListResponse, ProductSurfaceError> {
    let context = lifecycle_surface_context(caller.clone());
    let lifecycle = execute_lifecycle(
        facade.as_ref(),
        context.clone(),
        LifecycleProductAction::ExtensionList,
    )
    .await?;
    let installed = lifecycle_installed_extensions(&lifecycle);
    let connections = channel_connection_facade
        .caller_channel_connections(caller.clone())
        .await?;
    // Per-caller auth-account status per channel vendor: lets each account
    // project its real §6.3 state (expired / refresh-failed) instead of the
    // connected/disconnected collapse the connection bool alone permits.
    let account_states = channel_connection_facade
        .caller_channel_account_states(caller.clone())
        .await?;
    // Redacted per-extension activation errors from the durable installation
    // records, projected onto `RebornExtensionInfo::activation_error`.
    let activation_errors = facade
        .installed_activation_errors(context)
        .await
        .map_err(map_lifecycle_error)?;
    Ok(RebornExtensionListResponse {
        extensions: lifecycle_extension_infos(
            installed,
            extension_credentials,
            caller,
            connections,
            account_states,
            activation_errors,
        )
        .await?,
    })
}

pub(super) async fn list_extension_registry(
    facade: &dyn LifecycleProductFacade,
    caller: ProductSurfaceCaller,
) -> Result<RebornExtensionRegistryResponse, ProductSurfaceError> {
    let context = lifecycle_surface_context(caller);
    let (installed_result, registry_result) = tokio::join!(
        execute_lifecycle(
            facade,
            context.clone(),
            LifecycleProductAction::ExtensionList
        ),
        execute_lifecycle(
            facade,
            context,
            LifecycleProductAction::ExtensionSearch {
                query: String::new(),
            },
        ),
    );
    let (installed, registry) = (installed_result?, registry_result?);
    let installed_ids = match &installed.payload {
        Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions.as_slice(),
        _ => &[],
    }
    .iter()
    .map(|extension| extension.summary.package_ref.id.as_str().to_string())
    .collect::<HashSet<_>>();
    let registry_entries = match &registry.payload {
        Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) => extensions.as_slice(),
        _ => &[],
    };
    Ok(RebornExtensionRegistryResponse {
        entries: registry_entries
            .iter()
            .cloned()
            .map(|extension| registry_entry(extension.summary, &installed_ids))
            .collect(),
    })
}

pub(super) async fn import_extension_capability(
    facade: &dyn LifecycleProductFacade,
    caller: ProductSurfaceCaller,
    input: serde_json::Value,
) -> Result<(), ProductSurfaceError> {
    let bundle_base64 = match input {
        serde_json::Value::Object(mut object) => object
            .remove("bundle_base64")
            .and_then(|value| value.as_str().map(ToString::to_string))
            .ok_or_else(|| {
                validation_error("bundle_base64", ProductSurfaceValidationCode::MissingField)
            })?,
        _ => {
            return Err(validation_error(
                "input",
                ProductSurfaceValidationCode::InvalidValue,
            ));
        }
    };
    let bundle = STANDARD.decode(bundle_base64).map_err(|_| {
        validation_error("bundle_base64", ProductSurfaceValidationCode::InvalidValue)
    })?;
    let context = lifecycle_surface_context(caller);
    facade
        .import_extension_bundle(context, bundle)
        .await
        .map_err(map_lifecycle_error)?;
    Ok(())
}

async fn execute_lifecycle(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    action: LifecycleProductAction,
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
    facade
        .execute(context, action)
        .await
        .map_err(map_lifecycle_error)
}

fn lifecycle_surface_context(caller: ProductSurfaceCaller) -> LifecycleProductContext {
    LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
        agent_id: caller.agent_id,
        project_id: caller.project_id,
    })
}

fn lifecycle_installed_extensions(
    lifecycle: &LifecycleProductResponse,
) -> Vec<LifecycleInstalledExtensionSummary> {
    match &lifecycle.payload {
        Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions.clone(),
        _ => Vec::new(),
    }
}

async fn lifecycle_extension_infos(
    installed: Vec<LifecycleInstalledExtensionSummary>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    caller: ProductSurfaceCaller,
    connections: HashMap<String, bool>,
    account_states: HashMap<String, ChannelAuthAccountState>,
    activation_errors: HashMap<String, String>,
) -> Result<Vec<RebornExtensionInfo>, ProductSurfaceError> {
    let resolved = stream::iter(installed)
        .map(|installed| {
            let caller = caller.clone();
            let extension_credentials = extension_credentials.clone();
            async move {
                let readiness = credential_readiness_for_extension(
                    extension_credentials.as_deref(),
                    &caller,
                    &installed,
                )
                .await?;
                Ok::<_, ProductSurfaceError>((installed, readiness))
            }
        })
        .buffered(EXTENSION_READINESS_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(resolved
        .into_iter()
        .map(|(installed, readiness)| {
            extension_info(
                installed,
                readiness,
                &connections,
                &account_states,
                &activation_errors,
            )
        })
        .collect())
}

fn registry_entry(
    summary: LifecycleExtensionSummary,
    installed_ids: &HashSet<String>,
) -> RebornExtensionRegistryEntry {
    let runtime = summary.runtime_kind.runtime_wire_name().to_string();
    let surfaces = wire_surfaces(&summary, None);
    let installed = installed_ids.contains(summary.package_ref.id.as_str());
    RebornExtensionRegistryEntry {
        package_ref: summary.package_ref,
        display_name: summary.name,
        runtime,
        description: summary.description,
        installed,
        keywords: Vec::new(),
        version: Some(summary.version),
        surfaces,
    }
}

async fn credential_readiness_for_extension(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: &ProductSurfaceCaller,
    installed: &LifecycleInstalledExtensionSummary,
) -> Result<ExtensionCredentialReadiness, ProductSurfaceError> {
    let extension_id = ExtensionId::new(installed.summary.package_ref.id.as_str())
        .map_err(|_| ProductSurfaceError::internal_invariant())?;
    let scope = credential_scope(caller, &installed.summary.package_ref);
    readiness_for_requirements(
        extension_credentials,
        scope,
        &extension_id,
        &installed.summary.credential_requirements,
    )
    .await
}

fn extension_info(
    installed: LifecycleInstalledExtensionSummary,
    readiness: ExtensionCredentialReadiness,
    connections: &HashMap<String, bool>,
    account_states: &HashMap<String, ChannelAuthAccountState>,
    activation_errors: &HashMap<String, String>,
) -> RebornExtensionInfo {
    let phase = installed.phase;
    // Redacted activation error for this extension (host installation record's
    // typed `last_error`), threaded onto the card slot the frontend already
    // renders. `None` when the facade surfaces no failure for this extension.
    let activation_error = activation_errors
        .get(installed.summary.package_ref.id.as_str())
        .cloned();
    let onboarding = extension_onboarding::for_installed_with_credential_status(
        &installed,
        readiness,
        activation_error.is_some(),
    );
    let install_scope = installed.install_scope;
    let summary = installed.summary;
    let has_external_channel_surface = has_external_channel_surface(&summary);
    let runtime = summary.runtime_kind.runtime_wire_name().to_string();
    let connected = if has_external_channel_surface {
        connections.get(summary.package_ref.id.as_str()).copied()
    } else {
        None
    };
    let account_state = account_states.get(summary.package_ref.id.as_str());
    // Personal credential state is authoritative only for connection recipes
    // that actually bind a product-auth account. Host-generated-code channels
    // derive readiness from their identity binding and must not be made
    // setup-needed merely because the generic account reader has no row.
    let requires_personal_account = channel_requires_personal_account(&summary);
    let requires_personal_binding = channel_requires_personal_binding(&summary);
    let projected_account = if requires_personal_account || requires_personal_binding {
        projected_channel_account(
            connected,
            requires_personal_account.then_some(account_state).flatten(),
        )
    } else {
        None
    };
    let channel_unconnected = has_external_channel_surface
        && ((requires_personal_binding && connected != Some(true))
            || (requires_personal_account
                && projected_account
                    .as_ref()
                    .is_some_and(|(state, _)| *state != AuthAccountState::Connected)));
    let user_active = phase == LifecyclePublicState::Active
        && readiness != ExtensionCredentialReadiness::MissingRequired
        && !channel_unconnected
        && activation_error.is_none();
    let auth_accounts = vendor_auth_accounts(&summary, projected_account);
    let resolved_account_id = auth_accounts
        .first()
        .and_then(|vendor| vendor.accounts.first())
        .map(|account| account.account_id.clone());
    let surfaces = wire_surfaces(&summary, resolved_account_id);
    RebornExtensionInfo {
        package_ref: summary.package_ref,
        display_name: summary.name,
        runtime,
        description: summary.description,
        tools: summary.visible_capability_ids,
        installation_state: if user_active {
            LifecyclePublicState::Active
        } else {
            LifecyclePublicState::SetupNeeded
        },
        activation_error,
        version: Some(summary.version),
        onboarding: onboarding.onboarding,
        auth_accounts,
        surfaces,
        install_scope,
    }
}

/// Wire surfaces for a lifecycle summary: tool/auth pass through; the
/// channel surface carries typed direction, the caller's connection state
/// (when a connections map applies), and the connect affordance.
fn wire_surfaces(
    summary: &LifecycleExtensionSummary,
    resolved_account_id: Option<String>,
) -> Vec<RebornExtensionSurface> {
    summary
        .surface_kinds
        .iter()
        .filter_map(|kind| match kind {
            CapabilitySurfaceKind::Tool => Some(RebornExtensionSurface::Tool),
            CapabilitySurfaceKind::Auth => Some(RebornExtensionSurface::Auth),
            CapabilitySurfaceKind::Channel => Some(RebornExtensionSurface::Channel {
                inbound: summary
                    .channel_directions
                    .map(|directions| directions.inbound)
                    .unwrap_or(false),
                outbound: summary
                    .channel_directions
                    .map(|directions| directions.outbound)
                    .unwrap_or(false),
                // Length ≤ 1 today: the surface resolves to its vendor's single
                // account through the default binding (ADR 0001, shape only).
                binding_source: resolved_account_id
                    .as_ref()
                    .map(|_| RebornAccountBindingSource::Default),
                resolved_account_id: resolved_account_id.clone(),
                connection: summary.channel_connection.clone(),
            }),
            // Reserved kinds have no manifest section yet, so no wire form.
            CapabilitySurfaceKind::Trigger | CapabilitySurfaceKind::File => None,
        })
        .collect()
}

fn has_external_channel_surface(summary: &LifecycleExtensionSummary) -> bool {
    summary
        .surface_kinds
        .contains(&CapabilitySurfaceKind::Channel)
}

fn channel_requires_personal_account(summary: &LifecycleExtensionSummary) -> bool {
    summary
        .credential_requirements
        .iter()
        .any(|requirement| requirement.required)
        || summary
            .channel_connection
            .as_ref()
            .is_some_and(|connection| {
                connection.strategy == crate::RebornChannelConnectStrategy::OAuth
            })
}

fn channel_requires_personal_binding(summary: &LifecycleExtensionSummary) -> bool {
    summary
        .channel_connection
        .as_ref()
        .is_some_and(|connection| {
            matches!(
                connection.strategy,
                crate::RebornChannelConnectStrategy::WebGeneratedCode
                    | crate::RebornChannelConnectStrategy::OAuth
            )
        })
}

/// The credential-authority vendor a channel/auth surface binds. Prefers the
/// declared auth recipe vendor; falls back to the package id (today the two
/// real channel package ids equal their vendor ids).
fn channel_auth_vendor(summary: &LifecycleExtensionSummary) -> String {
    summary
        .credential_requirements
        .first()
        .map(|requirement| requirement.provider.clone())
        .unwrap_or_else(|| summary.package_ref.id.as_str().to_string())
}

/// Per-vendor accounts list for the extensions wire (overview §6.4, ADR 0001).
/// One vendor, at most one account today; the list shape is frozen so the
/// post-P7 multi-account follow-up lands without a wire break. When neither
/// the connection backfill nor a durable account/flow signal exists, there is
/// no per-caller account concept and therefore no vendor entry.
///
/// The account's state is the shared §6.3 machine, projected by
/// [`project_auth_account_state`] from the caller's durable auth-account signal
/// (`account_state`): a real credential-account status surfaces `expired` /
/// `refresh-failed` (with a typed `last_error`) and a live auth flow surfaces
/// `authenticating`. When the facade carries no richer status the connection
/// bool is the MIG-1 backfill — a live grant reads as a `configured` account
/// and projects `connected`.
fn vendor_auth_accounts(
    summary: &LifecycleExtensionSummary,
    projected_account: Option<(AuthAccountState, Option<AuthAccountLastError>)>,
) -> Vec<RebornVendorAuthAccounts> {
    let Some((state, last_error)) = projected_account else {
        return Vec::new();
    };
    let vendor = channel_auth_vendor(summary);
    vec![RebornVendorAuthAccounts {
        vendor: vendor.clone(),
        // One account per vendor today; its id is the vendor id until the
        // multi-account follow-up wires real per-account identity.
        accounts: vec![RebornAuthAccount {
            account_id: vendor,
            label: summary.name.clone(),
            state,
            last_error,
            is_default: true,
        }],
    }]
}

fn projected_channel_account(
    connected: Option<bool>,
    account_state: Option<&ChannelAuthAccountState>,
) -> Option<(AuthAccountState, Option<AuthAccountLastError>)> {
    if connected.is_none() && account_state.is_none() {
        return None;
    }
    let account_status = account_state
        .and_then(|state| state.account_status)
        .or_else(|| {
            (account_state.is_none() && connected == Some(true))
                .then_some(CredentialAccountStatus::Configured)
        });
    let active_flow_status = account_state.and_then(|state| state.active_flow_status);
    Some(project_auth_account_state(
        account_status,
        active_flow_status,
    ))
}

#[cfg(test)]
mod tests;
