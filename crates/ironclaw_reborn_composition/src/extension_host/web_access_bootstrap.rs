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

use ironclaw_host_api::{InstallationState, UserId};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef};

use crate::RebornBuildError;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, RebornLocalExtensionManagementPort,
};

const WEB_ACCESS_EXTENSION_ID: &str = "web-access";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebAccessBootstrapOutcome {
    /// The user (or a prior boot) explicitly removed/disabled `web-access`;
    /// that choice is preserved rather than silently overridden.
    SkippedPreservedRemoved,
    SkippedDisabled,
    /// Phase wasn't in an auto-activatable state (e.g. a foreign-owned
    /// private install this caller can't see) — fail open, not an error.
    SkippedNonActivatable,
    AlreadyActive,
    Activated,
}

impl WebAccessBootstrapOutcome {
    pub(crate) fn log_completion(self) {
        match self {
            Self::SkippedPreservedRemoved | Self::SkippedDisabled | Self::SkippedNonActivatable => {
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
/// tenant-operator identity, unless the user already explicitly disabled or
/// removed it. Errors are returned to the caller (composition-time failures
/// here are a real bug, not a "credentials missing" skip — there is nothing
/// to configure), but only a genuine lifecycle-service failure returns
/// `Err`; every ordinary "already installed"/"user opted out" case is a
/// normal `Ok(WebAccessBootstrapOutcome)`.
pub(crate) async fn bootstrap_web_access(
    extension_management: &Arc<RebornLocalExtensionManagementPort>,
) -> Result<WebAccessBootstrapOutcome, RebornBuildError> {
    let caller: UserId = extension_management.tenant_operator_user_id().clone();
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, WEB_ACCESS_EXTENSION_ID)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("web-access package ref is invalid: {error}"),
            })?;

    let phase = extension_management
        .project(package_ref.clone(), &caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web-access extension projection failed: {error}"),
        })?
        .phase;

    match phase {
        // `Installed` also covers a never-installed package (`project()`
        // reports the neutral `Installed` when there is no installation
        // record at all — see its doc comment); `install()` below is
        // idempotent for an already-installed package, so both cases take
        // the same path unconditionally.
        InstallationState::Installed | InstallationState::Configured => {}
        InstallationState::Active => return Ok(WebAccessBootstrapOutcome::AlreadyActive),
        InstallationState::Removed => {
            tracing::debug!(
                "web-access was explicitly removed; preserving that state rather than \
                 re-installing it"
            );
            return Ok(WebAccessBootstrapOutcome::SkippedPreservedRemoved);
        }
        InstallationState::Disabled => {
            tracing::debug!(
                "web-access is explicitly disabled; preserving that state rather than \
                 re-activating it"
            );
            return Ok(WebAccessBootstrapOutcome::SkippedDisabled);
        }
        InstallationState::Failed | InstallationState::Unsupported => {
            tracing::debug!(
                phase = ?phase,
                "web-access is not in an auto-activatable phase; skipping bootstrap"
            );
            return Ok(WebAccessBootstrapOutcome::SkippedNonActivatable);
        }
    }

    // Idempotent for an already-installed package (see the match above), so
    // this always runs rather than branching on whether `phase` distinguishes
    // "never installed" from "installed" — it no longer does.
    extension_management
        .install(package_ref.clone(), &caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web-access extension install failed: {error}"),
        })?;
    extension_management
        .activate(package_ref, ExtensionActivationMode::Static, &caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web-access extension activation failed: {error}"),
        })?;
    Ok(WebAccessBootstrapOutcome::Activated)
}
