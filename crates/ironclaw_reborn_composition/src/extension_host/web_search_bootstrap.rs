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

use ironclaw_host_api::SecretHandle;
use ironclaw_product::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload, LifecyclePublicState,
    ProductWorkflowError,
};
use secrecy::SecretString;

use crate::RebornBuildError;
use crate::admin_secrets::AdminSecretProvisioner;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, ExtensionManagementPort,
};

const WEB_SEARCH_EXTENSION_ID: &str = "web_search";
const BRAVE_API_KEY_SECRET_NAME: &str = "brave_api_key";
pub(crate) const BRAVE_API_KEY_ENV_VAR: &str = "BRAVE_API_KEY";

/// Matches `AvailableExtensionCatalog::resolve`'s error text for a package id
/// with no catalog entry at all. `build_backend_production` composes this
/// bootstrap for every deployment shape it serves (local-dev, hosted
/// single-tenant, and narrower test/production fixtures that compose a
/// reduced first-party catalog without `web_search`) — treat "not in this
/// deployment's catalog" as a normal skip rather than a hard composition
/// failure.
const EXTENSION_NOT_FOUND_REASON: &str = "available extension was not found";

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
    AlreadyActive,
    Activated,
    /// The extension is installed but not in an auto-activatable public
    /// state (e.g. the user explicitly disabled or removed it — the product
    /// contract deliberately collapses those into one non-`Active`,
    /// non-`Uninstalled` state, so this bootstrap can't distinguish them and
    /// errs on the side of not overriding whatever the user did).
    SkippedNonActivatable,
    /// `web_search` isn't in this deployment's available-extension catalog
    /// at all (a composition that never bundles it). `web-access` (Exa)
    /// should be bootstrapped instead, same as `NotConfigured`.
    SkippedUnavailable,
}

impl WebSearchBootstrapOutcome {
    /// Whether `web-access` (Exa) must stay off to avoid a `web_search`
    /// display-name collision. `NotConfigured`/`SkippedUnavailable` both
    /// leave room for it (no working Brave path either way).
    pub(crate) fn leaves_web_access_available(self) -> bool {
        matches!(self, Self::NotConfigured | Self::SkippedUnavailable)
    }

    pub(crate) fn log_completion(self) {
        match self {
            Self::NotConfigured => {
                tracing::debug!("web_search (Brave) bootstrap is not configured")
            }
            Self::SkippedNonActivatable | Self::SkippedUnavailable => {
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
/// activate `web_search` for the runtime's owner scope — unless it's
/// installed but not in an auto-activatable public state.
pub(crate) async fn bootstrap_web_search_brave(
    api_key: Option<SecretString>,
    extension_management: &Arc<ExtensionManagementPort>,
    admin_secret_provisioner: Option<&Arc<dyn AdminSecretProvisioner>>,
    owner_scope: &ironclaw_host_api::ResourceScope,
) -> Result<WebSearchBootstrapOutcome, RebornBuildError> {
    let Some(api_key) = api_key else {
        return Ok(WebSearchBootstrapOutcome::NotConfigured);
    };

    let caller = &owner_scope.user_id;
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, WEB_SEARCH_EXTENSION_ID)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("web_search package ref is invalid: {error}"),
            })?;

    let projection = match extension_management
        .project(package_ref.clone(), caller, None)
        .await
    {
        Ok(projection) => projection,
        Err(ProductWorkflowError::InvalidBindingRequest { reason })
            if reason == EXTENSION_NOT_FOUND_REASON =>
        {
            tracing::debug!("web_search is not in this deployment's extension catalog");
            return Ok(WebSearchBootstrapOutcome::SkippedUnavailable);
        }
        Err(error) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("web_search extension projection failed: {error}"),
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
            LifecyclePublicState::Active => return Ok(WebSearchBootstrapOutcome::AlreadyActive),
            LifecyclePublicState::SetupNeeded => {}
            LifecyclePublicState::Uninstalled => {
                tracing::debug!(
                    phase = ?phase,
                    "web_search is installed but projects as uninstalled; skipping bootstrap"
                );
                return Ok(WebSearchBootstrapOutcome::SkippedNonActivatable);
            }
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

    // Idempotent for an already-installed package, so this always runs
    // rather than branching on `installed` — install() no-ops if it's
    // already there.
    extension_management
        .install(package_ref.clone(), caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web_search extension install failed: {error}"),
        })?;
    extension_management
        .activate(package_ref, ExtensionActivationMode::Static, caller)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("web_search extension activation failed: {error}"),
        })?;
    Ok(WebSearchBootstrapOutcome::Activated)
}
