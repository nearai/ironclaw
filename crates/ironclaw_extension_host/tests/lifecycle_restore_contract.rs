//! Pins `restore_extension_lifecycle_state`'s activatable-surface guard
//! (`crates/ironclaw_extension_host/src/lifecycle_restore.rs`).
//!
//! Drives the real production caller directly: a real `ExtensionInstallationStore`
//! over an in-memory `RootFilesystem`, the real `ExtensionLifecycleService`, and
//! the real `ActiveExtensionPublisher`. No fakes stand in for the guard or its
//! caller.

use std::sync::Arc;

use ironclaw_extension_host::{
    ActiveExtensionPublisher, AvailableExtensionCatalog,
    product_extension_host_api_contract_registry, restore_extension_lifecycle_state,
};
use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionInstallationStorePort, ExtensionLifecycleService, ExtensionManifestRecord,
    ExtensionManifestRef, ExtensionRegistry, InstallationOwner, ManifestHash, ManifestSource,
    PackageRootBinding, SharedExtensionRegistry,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_host_api::{approval::sha256_digest_token, ids::CapabilityId, ids::UserId};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

fn host_port_catalog() -> ironclaw_host_api::host_port::HostPortCatalog {
    ironclaw_host_runtime::default_host_port_catalog().expect("default host port catalog")
}

fn contracts() -> ironclaw_extensions::HostApiContractRegistry {
    product_extension_host_api_contract_registry().expect("host API contracts")
}

/// A hosted-MCP `[mcp]` manifest, optionally carrying a statically-declared
/// (already "discovered") model-visible tool. With `tool_toml` empty, the
/// package declares nothing but the host-internal `{id}.mcp_server`
/// connection-template capability (`ironclaw_extensions::v3::parse_v3`,
/// `CapabilityVisibility::HostInternal`) — the exact shape of a hosted MCP
/// registration that has not yet been discovered.
fn hosted_mcp_manifest_record(id: &str, tool_toml: &str) -> ExtensionManifestRecord {
    let raw = format!(
        r#"schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = "{id} fixture"
version = "0.1.0"
description = "fixture: hosted MCP restore regression"
trust = "third_party"

[mcp]
server = "https://mcp.example.test/{id}"
namespace = "{id}"
max_tools = 32
default_permission = "ask"
effects = ["network", "use_secret"]
{tool_toml}"#
    );
    let manifest_hash =
        ManifestHash::new(sha256_digest_token(raw.as_bytes())).expect("manifest hash digest");
    ExtensionManifestRecord::from_toml_with_root_binding(
        raw,
        ManifestSource::UserRegistered,
        &host_port_catalog(),
        Some(manifest_hash),
        &contracts(),
        PackageRootBinding::Virtual,
    )
    .expect("fixture manifest parses")
}

/// Statically-pinned tool TOML for an already-discovered hosted MCP. Per
/// `parse_v3`, a static tool on an `[mcp]` manifest inherits the connection
/// template's credentials/effects, so it must not declare its own.
const DISCOVERED_TOOL_TOML: &str = r#"
[[tools]]
id = "mcp-healthy.search"
description = "Search the healthy MCP catalog."
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/mcp-healthy/dynamic/search.input.v1.json"
"#;

async fn persist_installation(
    store: &Arc<dyn ExtensionInstallationStorePort>,
    record: ExtensionManifestRecord,
    owner: &UserId,
) {
    let extension_id = record.resolved().id.clone();
    let manifest_hash = record
        .manifest_hash()
        .cloned()
        .expect("fixture manifest carries a hash");
    let installation_id = ExtensionInstallationId::new(extension_id.as_str().to_string())
        .expect("valid installation id");
    let installation = ExtensionInstallation::new(
        installation_id,
        extension_id.clone(),
        ExtensionManifestRef::new(extension_id, Some(manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(owner.clone()),
    )
    .expect("installation row constructs");
    store
        .upsert_manifest_and_installation(record, installation)
        .await
        .expect("persist manifest + installation row");
}

/// Regression for the boot-critical guard in `restore_extension_lifecycle_state`
/// (`lifecycle_restore.rs`, ~L84-102): a persisted installation whose package
/// declares no model-visible capability, channel, or hook — the shape of a
/// hosted-MCP registration that has been installed but never discovered, which
/// synthesizes only the host-internal `{id}.mcp_server` connection template
/// (`ironclaw_extensions::v3`, `CapabilityVisibility::HostInternal`) — must be
/// installed into the lifecycle service WITHOUT being enabled or published.
///
/// Before this guard existed, restore called `lifecycle.enable(..)`
/// unconditionally, which for such a package fails activation's binding check
/// with `BindError::EmptyHostedMcpToolCatalog`
/// (`entrypoint.rs::check_binding`). That error propagates out of
/// `restore_extension_lifecycle_state` via `?` — aborting the ENTIRE restore
/// loop, so every installation processed after the broken one silently fails
/// to restore too. This test proves the failure is now contained to the one
/// undiscovered installation: a second, ordinary installation later in the
/// same batch still restores, enables, and publishes normally.
///
/// Fixture ids are chosen so the undiscovered package sorts first
/// (`mcp-broken` < `mcp-healthy`): `ExtensionInstallationStore::list_installations`
/// sorts by installation id and installation id == extension id here, so restore
/// processes the broken package before the healthy one — the ordering that
/// exposes "the rest of the loop never runs" if the guard regresses.
#[tokio::test]
async fn restore_installs_but_does_not_enable_an_undiscovered_hosted_mcp_package() {
    let owner = UserId::new("restore-guard-user").expect("valid owner id");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let installation_store: Arc<dyn ExtensionInstallationStorePort> = Arc::new(
        ExtensionInstallationStore::load_at(
            Arc::clone(&filesystem),
            ExtensionInstallationStore::default_state_path().expect("default state path"),
            host_port_catalog(),
            contracts(),
        )
        .await
        .expect("installation store opens"),
    );

    // The undiscovered hosted MCP: only the HostInternal connection-template
    // capability, nothing model-visible.
    persist_installation(
        &installation_store,
        hosted_mcp_manifest_record("mcp-broken", ""),
        &owner,
    )
    .await;
    // An ordinary, already-discovered hosted MCP restoring in the same batch,
    // right after the broken one.
    persist_installation(
        &installation_store,
        hosted_mcp_manifest_record("mcp-healthy", DISCOVERED_TOOL_TOML),
        &owner,
    )
    .await;

    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let lifecycle_service = Arc::new(Mutex::new(ExtensionLifecycleService::new(
        active_registry.snapshot_owned(),
    )));
    let trust_policy = Arc::new(
        HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy builds"),
    );
    let active_extensions = ActiveExtensionPublisher::new(
        Arc::clone(&active_registry),
        trust_policy,
        Arc::new(InvalidationBus::new()),
    );
    let mut catalog = AvailableExtensionCatalog::from_packages(Vec::new());

    // Assertion 1: restore succeeds even though one installation has nothing
    // activatable yet.
    restore_extension_lifecycle_state(
        &mut catalog,
        &filesystem,
        &installation_store,
        &lifecycle_service,
        &active_extensions,
        &owner,
    )
    .await
    .expect(
        "restore must succeed for the whole batch even though mcp-broken has no \
         activatable surface yet",
    );

    let broken_id = ironclaw_host_api::ids::ExtensionId::new("mcp-broken").expect("extension id");
    let healthy_id = ironclaw_host_api::ids::ExtensionId::new("mcp-healthy").expect("extension id");

    // Assertion 2: mcp-broken is installed (present in the lifecycle service's
    // registry, proving `lifecycle.install(..)` ran) but not enabled or
    // published (absent from the active registry `active_extensions` publishes
    // into).
    assert!(
        lifecycle_service
            .lock()
            .await
            .registry()
            .get_extension(&broken_id)
            .is_some(),
        "mcp-broken must be installed into the lifecycle service"
    );
    assert!(
        active_extensions
            .snapshot()
            .get_extension(&broken_id)
            .is_none(),
        "mcp-broken has no activatable surface yet and must not be published active"
    );

    // Assertion 3: mcp-healthy, restored right after the broken package in the
    // same loop, still enables and publishes normally — the failure is
    // contained, not boot-wide.
    assert!(
        active_extensions
            .snapshot()
            .get_extension(&healthy_id)
            .is_some(),
        "mcp-healthy must still restore and publish after mcp-broken in the same batch"
    );
    assert!(
        active_extensions
            .snapshot()
            .get_capability(&CapabilityId::new("mcp-healthy.search").expect("capability id"))
            .is_some(),
        "mcp-healthy's discovered tool capability must be published active"
    );
}
