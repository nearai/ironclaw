use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService, LifecycleProductSurfaceContext,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::{StreamExt, TryStreamExt, stream};
use ironclaw_auth::{
    AuthAccountLastError, AuthAccountState, ChannelAuthAccountState, ChannelConnectionService,
    CredentialAccountStatus, project_auth_account_state,
};
use ironclaw_extension_contracts::{
    state::{InstallationState, LifecyclePublicState},
    surface::CapabilitySurfaceKind,
};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceValidationCode,
};

use crate::{
    LifecycleExtensionSummary, LifecycleInstalledExtensionSummary, LifecycleProductAction,
    LifecycleProductPayload, LifecycleProductResponse, ProductView, RebornAccountBindingSource,
    RebornAuthAccount, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornExtensionSurface,
    RebornVendorAuthAccounts,
};

use super::{
    ExtensionCredentialSetupService,
    extension_credentials::{
        ExtensionCredentialReadiness, credential_scope, readiness_for_requirements,
    },
    extension_onboarding,
    lifecycle_setup::validation_error,
};

const EXTENSION_READINESS_CONCURRENCY: usize = 8;

pub const EXTENSIONS_VIEW: ProductView<serde_json::Value, RebornExtensionListResponse> =
    ProductView::unpaginated("extensions");

pub const EXTENSION_REGISTRY_VIEW: ProductView<serde_json::Value, RebornExtensionRegistryResponse> =
    ProductView::unpaginated("extension_registry");

pub(super) async fn list_extensions(
    service: Arc<dyn LifecycleProductService>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    channel_connection_service: Arc<dyn ChannelConnectionService>,
    caller: ProductSurfaceCaller,
) -> Result<RebornExtensionListResponse, ProductSurfaceError> {
    let context = lifecycle_surface_context(caller.clone());
    let lifecycle = execute_lifecycle(
        service.as_ref(),
        context.clone(),
        LifecycleProductAction::ExtensionList,
    )
    .await?;
    // The web UI's browser-push channel is host infrastructure, not a
    // browse-and-install integration, so keep it out of the install UI while it
    // stays a working notification channel. Classified by id (see
    // `is_builtin_host_surface`), not by channel direction.
    let installed = lifecycle_installed_extensions(&lifecycle)
        .into_iter()
        .filter(|extension| !is_builtin_host_surface(&extension.summary))
        .collect::<Vec<_>>();
    let connections = channel_connection_service
        .caller_channel_connections(caller.clone())
        .await?;
    // Per-caller auth-account status per channel vendor: lets each account
    // project its real §6.3 state (expired / refresh-failed) instead of the
    // connected/disconnected collapse the connection bool alone permits.
    let account_states = channel_connection_service
        .caller_channel_account_states(caller.clone())
        .await?;
    // Redacted per-extension activation errors from the durable installation
    // records, projected onto `RebornExtensionInfo::activation_error`.
    let activation_errors = service.installed_activation_errors(context).await?;
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
    service: &dyn LifecycleProductService,
    caller: ProductSurfaceCaller,
) -> Result<RebornExtensionRegistryResponse, ProductSurfaceError> {
    let context = lifecycle_surface_context(caller);
    let (installed_result, registry_result) = tokio::join!(
        execute_lifecycle(
            service,
            context.clone(),
            LifecycleProductAction::ExtensionList
        ),
        execute_lifecycle(
            service,
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
            .filter(|extension| !is_builtin_host_surface(&extension.summary))
            .cloned()
            .map(|extension| registry_entry(extension.summary, &installed_ids))
            .collect(),
    })
}

pub(super) async fn import_extension_capability(
    service: &dyn LifecycleProductService,
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
    service.import_extension_bundle(context, bundle).await?;
    Ok(())
}

async fn execute_lifecycle(
    service: &dyn LifecycleProductService,
    context: LifecycleProductContext,
    action: LifecycleProductAction,
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
    service.execute(context, action).await
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
    connections: HashMap<ExtensionId, bool>,
    account_states: HashMap<ExtensionId, ChannelAuthAccountState>,
    activation_errors: HashMap<ExtensionId, String>,
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

/// The host's own built-in surface — always present, not a browse-and-install
/// integration — is hidden from the install catalog even though it backs a
/// channel (the web UI's browser-push channel). This is an explicit id
/// classification rather than one inferred from channel direction, so it stays
/// correct as the web-app channel later gains inbound/outbound capabilities.
/// Naming the package dir here is allowed by `NON_VENDOR_PROVIDER_PACKAGE_DIRS`
/// (reborn_extension_specificity).
fn is_builtin_host_surface(summary: &LifecycleExtensionSummary) -> bool {
    matches!(summary.package_ref.id.as_str(), "web-push")
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

/// The caller's channel-connection verdict for one lifecycle summary — shared
/// by the extensions card (`extension_info`) and the model-facing
/// communication context so the two can never diverge on what "connected for
/// this caller" means.
pub(crate) struct CallerChannelConnection {
    /// The caller's projected §6.3 vendor account, when the channel binds one.
    pub(crate) projected_account: Option<(AuthAccountState, Option<AuthAccountLastError>)>,
    /// A channel surface the calling user has not personally connected — via
    /// the vendor's OAuth or a pairing/proof-code binding. Always `false` for
    /// non-channel extensions and channels that need no personal connection.
    pub(crate) channel_unconnected: bool,
}

/// Compute [`CallerChannelConnection`] from the caller's per-channel
/// connection and auth-account maps.
///
/// Absence of a connection signal is NOT consent: a pairing/OAuth channel
/// with no binding row reads as unconnected (`connected != Some(true)`),
/// because the generic connections map is populated per discovered channel
/// and a missing entry means "no proof this caller connected", not
/// "connected".
pub(crate) fn caller_channel_connection(
    summary: &LifecycleExtensionSummary,
    connections: &HashMap<ExtensionId, bool>,
    account_states: &HashMap<ExtensionId, ChannelAuthAccountState>,
) -> CallerChannelConnection {
    let has_external_channel_surface = has_external_channel_surface(summary);
    // Maps are keyed by `ExtensionId`; a package id that is not valid
    // extension vocabulary cannot be a key, so it reports no connection and no
    // account state rather than being looked up as a string that could never
    // have matched.
    let extension_id = ExtensionId::new(summary.package_ref.id.as_str()).ok();
    let connected = if has_external_channel_surface {
        extension_id
            .as_ref()
            .and_then(|id| connections.get(id))
            .copied()
    } else {
        None
    };
    let account_state = extension_id.as_ref().and_then(|id| account_states.get(id));
    let requires_personal_account = channel_requires_personal_account(summary);
    let requires_personal_binding = channel_requires_personal_binding(summary);
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
    CallerChannelConnection {
        projected_account,
        channel_unconnected,
    }
}

/// Per-caller authentication verdict for one installed extension, composed
/// from the same primitives the extensions card projects into
/// `installation_state` (`caller_public_state`): required-credential
/// readiness plus, for channel surfaces, the caller's personal
/// connection/binding proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallerExtensionAuth {
    /// Every required credential is configured for this caller and any
    /// personal channel connection is proven. Includes extensions that
    /// require nothing per-caller.
    Authenticated,
    /// The caller is missing a required credential or has not personally
    /// connected a channel surface that needs it.
    Unauthenticated,
    /// The verdict cannot be established: a needed readiness/connection port
    /// is unavailable or a lookup failed. Callers must claim nothing.
    Unknown,
}

/// The caller's per-channel connection + auth-account maps, borrowed
/// together (as returned by `ChannelConnectionService`'s two lookups).
pub(crate) type CallerChannelMaps<'a> = (
    &'a HashMap<ExtensionId, bool>,
    &'a HashMap<ExtensionId, ChannelAuthAccountState>,
);

/// Compute the caller's auth verdict for one installed extension.
///
/// `channel_connections` carries the caller's per-channel connection and
/// auth-account maps; `None` means connection data is unavailable, which is
/// distinct from empty maps ("no connections"). Fail-closed: absence of proof
/// where proof is required yields `Unauthenticated`; inability to check
/// yields `Unknown`, never a fabricated positive.
pub(crate) async fn caller_extension_auth(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_connections: Option<CallerChannelMaps<'_>>,
    caller: &ProductSurfaceCaller,
    installed: &LifecycleInstalledExtensionSummary,
) -> CallerExtensionAuth {
    let readiness =
        match credential_readiness_for_extension(extension_credentials, caller, installed).await {
            Ok(readiness) => readiness,
            Err(error) => {
                tracing::debug!(
                    extension = %installed.summary.package_ref.id.as_str(),
                    status_code = error.status_code,
                    "credential readiness lookup failed; extension auth verdict is unknown"
                );
                return CallerExtensionAuth::Unknown;
            }
        };
    let summary = &installed.summary;
    let needs_personal_channel_proof = has_external_channel_surface(summary)
        && (channel_requires_personal_account(summary)
            || channel_requires_personal_binding(summary));
    let channel_unconnected = if needs_personal_channel_proof {
        match channel_connections {
            Some((connections, account_states)) => {
                caller_channel_connection(summary, connections, account_states).channel_unconnected
            }
            // Proof is required but no connection data exists to prove it —
            // the verdict is unknowable, not "connected".
            None => return CallerExtensionAuth::Unknown,
        }
    } else {
        false
    };
    match readiness {
        ExtensionCredentialReadiness::Unknown => CallerExtensionAuth::Unknown,
        ExtensionCredentialReadiness::MissingRequired => CallerExtensionAuth::Unauthenticated,
        ExtensionCredentialReadiness::Configured | ExtensionCredentialReadiness::NotRequired => {
            if channel_unconnected {
                CallerExtensionAuth::Unauthenticated
            } else {
                CallerExtensionAuth::Authenticated
            }
        }
    }
}

fn extension_info(
    installed: LifecycleInstalledExtensionSummary,
    readiness: ExtensionCredentialReadiness,
    connections: &HashMap<ExtensionId, bool>,
    account_states: &HashMap<ExtensionId, ChannelAuthAccountState>,
    activation_errors: &HashMap<ExtensionId, String>,
) -> RebornExtensionInfo {
    let phase = installed.phase;
    let onboarding =
        extension_onboarding::for_installed_with_credential_status(&installed, readiness);
    let install_scope = installed.install_scope;
    let summary = installed.summary;
    let runtime = summary.runtime_kind.runtime_wire_name().to_string();
    // The per-extension maps are keyed by `ExtensionId`; a package id that is
    // not valid extension vocabulary cannot be a key in any of them, so it
    // reports no connection, no account state, and no activation error rather
    // than being looked up as a string that could never have matched.
    let extension_id = ExtensionId::new(summary.package_ref.id.as_str()).ok();
    // Redacted activation error for this extension (host installation record's
    // typed `last_error`), threaded onto the card slot the frontend already
    // renders. `None` when the service surfaces no failure for this extension.
    let activation_error = extension_id
        .as_ref()
        .and_then(|id| activation_errors.get(id).cloned());
    let CallerChannelConnection {
        projected_account,
        channel_unconnected,
    } = caller_channel_connection(&summary, connections, account_states);
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
        installation_state: caller_public_state(
            phase,
            readiness,
            channel_unconnected,
            activation_error.is_some(),
        ),
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

/// Does this channel bind a personal product-auth *account* (an OAuth grant or
/// a required personal credential)? Only those recipes have an account whose
/// §6.3 state can say `expired` / `refresh-failed`.
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

/// Does this channel bind a personal *identity binding* (pairing proof code or
/// OAuth)? Host-generated-code channels such as a proof-code pairing lane have
/// no product-auth account at all — their readiness is the binding alone, so
/// they must not be judged by the account reader (which has no row for them).
fn channel_requires_personal_binding(summary: &LifecycleExtensionSummary) -> bool {
    summary
        .channel_connection
        .as_ref()
        .is_some_and(|connection| {
            // Exhaustive on purpose: a new strategy must not silently default
            // to "no personal binding required", which reads as Active for a
            // caller who never connected. No wildcard arm -- adding a variant
            // is a compile error until someone classifies it.
            match connection.strategy {
                // Per-user proofs: the caller personally completes these, so
                // an extension is not connected for them until they do.
                crate::RebornChannelConnectStrategy::WebGeneratedCode
                | crate::RebornChannelConnectStrategy::OAuth
                | crate::RebornChannelConnectStrategy::InboundProofCode
                | crate::RebornChannelConnectStrategy::QrCode => true,
                // Operator-configured for the whole tenant; there is no
                // per-user binding to wait on.
                crate::RebornChannelConnectStrategy::AdminManagedChannels => false,
            }
        })
}

/// The caller-scoped public lifecycle verdict (§6.1).
///
/// The host checkpoint is only one input. An extension is `active` for *this*
/// caller only when the host is serving it AND the caller's required
/// credentials are present AND the caller has connected/paired every channel
/// surface that needs it AND activation did not fail. Anything else is
/// `setup_needed`, which is what drives the Configure/Connect affordance.
///
/// Fails closed: every unproven input yields `setup_needed`, never `active`.
fn caller_public_state(
    phase: InstallationState,
    readiness: ExtensionCredentialReadiness,
    channel_unconnected: bool,
    activation_failed: bool,
) -> LifecyclePublicState {
    let caller_ready = readiness != ExtensionCredentialReadiness::MissingRequired
        && !channel_unconnected
        && !activation_failed;
    match LifecyclePublicState::from_host_checkpoint(phase) {
        LifecyclePublicState::Active if caller_ready => LifecyclePublicState::Active,
        LifecyclePublicState::Uninstalled => LifecyclePublicState::Uninstalled,
        _ => LifecyclePublicState::SetupNeeded,
    }
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
/// `authenticating`. When the service carries no richer status the connection
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

/// Project the caller's channel account onto the shared §6.3 state machine.
/// `None` when there is no per-caller account concept at all (neither a
/// connection signal nor a durable account/flow row) — that is distinct from a
/// known-disconnected account and must not fabricate a vendor entry.
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
            // MIG-1 backfill: a live grant with no richer durable status reads
            // as a `configured` account. Only applied when the service carried
            // no account row at all, so a real `expired` status always wins.
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
mod tests {
    use std::{
        collections::HashSet,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use ironclaw_auth::{
        CredentialAccountId, CredentialAccountLabel, CredentialAccountProjection,
        CredentialOwnership,
    };
    use ironclaw_extension_contracts::surface::CapabilitySurfaceKind;
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};

    use super::*;
    use crate::reborn_services::StaticChannelConnectionService;
    use crate::{
        ChannelConnectionRequirement, ExtensionCredentialStatusRequest,
        ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialRequirement,
        LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding,
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
        LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
        LifecycleSearchExtensionSummary, RebornChannelConnectStrategy,
    };
    use ironclaw_auth::ChannelConnectionService;
    use ironclaw_product_contracts::surface::{
        ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    };

    #[derive(Default)]
    struct TestConnections {
        connections: std::collections::HashMap<ExtensionId, bool>,
    }

    impl TestConnections {
        fn with_connections(entries: &[(&str, bool)]) -> Self {
            Self {
                connections: entries
                    .iter()
                    .map(|(key, value)| {
                        (
                            ExtensionId::new(*key).expect("test key is valid extension id"),
                            *value,
                        )
                    })
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl ChannelConnectionService for TestConnections {
        async fn caller_channel_connections(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<std::collections::HashMap<ExtensionId, bool>, ProductSurfaceError> {
            Ok(self.connections.clone())
        }
    }

    fn no_channel_connections() -> Arc<dyn ChannelConnectionService> {
        Arc::new(TestConnections::default())
    }

    fn channel_connections(entries: &[(&str, bool)]) -> Arc<dyn ChannelConnectionService> {
        Arc::new(TestConnections::with_connections(entries))
    }

    #[tokio::test]
    async fn static_channel_connection_service_fails_disconnect_closed() {
        let error = StaticChannelConnectionService
            .disconnect_channel_for_caller(caller(), &ExtensionId::new("slack").expect("valid id"))
            .await
            .expect_err("unwired disconnect must not report success");

        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn list_marks_active_credentialed_extension_unauthenticated_without_caller_account() {
        let service = ListingService {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_with_onboarding(),
                phase: InstallationState::Active,
                install_scope: None,
            },
        };
        let credentials = Arc::new(RecordingCredentials::default());
        let caller = caller();

        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials.clone();
        let response = list_extensions(
            Arc::new(service),
            Some(credentials_service),
            no_channel_connections(),
            caller.clone(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert_eq!(
            extension.installation_state,
            LifecyclePublicState::SetupNeeded,
            "credential readiness must be evaluated for the current caller"
        );

        let requests = credentials.status_requests.lock().expect("lock");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.scope.resource.tenant_id, caller.tenant_id);
        assert_eq!(request.scope.resource.user_id, caller.user_id);
        assert_eq!(request.scope.resource.agent_id, caller.agent_id);
        assert_eq!(request.scope.resource.project_id, caller.project_id);
        assert_eq!(request.provider.as_str(), "fixture");
        assert_eq!(request.requester_extension.as_str(), "fixture");
    }

    /// A durable credential account that exists but is not `Configured`
    /// (expired, revoked, refresh-failed, missing secret material) is NOT
    /// readiness. Projecting the mere existence of an account row as
    /// "configured" makes a broken integration look callable and hides the
    /// reconnect affordance the caller needs.
    #[tokio::test]
    async fn nonconfigured_durable_credential_status_overrides_active_runtime_checkpoint() {
        for account_status in [
            CredentialAccountStatus::Revoked,
            CredentialAccountStatus::Expired,
            CredentialAccountStatus::RefreshFailed,
            CredentialAccountStatus::Missing,
            CredentialAccountStatus::Inactive,
            CredentialAccountStatus::PendingSetup,
        ] {
            let service = ListingService {
                extension: LifecycleInstalledExtensionSummary {
                    summary: summary_with_onboarding(),
                    phase: InstallationState::Active,
                    install_scope: None,
                },
            };
            let credentials: Arc<dyn ExtensionCredentialSetupService> =
                Arc::new(RecordingCredentials::with_account_status(account_status));

            let response = list_extensions(
                Arc::new(service),
                Some(credentials),
                no_channel_connections(),
                caller(),
            )
            .await
            .expect("list extensions");
            let extension = response.extensions.first().expect("one extension");

            assert_eq!(
                extension.installation_state,
                LifecyclePublicState::SetupNeeded,
                "{account_status:?} personal auth must prevent active projection"
            );
        }
    }

    #[tokio::test]
    async fn list_preserves_lifecycle_state_when_credential_status_is_retryably_unavailable() {
        let service = ListingService {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_with_onboarding(),
                phase: InstallationState::Active,
                install_scope: None,
            },
        };
        let credentials = UnavailableCredentials;

        let response = list_extensions(
            Arc::new(service),
            Some(Arc::new(credentials)),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert_eq!(
            extension.installation_state,
            LifecyclePublicState::Active,
            "retryable status outages should not be projected as missing credentials",
        );
    }

    #[tokio::test]
    async fn list_marks_active_host_managed_credential_extension_ready_without_setup_prompt() {
        let service = ListingService {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_without_browser_setup_credentials(),
                phase: InstallationState::Active,
                install_scope: None,
            },
        };
        let credentials = Arc::new(RecordingCredentials::default());
        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials.clone();

        let response = list_extensions(
            Arc::new(service),
            Some(credentials_service),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert_eq!(extension.installation_state, LifecyclePublicState::Active);
        assert!(extension.onboarding.is_none());
        assert!(
            credentials.status_requests.lock().expect("lock").is_empty(),
            "host-managed credentials must not trigger browser credential status checks"
        );
    }

    #[tokio::test]
    async fn list_checks_extension_readiness_with_bounded_concurrency() {
        let service = MultiListingService {
            extensions: (0..EXTENSION_READINESS_CONCURRENCY + 3)
                .map(|index| LifecycleInstalledExtensionSummary {
                    summary: summary_with_onboarding_for(&format!("fixture-{index}")),
                    phase: InstallationState::Active,
                    install_scope: None,
                })
                .collect(),
        };
        let credentials = Arc::new(ConcurrentCredentials::default());
        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials.clone();

        let response = list_extensions(
            Arc::new(service),
            Some(credentials_service),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");

        assert_eq!(
            response.extensions.len(),
            EXTENSION_READINESS_CONCURRENCY + 3
        );
        assert!(
            credentials.max_active.load(Ordering::SeqCst) > 1,
            "readiness checks should not run as a serialized page-load path"
        );
        assert!(
            credentials.max_active.load(Ordering::SeqCst) <= EXTENSION_READINESS_CONCURRENCY,
            "readiness checks must stay bounded"
        );
    }

    #[test]
    fn product_adapter_surface_projects_channel_kind() {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];

        let entry = registry_entry(summary, &HashSet::new());

        assert!(
            entry
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. })),
            "{:?}",
            entry.surfaces
        );
    }

    #[test]
    fn runtime_wire_names_are_implementation_labels_not_taxonomy() {
        // The wire carries the honest runtime name; product taxonomy travels
        // in `surfaces`, so runtime never masquerades as a product kind.
        assert_eq!(
            LifecycleExtensionRuntimeKind::WasmTool.runtime_wire_name(),
            "wasm"
        );
        assert_eq!(
            LifecycleExtensionRuntimeKind::McpServer.runtime_wire_name(),
            "mcp"
        );
        assert_eq!(
            LifecycleExtensionRuntimeKind::Script.runtime_wire_name(),
            "script"
        );

        // A channel-surface extension keeps its runtime label AND projects
        // the channel surface — two separate axes.
        let mut channel_summary = summary_with_onboarding();
        channel_summary.runtime_kind = LifecycleExtensionRuntimeKind::WasmTool;
        channel_summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        assert_eq!(
            channel_summary.runtime_kind.runtime_wire_name(),
            "wasm",
            "runtime label is unchanged by the channel surface"
        );
        assert!(
            wire_surfaces(&channel_summary, None)
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. })),
            "the channel surface projects alongside the runtime label"
        );
    }

    #[tokio::test]
    async fn list_projects_external_channel_surface_kind_through_extension_info() {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        summary.credential_requirements = Vec::new();
        let service = ListingService {
            extension: LifecycleInstalledExtensionSummary {
                summary,
                phase: InstallationState::Active,
                install_scope: None,
            },
        };

        let response = list_extensions(Arc::new(service), None, no_channel_connections(), caller())
            .await
            .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert!(
            extension
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. })),
            "{:?}",
            extension.surfaces
        );

        // Both personal-connection recipes: the vendor-OAuth channel and the
        // host-generated proof-code (pairing) channel Telegram uses. Each takes
        // a different branch of the `channel_unconnected` predicate, and both
        // must keep the caller in `setup_needed` until they connect.
        for strategy in [
            RebornChannelConnectStrategy::OAuth,
            RebornChannelConnectStrategy::WebGeneratedCode,
        ] {
            let mut unconnected_summary = summary_with_onboarding();
            unconnected_summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            unconnected_summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            unconnected_summary.credential_requirements = Vec::new();
            unconnected_summary.channel_connection = Some(test_channel_connection(strategy));
            let unconnected = list_extensions(
                Arc::new(ListingService {
                    extension: LifecycleInstalledExtensionSummary {
                        summary: unconnected_summary,
                        // The host is serving the extension: only the caller's
                        // missing personal connection separates this from ready.
                        phase: InstallationState::Active,
                        install_scope: None,
                    },
                }),
                None,
                channel_connections(&[("fixture", false)]),
                caller(),
            )
            .await
            .expect("list extensions");
            let unconnected_extension = unconnected.extensions.first().expect("one");
            assert_eq!(
                unconnected_extension.installation_state,
                LifecyclePublicState::SetupNeeded,
                "an unconnected {strategy:?} channel must keep the setup affordance visible",
            );
            // The Extensions card reads exactly one field — `installation_state`
            // (`extension-actions.ts::extensionLifecycleState`, pinned by
            // `assets.rs::extension_configuration_uses_only_the_public_lifecycle_states`).
            // Asserted on the serialized wire form because that is the contract
            // the browser actually consumes. #6616 regressed this to `active`,
            // which rendered an unpaired Telegram card as ready to use.
            assert_eq!(
                serde_json::to_value(unconnected_extension)
                    .expect("extension info serializes")
                    .get("installation_state")
                    .and_then(serde_json::Value::as_str),
                Some("setup_needed"),
                "an unpaired {strategy:?} channel extension must not be presented \
                 to the user as ready to use",
            );

            // Absence of a connection signal is NOT consent. A pairing/OAuth
            // channel with no row in the connections map has produced no proof
            // the caller connected, so it must read unconnected. Without this
            // arm the predicate can be weakened from `connected != Some(true)`
            // to `connected == Some(false)` and every test still passes -- which
            // is exactly the shape #6616 shipped.
            let mut absent_summary = summary_with_onboarding();
            absent_summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            absent_summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            absent_summary.credential_requirements = Vec::new();
            absent_summary.channel_connection = Some(test_channel_connection(strategy));
            let absent = list_extensions(
                Arc::new(ListingService {
                    extension: LifecycleInstalledExtensionSummary {
                        summary: absent_summary,
                        phase: InstallationState::Active,
                        install_scope: None,
                    },
                }),
                None,
                no_channel_connections(),
                caller(),
            )
            .await
            .expect("list extensions");
            assert_eq!(
                absent.extensions.first().expect("one").installation_state,
                LifecyclePublicState::SetupNeeded,
                "no connection signal for a {strategy:?} channel must read unconnected, never silently active",
            );

            // ...and once the caller connects, the same extension goes active.
            let mut connected_summary = summary_with_onboarding();
            connected_summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            connected_summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            connected_summary.credential_requirements = Vec::new();
            connected_summary.channel_connection = Some(test_channel_connection(strategy));
            let connected = list_extensions(
                Arc::new(ListingService {
                    extension: LifecycleInstalledExtensionSummary {
                        summary: connected_summary,
                        phase: InstallationState::Active,
                        install_scope: None,
                    },
                }),
                None,
                channel_connections(&[("fixture", true)]),
                caller(),
            )
            .await
            .expect("list extensions");
            assert_eq!(
                connected
                    .extensions
                    .first()
                    .expect("one")
                    .installation_state,
                LifecyclePublicState::Active,
                "a connected {strategy:?} channel must project active",
            );
        }
    }

    fn test_channel_connection(
        strategy: RebornChannelConnectStrategy,
    ) -> ChannelConnectionRequirement {
        ChannelConnectionRequirement {
            channel: "fixture".to_string(),
            display_name: "Fixture Messenger".to_string(),
            strategy,
            instructions: "Connect Fixture Messenger.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect".to_string(),
            error_message: "Connection failed.".to_string(),
        }
    }

    #[tokio::test]
    async fn list_extension_registry_projects_external_channel_kind_and_installed_status_from_webui_caller()
     {
        let caller = caller();
        let installed_summary = {
            let mut summary = summary_with_onboarding_for("installed-fixture");
            summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            summary
        };
        let registry_installed_summary = installed_summary.clone();
        let registry_uninstalled_summary = {
            let mut summary = summary_with_onboarding_for("uninstalled-fixture");
            summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            summary
        };
        let service = RegistryListingService {
            installed: LifecycleInstalledExtensionSummary {
                summary: installed_summary,
                phase: InstallationState::Active,
                install_scope: None,
            },
            registry: vec![
                search_extension_summary(registry_installed_summary),
                search_extension_summary(registry_uninstalled_summary),
            ],
            calls: Mutex::new(Vec::new()),
        };

        let response = list_extension_registry(&service, caller.clone())
            .await
            .expect("registry response");

        assert_eq!(response.entries.len(), 2);

        let installed_entry = response
            .entries
            .iter()
            .find(|entry| entry.package_ref.id.as_str() == "installed-fixture")
            .expect("installed entry");
        assert!(
            installed_entry
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. }))
        );
        assert!(installed_entry.installed);

        let uninstalled_entry = response
            .entries
            .iter()
            .find(|entry| entry.package_ref.id.as_str() == "uninstalled-fixture")
            .expect("uninstalled entry");
        assert!(
            uninstalled_entry
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. }))
        );
        assert!(!uninstalled_entry.installed);

        let calls = service.calls.lock().expect("lock");
        assert_eq!(calls.len(), 2);
        for (context, action) in calls.iter() {
            match action {
                LifecycleProductAction::ExtensionList => {}
                LifecycleProductAction::ExtensionSearch { query } => {
                    assert!(query.is_empty(), "registry search uses the empty query");
                }
                other => panic!("unexpected lifecycle action: {other:?}"),
            }
            match context {
                LifecycleProductContext::Surface(surface) => {
                    assert_eq!(surface.tenant_id, caller.tenant_id);
                    assert_eq!(surface.user_id, caller.user_id);
                    assert_eq!(surface.agent_id, caller.agent_id);
                    assert_eq!(surface.project_id, caller.project_id);
                }
                other => panic!("unexpected lifecycle context: {other:?}"),
            }
        }
    }

    #[derive(Default)]
    struct RecordingCredentials {
        status_requests: Mutex<Vec<ExtensionCredentialStatusRequest>>,
        account_status: Option<CredentialAccountStatus>,
    }

    impl RecordingCredentials {
        fn with_account_status(account_status: CredentialAccountStatus) -> Self {
            Self {
                account_status: Some(account_status),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for RecordingCredentials {
        async fn credential_status(
            &self,
            request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            let account = self
                .account_status
                .map(|status| CredentialAccountProjection {
                    id: CredentialAccountId::new(),
                    provider: request.provider.clone(),
                    label: CredentialAccountLabel::new("fixture account")
                        .expect("valid account label"),
                    status,
                    ownership: CredentialOwnership::UserReusable,
                    owner_extension: None,
                    granted_extensions: Vec::new(),
                    secret_handle_count: 1,
                });
            self.status_requests.lock().expect("lock").push(request);
            Ok(account)
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
            Ok(CredentialAccountId::new())
        }
    }

    struct UnavailableCredentials;

    #[async_trait]
    impl ExtensionCredentialSetupService for UnavailableCredentials {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            Err(ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            })
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
            Ok(CredentialAccountId::new())
        }
    }

    #[derive(Default)]
    struct ConcurrentCredentials {
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for ConcurrentCredentials {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
            Ok(CredentialAccountId::new())
        }
    }

    struct ListingService {
        extension: LifecycleInstalledExtensionSummary,
    }

    #[async_trait]
    impl LifecycleProductService for ListingService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            assert!(matches!(action, LifecycleProductAction::ExtensionList));
            Ok(LifecycleProductResponse {
                package_ref: None,
                phase: self.extension.phase,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: vec![self.extension.clone()],
                    count: 1,
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            panic!("list_extensions should execute the list action, not project one package")
        }
    }

    struct MultiListingService {
        extensions: Vec<LifecycleInstalledExtensionSummary>,
    }

    #[async_trait]
    impl LifecycleProductService for MultiListingService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            assert!(matches!(action, LifecycleProductAction::ExtensionList));
            Ok(LifecycleProductResponse {
                package_ref: None,
                phase: InstallationState::Active,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: self.extensions.clone(),
                    count: self.extensions.len(),
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            panic!("list_extensions should execute the list action, not project one package")
        }
    }

    struct RegistryListingService {
        installed: LifecycleInstalledExtensionSummary,
        registry: Vec<LifecycleSearchExtensionSummary>,
        calls: Mutex<Vec<(LifecycleProductContext, LifecycleProductAction)>>,
    }

    #[async_trait]
    impl LifecycleProductService for RegistryListingService {
        async fn execute(
            &self,
            context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            self.calls
                .lock()
                .expect("lock")
                .push((context.clone(), action.clone()));
            match action {
                LifecycleProductAction::ExtensionList => Ok(LifecycleProductResponse {
                    package_ref: None,
                    phase: self.installed.phase,
                    blockers: Vec::new(),
                    message: None,
                    payload: Some(LifecycleProductPayload::ExtensionList {
                        extensions: vec![self.installed.clone()],
                        count: 1,
                    }),
                }),
                LifecycleProductAction::ExtensionSearch { query } => {
                    assert!(query.is_empty(), "registry search uses the empty query");
                    Ok(LifecycleProductResponse {
                        package_ref: None,
                        phase: InstallationState::Active,
                        blockers: Vec::new(),
                        message: None,
                        payload: Some(LifecycleProductPayload::ExtensionSearch {
                            extensions: self.registry.clone(),
                            count: self.registry.len(),
                        }),
                    })
                }
                other => panic!("unexpected lifecycle action: {other:?}"),
            }
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            panic!("list_extension_registry should not project one package")
        }
    }

    fn caller() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new("tenant-alpha").expect("valid tenant"),
            UserId::new("user-alpha").expect("valid user"),
            Some(AgentId::new("agent-alpha").expect("valid agent")),
            Some(ProjectId::new("project-alpha").expect("valid project")),
        )
    }

    fn summary_with_onboarding() -> LifecycleExtensionSummary {
        summary_with_onboarding_for("fixture")
    }

    fn summary_with_onboarding_for(package_id: &str) -> LifecycleExtensionSummary {
        LifecycleExtensionSummary {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
                .expect("valid package ref"),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            description: "test extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
            surface_kinds: Vec::new(),
            channel_directions: None,
            channel_connection: None,
            channel_presentation: None,
            visible_capability_ids: Vec::new(),
            visible_read_only_capability_ids: Vec::new(),
            credential_requirements: vec![LifecycleExtensionCredentialRequirement {
                name: "fixture_token".to_string(),
                provider: "fixture".to_string(),
                required: true,
                setup: LifecycleExtensionCredentialSetup::ManualToken,
            }],
            onboarding: Some(LifecycleExtensionOnboarding {
                instructions: "Fixture needs a token before its tools can run.".to_string(),
                credential_instructions: Some("Paste the fixture token.".to_string()),
                setup_url: None,
                credential_next_step: Some(
                    "After saving the token, activate Fixture to publish its tools.".to_string(),
                ),
            }),
        }
    }

    fn summary_without_browser_setup_credentials() -> LifecycleExtensionSummary {
        LifecycleExtensionSummary {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai")
                .expect("valid package ref"),
            name: "NEAR AI".to_string(),
            version: "1.0.0".to_string(),
            description: "host-managed MCP extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: LifecycleExtensionRuntimeKind::McpServer,
            surface_kinds: Vec::new(),
            channel_directions: None,
            channel_connection: None,
            channel_presentation: None,
            visible_capability_ids: vec!["nearai.web_search".to_string()],
            visible_read_only_capability_ids: vec!["nearai.web_search".to_string()],
            credential_requirements: Vec::new(),
            onboarding: Some(LifecycleExtensionOnboarding {
                instructions: "NEAR AI MCP uses the NEAR AI credentials configured for the assistant. If NEAR AI is not configured yet, add a NEAR AI API key in assistant inference settings before activating this extension."
                    .to_string(),
                credential_instructions: Some(
                    "Configure NEAR AI for the assistant with an API key; MCP reuses that credential."
                        .to_string(),
                ),
                setup_url: None,
                credential_next_step: Some(
                    "After NEAR AI is configured for the assistant, activate NEAR AI MCP to publish its tools."
                        .to_string(),
                ),
            }),
        }
    }

    fn search_extension_summary(
        summary: LifecycleExtensionSummary,
    ) -> LifecycleSearchExtensionSummary {
        LifecycleSearchExtensionSummary {
            summary,
            installation_phase: None,
        }
    }
}
