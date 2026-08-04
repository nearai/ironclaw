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

/// A first-party companion package as the retired `slack_user` extension was
/// declared: `HostBundled` provenance (the only source `parse_v3` lets assert
/// first-party trust), a WASM runtime, one model-visible tool. `Virtual` root
/// binding so no package files need to exist on disk.
fn retired_first_party_record(id: &str) -> ExtensionManifestRecord {
    let raw = format!(
        r#"schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = "{id} fixture"
version = "0.1.0"
description = "fixture: retired first-party companion package"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/{id}_tool.wasm"

[[tools]]
id = "{id}.search"
description = "Search messages."
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/search.input.v1.json"
"#
    );
    let manifest_hash =
        ManifestHash::new(sha256_digest_token(raw.as_bytes())).expect("manifest hash digest");
    ExtensionManifestRecord::from_toml_with_root_binding(
        raw,
        ManifestSource::HostBundled,
        &host_port_catalog(),
        Some(manifest_hash),
        &contracts(),
        PackageRootBinding::Virtual,
    )
    .expect("fixture manifest parses")
}

/// Behavioural coverage for the retired-`slack_user` boot migration
/// (`lifecycle_restore.rs::remove_retired_internal_installation`).
///
/// This branch has had **no behavioural coverage since #6616**, which deleted
/// `restore_removes_retired_slack_user_installation_without_catalog_entry` and
/// replaced it with `assert_eq!(RETIRED_SLACK_USER_EXTENSION_ID, "slack_user")`
/// — a constant compared to its own literal. That matters more than usual here:
/// the branch runs on **every boot** and **destructively** deletes persisted
/// installation rows, and its disposition is an open owner decision
/// (PROPOSAL §12.11 D-I lists restoring this coverage as step (i) of the
/// recommended sequencing, "required either way"). This test restores it
/// without changing the behavior, so the owner rules on a pinned contract
/// rather than on the code's silence.
///
/// What it pins, deliberately including the parts that read as surprising:
///
/// 1. **The retired row is gone from both port reads.** `get_installation` and
///    `get_manifest` return `None` — the branch's two store calls, in order:
///    `delete_installation` alone leaves the manifest projection authoritative
///    on purpose, so the second call is load-bearing.
/// 2. **"Deleted" means tombstoned, not erased.** The v2 record survives on the
///    filesystem with `removed_at` stamped, `removal_cleanup_pending` converged
///    back to `false`, and the full embedded manifest retained. That is the
///    actual behavior; a test asserting the row is *erased* would pin a
///    contract this code does not implement, and would hide that the migration
///    is recoverable evidence rather than data loss.
/// 3. **The two legacy projections ARE hard-deleted.**
/// 4. **The control: deletion is keyed on the extension id, not on catalog
///    absence.** A second `HostBundled` installation, equally absent from the
///    empty catalog, must survive untouched — it takes the warn-and-skip path
///    instead. Without this, a regression that deleted every catalog-miss row
///    would pass every other assertion here.
/// 5. **Neither package is published active** — the retired one because the
///    branch `continue`s before publish, the survivor because the catalog
///    cannot resolve it.
#[tokio::test]
async fn restore_removes_the_retired_slack_user_installation_and_leaves_other_uncatalogued_rows() {
    // The literal, not the crate's private constant: this test is the contract
    // for the persisted identity a deployed host may still be carrying, and it
    // must fail if that identity is quietly changed.
    const RETIRED_ID: &str = "slack_user";
    const SURVIVOR_ID: &str = "orbital-relay";

    let owner = UserId::new("retired-migration-user").expect("valid owner id");
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

    persist_installation(
        &installation_store,
        retired_first_party_record(RETIRED_ID),
        &owner,
    )
    .await;
    persist_installation(
        &installation_store,
        retired_first_party_record(SURVIVOR_ID),
        &owner,
    )
    .await;

    let retired_extension = ironclaw_host_api::ids::ExtensionId::new(RETIRED_ID).expect("id");
    let retired_installation = ExtensionInstallationId::new(RETIRED_ID).expect("id");
    let survivor_extension = ironclaw_host_api::ids::ExtensionId::new(SURVIVOR_ID).expect("id");
    let survivor_installation = ExtensionInstallationId::new(SURVIVOR_ID).expect("id");

    // Both rows are live before the migration runs, so the assertions below
    // cannot pass vacuously against a store that never held them.
    assert!(
        installation_store
            .get_installation(&retired_installation)
            .await
            .expect("read retired installation")
            .is_some(),
        "fixture must seed a live retired installation for the migration to remove"
    );
    assert!(
        installation_store
            .get_manifest(&retired_extension)
            .await
            .expect("read retired manifest")
            .is_some()
    );

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
    // Empty, as at boot before any package is discovered: neither id resolves.
    let mut catalog = AvailableExtensionCatalog::from_packages(Vec::new());

    restore_extension_lifecycle_state(
        &mut catalog,
        &filesystem,
        &installation_store,
        &lifecycle_service,
        &active_extensions,
        &owner,
    )
    .await
    .expect("restore must succeed while migrating the retired installation away");

    // 1. The retired rows are gone from both port reads.
    assert!(
        installation_store
            .get_installation(&retired_installation)
            .await
            .expect("read retired installation")
            .is_none(),
        "the retired installation must not survive restore"
    );
    assert!(
        installation_store
            .get_manifest(&retired_extension)
            .await
            .expect("read retired manifest")
            .is_none(),
        "delete_installation alone leaves the manifest authoritative — the branch's \
         second store call (delete_manifest) is what retires it"
    );
    assert!(
        !installation_store
            .list_installations()
            .await
            .expect("list installations")
            .iter()
            .any(|installation| installation.extension_id() == &retired_extension),
        "the retired installation must not be listed"
    );

    // 4. The control: an equally uncatalogued row that is NOT the retired id
    //    survives untouched. Deletion keys on the extension id, never on
    //    "the catalog could not resolve it".
    assert!(
        installation_store
            .get_installation(&survivor_installation)
            .await
            .expect("read survivor installation")
            .is_some(),
        "an unrelated installation absent from the catalog must be skipped, not deleted"
    );
    assert!(
        installation_store
            .get_manifest(&survivor_extension)
            .await
            .expect("read survivor manifest")
            .is_some(),
        "the survivor's manifest must remain authoritative"
    );

    // 5. Neither package reaches the active registry.
    assert!(
        active_extensions
            .snapshot()
            .get_extension(&retired_extension)
            .is_none(),
        "the retired package must never be published active"
    );
    assert!(
        active_extensions
            .snapshot()
            .get_extension(&survivor_extension)
            .is_none(),
        "an uncatalogued package cannot be published active"
    );

    // 2 and 3. The durable shape, read straight off the filesystem: the v2
    // record is a tombstone that retains its definition, and only the two
    // legacy projections are hard-deleted. `row_token` is
    // `sha256_digest_token(id)` with ':' folded to '_'.
    let row_token = sha256_digest_token(RETIRED_ID.as_bytes()).replace(':', "_");
    let installations_root = ExtensionInstallationStore::default_state_path()
        .expect("default state path")
        .as_str()
        .to_string();
    let read_row = |suffix: String| {
        let filesystem = Arc::clone(&filesystem);
        let path = format!("{installations_root}/{suffix}");
        async move {
            filesystem
                .get(&ironclaw_host_api::path::VirtualPath::new(&path).expect("valid row path"))
                .await
                .expect("read row")
        }
    };

    let tombstone = read_row(format!("v2/installations/{row_token}.json"))
        .await
        .expect("the v2 record survives removal as a tombstone, it is not erased");
    let tombstone: serde_json::Value =
        serde_json::from_slice(&tombstone.entry.body).expect("v2 record is JSON");
    assert!(
        tombstone.get("removed_at").is_some(),
        "the v2 record must carry removed_at — the migration tombstones, it does not erase"
    );
    assert!(
        tombstone.get("manifest").is_some(),
        "the tombstone must retain the embedded manifest record, so the removal stays \
         recoverable evidence rather than data loss"
    );
    assert!(
        tombstone.get("removal_cleanup_pending").is_none(),
        "delete_manifest must converge the tombstone (the flag is skip_serializing_if \
         Not::not, so converged == absent)"
    );

    assert!(
        read_row(format!("installations/{row_token}.json"))
            .await
            .is_none(),
        "the legacy installation projection is hard-deleted"
    );
    assert!(
        read_row(format!("manifests/{row_token}.json"))
            .await
            .is_none(),
        "the legacy manifest projection is hard-deleted"
    );
}
