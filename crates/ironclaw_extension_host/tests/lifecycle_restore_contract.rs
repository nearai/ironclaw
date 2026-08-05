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
    ironclaw_host_api::host_port::default_host_port_catalog().expect("default host port catalog")
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
fn first_party_companion_record(id: &str) -> ExtensionManifestRecord {
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

/// Boot restore special-cases **no** extension id (owner ruling 2026-08-04,
/// PROPOSAL §12.11 D-I).
///
/// This test replaces
/// `restore_removes_the_retired_slack_user_installation_and_leaves_other_uncatalogued_rows`,
/// which pinned the opposite contract. `lifecycle_restore.rs` used to carry a
/// retired-extension-id constant and a removal branch keyed on it, which ran on
/// **every boot** and destructively deleted the persisted installation and
/// manifest rows for the formerly-retired `slack_user` identity. Both
/// identifiers are banned by name in `reborn_retired_taxonomy.rs`, which is why
/// neither is spelled out here. The owner ruled that machinery out
/// ("just get rid of slack_bot and slack_personal etc.. those can get nuked.
/// Don't worry about migration data"), and it is deleted.
///
/// What this pins is the *absence*, which is the part a future change could
/// silently undo:
///
/// 1. **A `slack_user` row is not deleted at boot.** Both port reads still
///    return it. This is the exact assertion that was inverted; re-introducing
///    the branch fails here first.
/// 2. **Its durable record is untouched** — no `removed_at` tombstone is
///    stamped, and both legacy projections survive. The old branch hard-deleted
///    the two projections and tombstoned the v2 record, so this half fails too
///    if any part of the deletion path comes back.
/// 3. **It is treated exactly like any other uncatalogued row** — the control
///    (`orbital-relay`, equally absent from the empty catalog) is asserted
///    identically. There is no longer a per-id code path for the two to differ
///    on, and pinning them symmetrically is what says so.
/// 4. **Neither package is published active**, because the catalog cannot
///    resolve either. That is the warn-and-skip path in `restore`, and it is
///    the entire cost of not auto-deleting: one `warn!` per boot and one inert
///    row that `installed_summaries` already drops on the catalog miss.
#[tokio::test]
async fn restore_special_cases_no_extension_id_and_leaves_every_uncatalogued_row_intact() {
    // The literal, not a crate constant — there is deliberately no constant
    // for it any more. This is the persisted identity a deployed host may
    // still be carrying, and the point of the test is that nothing keys on it.
    const FORMERLY_RETIRED_ID: &str = "slack_user";
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
        first_party_companion_record(FORMERLY_RETIRED_ID),
        &owner,
    )
    .await;
    persist_installation(
        &installation_store,
        first_party_companion_record(SURVIVOR_ID),
        &owner,
    )
    .await;

    let formerly_retired_extension =
        ironclaw_host_api::ids::ExtensionId::new(FORMERLY_RETIRED_ID).expect("id");
    let formerly_retired_installation =
        ExtensionInstallationId::new(FORMERLY_RETIRED_ID).expect("id");
    let survivor_extension = ironclaw_host_api::ids::ExtensionId::new(SURVIVOR_ID).expect("id");
    let survivor_installation = ExtensionInstallationId::new(SURVIVOR_ID).expect("id");

    // Both rows are live before the migration runs, so the assertions below
    // cannot pass vacuously against a store that never held them.
    assert!(
        installation_store
            .get_installation(&formerly_retired_installation)
            .await
            .expect("read retired installation")
            .is_some(),
        "fixture must seed a live retired installation for the migration to remove"
    );
    assert!(
        installation_store
            .get_manifest(&formerly_retired_extension)
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

    // 1. The formerly-retired row survives both port reads. This is the
    //    assertion the deleted branch inverted: it called delete_installation
    //    and delete_manifest on exactly this id.
    assert!(
        installation_store
            .get_installation(&formerly_retired_installation)
            .await
            .expect("read formerly-retired installation")
            .is_some(),
        "boot restore must not delete any installation by extension id — the \
         retired-identity branch was removed by owner ruling (PROPOSAL §12.11 D-I)"
    );
    assert!(
        installation_store
            .get_manifest(&formerly_retired_extension)
            .await
            .expect("read formerly-retired manifest")
            .is_some(),
        "the manifest projection must stay authoritative; the deleted branch's \
         second store call (delete_manifest) is what used to retire it"
    );
    assert!(
        installation_store
            .list_installations()
            .await
            .expect("list installations")
            .iter()
            .any(|installation| installation.extension_id() == &formerly_retired_extension),
        "the row stays listed — it is inert, not erased"
    );

    // 3. The control, asserted identically to the row above: there is no
    //    per-id code path for the two to differ on any more.
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

    // 4. Neither package reaches the active registry: the catalog cannot
    //    resolve either, so both take restore's warn-and-skip path.
    assert!(
        active_extensions
            .snapshot()
            .get_extension(&formerly_retired_extension)
            .is_none(),
        "an uncatalogued package cannot be published active"
    );
    assert!(
        active_extensions
            .snapshot()
            .get_extension(&survivor_extension)
            .is_none(),
        "an uncatalogued package cannot be published active"
    );

    // 2. The durable shape, read straight off the filesystem: the record is
    //    untouched. The deleted branch tombstoned the v2 record (`removed_at`
    //    stamped) and hard-deleted both legacy projections, so each assertion
    //    below fails if any part of that path returns. `row_token` is
    //    `sha256_digest_token(id)` with ':' folded to '_'.
    let row_token = sha256_digest_token(FORMERLY_RETIRED_ID.as_bytes()).replace(':', "_");
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

    let record = read_row(format!("v2/installations/{row_token}.json"))
        .await
        .expect("the v2 record must still be on disk — nothing removed it");
    let record: serde_json::Value =
        serde_json::from_slice(&record.entry.body).expect("v2 record is JSON");
    assert!(
        record.get("removed_at").is_none_or(|stamp| stamp.is_null()),
        "no removed_at timestamp may be stamped: boot restore no longer removes this \
         row, so a tombstone here means the retired-identity branch is back. Accepting \
         both absent and explicit-null because either spelling means 'not removed'."
    );
    assert!(
        record.get("manifest").is_some(),
        "the record must retain its embedded manifest"
    );

    assert!(
        read_row(format!("installations/{row_token}.json"))
            .await
            .is_some(),
        "the legacy installation projection must survive — the deleted branch \
         hard-deleted it"
    );
    assert!(
        read_row(format!("manifests/{row_token}.json"))
            .await
            .is_some(),
        "the legacy manifest projection must survive — the deleted branch hard-deleted it"
    );
}
