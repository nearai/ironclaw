//! Reborn integration-test harness — first-party `web-access.*` scaffolding
//! (C-WEBACCESS).
//!
//! `web-access.search` / `web-access.get_content` are `RuntimeKind::FirstParty`
//! capabilities, not MCP-extension capabilities — this module does NOT reuse
//! `harness_mcp.rs`'s `McpRuntime` scaffolding. The real dispatch logic
//! (`WebAccessExecutor::dispatch`, three sequential `RuntimeHttpEgress` calls
//! to the Exa MCP endpoint) lives in the `ironclaw_first_party_extensions`
//! executor; extension-runtime DEL-7 moved the thin `FirstPartyCapabilityHandler`
//! wrapper out of composition into the assembling binary, so this harness — like
//! the binary — builds that wrapper directly over the executor
//! (`register_web_access_first_party_handlers` below). Only manifest/schema
//! loading and the trust/network policy below are harness-local test-support
//! concerns.
//!
//! `web_access_extension_package()` mirrors `github.rs`'s
//! `extension_registry()`: reads the REAL production manifest off disk via
//! `ExtensionManifest::parse` rather than hand-
//! authoring a synthetic one. Schema `$ref`s resolve later against the real
//! schema files mounted at `/system/extensions/web-access`.
//!
//! web-access declares zero `runtime_credentials`, so no credential-injecting
//! authorizer is needed — `web_access_tools` wires the plain default
//! `GrantAuthorizer`, same as `core_builtin_tools()` for `builtin.http`.

#![allow(dead_code)] // Test-only scaffolding; not every consumer exercises every helper.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_first_party_extensions::{
    EXA_MCP_HOST, NETWORK_EGRESS_LIMIT, WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID,
    WebAccessDispatchError, WebAccessDispatchRequest, WebAccessExecutor,
};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    PackageId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion as HostRuntimeCapabilitySurfaceVersion, FirstPartyCapabilityError,
    FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest,
    FirstPartyCapabilityResult, HostRuntime, HostRuntimeServices,
    default_host_api_contract_registry, default_host_port_catalog,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::SecretStore;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};

use super::harness::{LocalDevRootMounts, RecordingRuntimeHttpEgress, local_dev_root_filesystem};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Extension id (== manifest `service`) for the real production web-access
/// package (its manifest/schema assets are loaded from disk, not test-only).
pub(super) const WEB_ACCESS_PROVIDER_ID: &str = "web-access";

/// Build the `ExtensionPackage` for the real `web-access` provider by parsing
/// its production manifest off disk, mirroring `github.rs`'s
/// `extension_registry()` construction 1:1 via
/// `ExtensionManifest::parse` and
/// `ExtensionPackage::from_manifest`. The two capability manifests,
/// `web-access.search` and `web-access.get_content`, and their real JSON
/// Schema refs come from the manifest itself — no hand-authored schema.
pub(super) fn web_access_extension_package() -> HarnessResult<ExtensionPackage> {
    // Parse through the single record entry point (the bundled assets are
    // manifest v3 documents since the first-party rewrite).
    let record = ironclaw_extensions::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(asset_root().join("manifest.toml"))?,
        ManifestSource::HostBundled,
        &default_host_port_catalog()?,
        None,
        &default_host_api_contract_registry()?,
    )?;
    let manifest = ExtensionManifest::try_from(record.manifest().clone())?;
    Ok(ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{WEB_ACCESS_PROVIDER_ID}"))?,
    )?)
}

/// Filesystem location of the real production `web-access` extension assets
/// (manifest + JSON schemas), mirroring `github.rs`'s `asset_root()`.
pub(super) fn asset_root() -> PathBuf {
    repo_root().join("crates/ironclaw_first_party_extensions/assets/web-access")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Trust policy admitting the test-only `web-access` provider as first-party,
/// kept aligned with `first_party_trust_policy()`/`github_first_party_trust_policy()`
/// in `harness.rs`. The manifest path must match the `PackageSource::LocalManifest`
/// key the host runtime derives from `web_access_extension_package()`'s root.
pub(super) fn web_access_first_party_trust_policy() -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![AdminEntry::for_local_manifest(
            PackageId::new(WEB_ACCESS_PROVIDER_ID)?,
            format!("/system/extensions/{WEB_ACCESS_PROVIDER_ID}/manifest.toml"),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability, EffectKind::Network],
            None,
        )]),
    )])?)
}

/// Network policy restricted to the Exa MCP host, kept aligned with the
/// production policy inputs from private `exa_mcp_network_policy()`
/// (`crates/ironclaw_first_party_extensions/src/web_access.rs`, not `pub`) —
/// re-declared here rather than imported.
pub(super) fn exa_mcp_test_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: EXA_MCP_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

/// Variant of `local_dev_host_runtime_with_registry_and_runtime_http_egress`
/// (`harness.rs`) that wires the `web-access` package registry plus the real
/// production `FirstPartyCapabilityRegistry` registration for both
/// capability ids, instead of the built-in first-party handler set.
pub(super) fn local_dev_host_runtime_with_web_access(
    storage_root: PathBuf,
    package_registry: ExtensionRegistry,
    http_egress: Arc<RecordingRuntimeHttpEgress>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut handlers = FirstPartyCapabilityRegistry::new();
    register_web_access_first_party_handlers(&mut handlers)?;

    let services = HostRuntimeServices::new(
        Arc::new(package_registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::web_access_assets())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(SecretStore::ephemeral()))
    .with_first_party_capabilities(Arc::new(handlers))
    .with_first_party_http_egress(http_egress)
    .with_trust_policy(Arc::new(web_access_first_party_trust_policy()?));

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

/// Register the web-access first-party capability handlers — the same thin
/// wrapper over `WebAccessExecutor` the production binary supplies through its
/// `FirstPartyHandlerRegistrar` (extension-runtime DEL-7). Mirrored here because
/// composition no longer owns this wrapper and the binary is not importable from
/// an integration test.
fn register_web_access_first_party_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    let handler = Arc::new(WebAccessFirstPartyHandler {
        executor: WebAccessExecutor::default(),
    });
    registry.insert_handler(
        CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    Ok(())
}

struct WebAccessFirstPartyHandler {
    executor: WebAccessExecutor,
}

#[async_trait::async_trait]
impl FirstPartyCapabilityHandler for WebAccessFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let result = self
            .executor
            .dispatch(WebAccessDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: request.services.runtime_http_egress.clone(),
            })
            .await
            .map_err(web_access_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn web_access_error(error: WebAccessDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}
