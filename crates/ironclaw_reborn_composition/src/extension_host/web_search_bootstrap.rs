//! Auto-activate the real, credentialed `web_search` (Brave Search API) tool
//! when a `BRAVE_API_KEY` is configured, mirroring
//! `web_access_bootstrap::bootstrap_web_access`'s install-then-activate
//! pattern but gated on a real credential the way `bootstrap_nearai_mcp` is.
//!
//! Why this exists: `web_search` is a fully-built, versioned, first-party-
//! published WASM tool (`registry/tools/web_search.json`, tagged `default`)
//! that calls the real Brave Search API — it already has its own capability
//! manifest declaring the `brave_api_key` credential injection and the
//! `api.search.brave.com` egress allowlist. It is not a stub. But like
//! `web-access` before #6232, nothing points a session's model at the
//! extension catalog, so an operator who sets `BRAVE_API_KEY` in their
//! environment never sees it actually used — the key sits unconsumed and the
//! model never discovers a working search tool exists.
//!
//! `web_search`'s model-facing tool name is `web_search` — the same display
//! name `web-access.search` is aliased to (see
//! `ironclaw_runner::tool_disclosure`). Only one of the two can be active at
//! once without a name collision, so `build_local_runtime` calls this
//! bootstrap INSTEAD OF `bootstrap_web_access` when `BRAVE_API_KEY` is
//! present: a real, quota'd, credentialed search backend beats Exa's
//! zero-config but rate-shared free MCP tier whenever an operator has
//! actually provisioned one.

use std::sync::Arc;

use ironclaw_host_api::{SecretHandle, UserId};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase};
use secrecy::SecretString;

use crate::RebornBuildError;
use crate::admin_secrets::AdminSecretProvisioner;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, RebornLocalExtensionManagementPort,
};

const WEB_SEARCH_EXTENSION_ID: &str = "web_search";
const BRAVE_API_KEY_SECRET_NAME: &str = "brave_api_key";
pub(crate) const BRAVE_API_KEY_ENV_VAR: &str = "BRAVE_API_KEY";

/// Read `BRAVE_API_KEY` from the process environment. Split out from
/// [`bootstrap_web_search_brave`] so the bootstrap logic itself takes the key
/// as a plain parameter and stays hermetically testable — no process-global
/// env mutation required in tests, matching `nearai_mcp`'s
/// config-resolved-by-the-caller shape.
pub(crate) fn web_search_bootstrap_api_key_from_env() -> Option<SecretString> {
    ironclaw_common::env_helpers::env_or_override(BRAVE_API_KEY_ENV_VAR)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSearchBootstrapOutcome {
    /// No `BRAVE_API_KEY` in the environment — nothing to do, and
    /// `web-access` (Exa) should be bootstrapped instead.
    NotConfigured,
    /// A prior boot (or the user) explicitly removed/disabled `web_search`;
    /// that choice is preserved rather than silently overridden.
    SkippedPreservedRemoved,
    SkippedDisabled,
    SkippedNonActivatable,
    AlreadyActive,
    Activated,
}

impl WebSearchBootstrapOutcome {
    /// Whether `web-access` (Exa) must stay off to avoid a `web_search`
    /// display-name collision. Only `NotConfigured` leaves room for it.
    pub(crate) fn leaves_web_access_available(self) -> bool {
        matches!(self, Self::NotConfigured)
    }

    pub(crate) fn log_completion(self) {
        match self {
            Self::NotConfigured => {
                tracing::debug!("web_search (Brave) bootstrap is not configured")
            }
            Self::SkippedPreservedRemoved | Self::SkippedDisabled | Self::SkippedNonActivatable => {
                tracing::debug!(
                    outcome = ?self,
                    "web_search (Brave) bootstrap skipped; extension will not be auto-activated"
                );
            }
            Self::AlreadyActive | Self::Activated => {
                tracing::debug!(outcome = ?self, "web_search (Brave) bootstrap completed");
            }
        }
    }
}

/// Read `BRAVE_API_KEY` from the process environment, seed it into the
/// tenant/user's secret store (so the WASM tool's declared `brave_api_key`
/// credential injection resolves), then install (if not already) and
/// activate `web_search` for the runtime's tenant-operator identity — unless
/// the user already explicitly disabled or removed it.
pub(crate) async fn bootstrap_web_search_brave(
    api_key: Option<SecretString>,
    extension_management: &Arc<RebornLocalExtensionManagementPort>,
    admin_secret_provisioner: Option<&Arc<dyn AdminSecretProvisioner>>,
    owner_scope: &ironclaw_host_api::ResourceScope,
) -> Result<WebSearchBootstrapOutcome, RebornBuildError> {
    let Some(api_key) = api_key else {
        return Ok(WebSearchBootstrapOutcome::NotConfigured);
    };

    let caller: UserId = extension_management.tenant_operator_user_id().clone();
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, WEB_SEARCH_EXTENSION_ID)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("web_search package ref is invalid: {error}"),
            })?;

    let phase = extension_management
        .project(package_ref.clone(), &caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web_search extension projection failed: {error}"),
        })?
        .phase;

    match phase {
        LifecyclePhase::Discovered | LifecyclePhase::Installed => {}
        LifecyclePhase::Active => return Ok(WebSearchBootstrapOutcome::AlreadyActive),
        LifecyclePhase::Removed => {
            tracing::debug!(
                "web_search was explicitly removed; preserving that state rather than \
                 re-installing it"
            );
            return Ok(WebSearchBootstrapOutcome::SkippedPreservedRemoved);
        }
        LifecyclePhase::Disabled => {
            tracing::debug!(
                "web_search is explicitly disabled; preserving that state rather than \
                 re-activating it"
            );
            return Ok(WebSearchBootstrapOutcome::SkippedDisabled);
        }
        other => {
            tracing::debug!(
                phase = ?other,
                "web_search is not in an auto-activatable phase; skipping bootstrap"
            );
            return Ok(WebSearchBootstrapOutcome::SkippedNonActivatable);
        }
    }

    // Seed the secret before activation so the tool's declared credential
    // injection has something to lease. Best-effort: a provisioner is only
    // wired up on backends with durable secret-store crypto; if it's absent
    // we still install/activate (the tool itself fails closed per-call with
    // a clear "Brave API key not found" message rather than silently
    // pretending to work), so a missing provisioner is not fatal here.
    //
    // Provision under the TENANT-SHARED scope, not the bootstrap caller's own
    // (tenant, user) pair: `web_search` installs Tenant-owned (same as
    // `web-access`), and the host's dispatch-time credential pre-flight
    // (`secret_owner_scope`) only ever checks the model-invocation caller's
    // own scope, then falls back to `caller_scope.tenant_shared_managed_scope()`
    // — never an arbitrary third scope. Provisioning under the bootstrap's
    // own owner scope left the secret invisible to every real invocation
    // (dispatch-time pre-flight always reported it absent, surfacing
    // AuthRequired instead of ever calling Brave).
    if let Some(provisioner) = admin_secret_provisioner {
        let handle = SecretHandle::new(BRAVE_API_KEY_SECRET_NAME).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("brave_api_key secret handle is invalid: {error}"),
            }
        })?;
        let shared_scope = owner_scope.tenant_shared_managed_scope();
        provisioner
            .put(
                &shared_scope.tenant_id,
                &shared_scope.user_id,
                handle,
                api_key,
            )
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("brave_api_key secret provisioning failed: {error}"),
            })?;
    }

    if phase == LifecyclePhase::Discovered {
        extension_management
            .install(package_ref.clone(), &caller)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("web_search extension install failed: {error}"),
            })?;
    }
    extension_management
        .activate(package_ref, ExtensionActivationMode::Static, &caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web_search extension activation failed: {error}"),
        })?;
    Ok(WebSearchBootstrapOutcome::Activated)
}
