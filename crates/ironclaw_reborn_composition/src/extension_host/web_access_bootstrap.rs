//! Auto-activate the zero-config `web-access` extension at runtime
//! composition time, mirroring `llm_admin::nearai_mcp::bootstrap_nearai_mcp`'s
//! pattern (install-then-activate against the current lifecycle phase) but
//! simpler: `web-access` needs no credential/config, so there is no
//! credential-submit step and no durable-storage gate.
//!
//! Why this exists: `web-access` is a first-party, zero-config extension
//! (Exa-MCP-backed search + get_content, no API key) that is already
//! trust-policy-approved for local-dev/production composition — but nothing
//! in the base toolset ever points a session's model at the extension
//! catalog, so it is discoverable (`extension_search`/`extension_install`/
//! `extension_activate` are always in the model's core tool set — see
//! `ironclaw_runner::tool_disclosure::CORE_TOOL_NAMES`) but never actually
//! *discovered*. A production agent asked a question needing current
//! information has no signal that a web-search capability exists behind that
//! catalog, so it silently falls back to whatever raw tools it already has
//! (e.g. `http`) with no way to find a URL. Auto-activating removes that
//! discovery burden entirely, the same way `bootstrap_nearai_mcp` already
//! does for the `nearai` extension when NEAR AI credentials are configured.

use std::sync::Arc;

use ironclaw_product::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload, LifecyclePublicState,
    ProductWorkflowError,
};

use crate::RebornBuildError;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, ExtensionManagementPort,
};

const WEB_ACCESS_EXTENSION_ID: &str = "web-access";

/// Matches `AvailableExtensionCatalog::resolve`'s error text for a package id
/// with no catalog entry at all. `build_backend_production` composes this
/// bootstrap for every deployment shape it serves (local-dev, hosted
/// single-tenant, and narrower test/production fixtures that compose a
/// reduced first-party catalog without `web-access`) — treat "not in this
/// deployment's catalog" as a normal skip rather than a hard composition
/// failure.
const EXTENSION_NOT_FOUND_REASON: &str = "available extension was not found";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebAccessBootstrapOutcome {
    AlreadyActive,
    Activated,
    /// The extension is installed but not in an auto-activatable public
    /// state (e.g. the user explicitly disabled or removed it — the product
    /// contract deliberately collapses those into one non-`Active`,
    /// non-`Uninstalled` state, so this bootstrap can't distinguish them and
    /// errs on the side of not overriding whatever the user did).
    SkippedNonActivatable,
    /// `web-access` isn't in this deployment's available-extension catalog
    /// at all (a composition that never bundles it).
    SkippedUnavailable,
}

impl WebAccessBootstrapOutcome {
    pub(crate) fn log_completion(self) {
        match self {
            Self::SkippedNonActivatable | Self::SkippedUnavailable => {
                tracing::debug!(
                    outcome = ?self,
                    "web-access bootstrap skipped; extension will not be auto-activated"
                );
            }
            Self::AlreadyActive | Self::Activated => {
                tracing::debug!(outcome = ?self, "web-access bootstrap completed");
            }
        }
    }
}

/// Install (if not already) and activate `web-access` for the runtime's
/// owner scope, unless it's installed but not in an auto-activatable public
/// state. Errors are returned to the caller (composition-time failures here
/// are a real bug, not a "credentials missing" skip — there is nothing to
/// configure), but only a genuine lifecycle-service failure returns `Err`;
/// every ordinary "already installed"/"user opted out" case is a normal
/// `Ok(WebAccessBootstrapOutcome)`.
pub(crate) async fn bootstrap_web_access(
    extension_management: &Arc<ExtensionManagementPort>,
    owner_scope: &ironclaw_host_api::ResourceScope,
) -> Result<WebAccessBootstrapOutcome, RebornBuildError> {
    let caller = &owner_scope.user_id;
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, WEB_ACCESS_EXTENSION_ID)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("web-access package ref is invalid: {error}"),
            })?;

    let projection = match extension_management
        .project(package_ref.clone(), caller, None)
        .await
    {
        Ok(projection) => projection,
        Err(ProductWorkflowError::InvalidBindingRequest { reason })
            if reason == EXTENSION_NOT_FOUND_REASON =>
        {
            tracing::debug!("web-access is not in this deployment's extension catalog");
            return Ok(WebAccessBootstrapOutcome::SkippedUnavailable);
        }
        Err(error) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("web-access extension projection failed: {error}"),
            });
        }
    };
    let phase = projection.phase;
    // `install_scope` is present exactly when the caller has a visible
    // installation; the projected `phase` is a resting state only for an
    // installed package (a not-installed projection carries a neutral
    // phase) — same convention `bootstrap_nearai_mcp` uses.
    let installed = matches!(
        projection.payload.as_ref(),
        Some(LifecycleProductPayload::ExtensionList { extensions, .. })
            if extensions.first().and_then(|extension| extension.install_scope).is_some()
    );
    if installed {
        match phase {
            LifecyclePublicState::Active => return Ok(WebAccessBootstrapOutcome::AlreadyActive),
            LifecyclePublicState::SetupNeeded => {}
            LifecyclePublicState::Uninstalled => {
                tracing::debug!(
                    phase = ?phase,
                    "web-access is installed but projects as uninstalled; skipping bootstrap"
                );
                return Ok(WebAccessBootstrapOutcome::SkippedNonActivatable);
            }
        }
    }

    // Idempotent for an already-installed package, so this always runs
    // rather than branching on `installed` — install() no-ops if it's
    // already there.
    extension_management
        .install(package_ref.clone(), caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web-access extension install failed: {error}"),
        })?;
    extension_management
        .activate(package_ref, ExtensionActivationMode::Static, caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web-access extension activation failed: {error}"),
        })?;
    Ok(WebAccessBootstrapOutcome::Activated)
}
