use std::sync::Arc;

use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationId,
    ExtensionInstallationStore, ExtensionInstallationStorePort, ExtensionManifestRecord,
    ExtensionManifestRef, HostApiContractRegistry, InstallationOwner, MANIFEST_SCHEMA_VERSION,
    ManifestSource,
};
use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RootFilesystem};
use ironclaw_host_api::{
    host_port::HostPortCatalog,
    ids::{ExtensionId, SecretHandle},
    ingress::AllowedEffectPath,
    path::VirtualPath,
};
use ironclaw_product::adapter_registry::{
    ManifestHash, parse_product_adapter_manifest_record, product_adapter_sections,
    register_product_adapter_host_api_contract,
};

fn extension_id() -> ExtensionId {
    ExtensionId::new("telegram-v2").unwrap()
}

fn installation_id() -> ExtensionInstallationId {
    ExtensionInstallationId::new("acme-telegram-prod").unwrap()
}

fn credential(value: &str) -> ExtensionCredentialHandle {
    ExtensionCredentialHandle::new(value).unwrap()
}

fn manifest_hash(value: &str) -> ManifestHash {
    ManifestHash::new(value).unwrap()
}

fn product_contracts() -> HostApiContractRegistry {
    let mut contracts = HostApiContractRegistry::new();
    register_product_adapter_host_api_contract(&mut contracts).unwrap();
    contracts
}

async fn filesystem_store() -> ExtensionInstallationStore {
    ExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/test").unwrap(),
        HostPortCatalog::empty(),
        product_contracts(),
    )
    .await
    .unwrap()
}

fn manifest(required_credential: &str, hash: &str) -> ExtensionManifestRecord {
    let raw = format!(
        r#"
schema_version = "{schema}"
id = "telegram-v2"
name = "Telegram"
version = "0.1.0"
description = "Telegram product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/telegram-v2.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "bearer_token"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "{required_credential}"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    parse_product_adapter_manifest_record(
        raw,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        Some(manifest_hash(hash)),
    )
    .unwrap()
}

/// Frozen from the rc1 product-adapter wire contract. The effect path was
/// renamed before 1.1, but hosted rc1 persisted the original manifest bytes in
/// its monolithic installation snapshot.
fn exact_rc1_product_workflow_manifest() -> &'static str {
    r#"
schema_version = "reborn.extension_manifest.v2"
id = "telegram"
name = "Telegram"
version = "0.1.0"
description = "Telegram Bot API channel: DM IronClaw on Telegram after pairing your account."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "telegram_v2_host_beta"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "shared_secret_header"
header_name = "X-Telegram-Bot-Api-Secret-Token"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages", "external_final_reply_push", "delivery_status_reporting"]

[[product_adapter.inbound.required_credentials]]
handle = "telegram_bot_token"

[[product_adapter.inbound.required_credentials]]
handle = "telegram_webhook_secret"

[[product_adapter.inbound.egress]]
host = "api.telegram.org"
credential_handle = "telegram_bot_token"

[[product_adapter.inbound.host_ingress]]
credential_handles = ["telegram_webhook_secret"]

[product_adapter.inbound.host_ingress.descriptor]
route_id = "telegram.updates"
method = "post"
route_pattern = "/webhooks/extensions/telegram/updates"

[product_adapter.inbound.host_ingress.descriptor.policy]
listener_class = "public_webhook"
auth = { type = "required", schemes = ["webhook_signature"] }
scope_source = "host_resolved"
body_limit = { type = "limited", max_bytes = 1048576 }
rate_limit = { type = "limited", scope = "global", max_requests = 12000, window_seconds = 60 }
cors = "not_applicable"
websocket_origin = "not_applicable"
streaming = "none"
audit = "public_callback"
effect_path = { type = "product_workflow" }
"#
}

fn installation() -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id(),
        extension_id(),
        ExtensionManifestRef::new(extension_id(), Some(manifest_hash("sha256:abc123"))),
        vec![ExtensionCredentialBinding::new(
            credential("telegram_bot_token"),
            SecretHandle::new("secret_telegram_bot_token").unwrap(),
        )],
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .unwrap()
}

#[tokio::test]
async fn default_store_has_no_enabled_installations() {
    let store = filesystem_store().await;

    assert!(store.list_manifests().await.unwrap().is_empty());
    assert!(store.list_installations().await.unwrap().is_empty());
}

#[tokio::test]
async fn installed_extension_surfaces_product_adapter_runtime_entries() {
    let store = filesystem_store().await;
    store
        .upsert_manifest_and_installation(
            manifest("telegram_bot_token", "sha256:abc123"),
            installation(),
        )
        .await
        .unwrap();

    let installed = store.list_installations().await.unwrap();
    assert_eq!(installed.len(), 1);

    let manifest = store
        .get_manifest(installed[0].extension_id())
        .await
        .unwrap()
        .expect("manifest for installation");
    let sections = product_adapter_sections(&manifest).unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].adapter_id().as_str(), "telegram-v2/inbound");
}

#[tokio::test]
async fn rc1_snapshot_imports_product_workflow_effect_path() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let snapshot_path =
        VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json").unwrap();
    let snapshot = serde_json::json!({
        "manifests": [{
            "raw_toml": exact_rc1_product_workflow_manifest(),
            "source": "host_bundled",
            "manifest_hash": "sha256:abc123"
        }],
        "installations": [{
            "installation_id": "telegram",
            "extension_id": "telegram",
            "activation_state": "enabled",
            "manifest_ref": {
                "extension_id": "telegram",
                "manifest_hash": "sha256:abc123"
            },
            "credential_bindings": [{
                "credential_handle": "telegram_bot_token",
                "secret_handle": "secret_telegram_bot_token"
            }, {
                "credential_handle": "telegram_webhook_secret",
                "secret_handle": "secret_telegram_webhook_secret"
            }],
            "health": null,
            "updated_at": "2026-07-01T00:00:00Z"
        }]
    });
    filesystem
        .put(
            &snapshot_path,
            Entry::bytes(serde_json::to_vec(&snapshot).unwrap()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    let ordinary_error = parse_product_adapter_manifest_record(
        exact_rc1_product_workflow_manifest(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        Some(manifest_hash("sha256:abc123")),
    )
    .expect_err("ordinary 1.1 manifest parsing must keep rejecting the retired value");
    assert!(ordinary_error.to_string().contains("product_workflow"));
    let root = VirtualPath::new("/system/extensions/.installations/rc1-product-adapter").unwrap();
    let mut store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        product_contracts(),
    )
    .await
    .unwrap();

    let report = store
        .import_rc1_snapshot_at(&snapshot_path)
        .await
        .expect("released rc1 product-adapter snapshot must restore during startup");
    assert_eq!(report.sources_migrated, 1);
    assert_eq!(report.manifests_migrated, 1);
    assert_eq!(report.installations_migrated, 1);

    let rc1_installation_id = ExtensionInstallationId::new("telegram").unwrap();
    let rc1_extension_id = ExtensionId::new("telegram").unwrap();
    let installation = store
        .get_installation(&rc1_installation_id)
        .await
        .unwrap()
        .expect("rc1 installation remains available");
    assert_eq!(
        installation.persisted_activation_state(),
        ExtensionActivationState::Enabled
    );
    assert_eq!(installation.credential_bindings().len(), 2);
    assert!(installation.credential_bindings().iter().any(|binding| {
        binding.credential_handle().as_str() == "telegram_webhook_secret"
            && binding.secret_handle().as_str() == "secret_telegram_webhook_secret"
    }));
    let imported = store
        .get_manifest(&rc1_extension_id)
        .await
        .unwrap()
        .expect("rc1 manifest remains available");
    assert!(!imported.raw_toml().contains("product_workflow"));
    assert!(imported.raw_toml().contains("product_surface"));
    let adapters = product_adapter_sections(&imported).unwrap();
    assert_eq!(
        adapters[0].host_ingress()[0]
            .descriptor()
            .policy()
            .effect_path(),
        &AllowedEffectPath::ProductSurface
    );
    assert_eq!(
        filesystem
            .get(&snapshot_path)
            .await
            .unwrap()
            .unwrap()
            .entry
            .body,
        serde_json::to_vec(&snapshot).unwrap(),
        "the rc1 source remains byte-for-byte available for rollback"
    );

    drop(store);
    let mut reopened = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root,
        HostPortCatalog::empty(),
        product_contracts(),
    )
    .await
    .unwrap();
    let repeat = reopened
        .import_rc1_snapshot_at(&snapshot_path)
        .await
        .expect("a restart must recognize the retained rc1 snapshot");
    assert_eq!(repeat.sources_migrated, 0);
    assert_eq!(repeat.sources_unchanged, 1);
    assert!(
        reopened
            .get_manifest(&rc1_extension_id)
            .await
            .unwrap()
            .expect("normalized manifest survives restart")
            .raw_toml()
            .contains("product_surface")
    );
    assert!(
        reopened
            .get_installation(&rc1_installation_id)
            .await
            .unwrap()
            .is_some(),
        "normalized installation survives restart"
    );
}

#[tokio::test]
async fn non_product_adapter_extension_is_skipped_in_product_adapter_projection() {
    let plain_raw = format!(
        r#"
schema_version = "{schema}"
id = "plain-tool"
name = "Plain Tool"
version = "0.1.0"
description = "No product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/plain.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "plain-tool.do"
description = "Do something"
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/in.json"
output_schema_ref = "schemas/out.json"
prompt_doc_ref = "prompts/do.md"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let plain_id = ExtensionId::new("plain-tool").unwrap();
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new().unwrap(),
        ))
        .unwrap();
    let plain_manifest = ExtensionManifestRecord::from_toml(
        plain_raw,
        ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::HostPortCatalog::empty(),
        Some(manifest_hash("sha256:plain")),
        &contracts,
        None,
    )
    .unwrap();
    let plain_install = ExtensionInstallation::new(
        ExtensionInstallationId::new("plain-install").unwrap(),
        plain_id.clone(),
        ExtensionManifestRef::new(plain_id, Some(manifest_hash("sha256:plain"))),
        vec![],
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .unwrap();

    let store = filesystem_store().await;
    store
        .upsert_manifest_and_installation(plain_manifest.clone(), plain_install)
        .await
        .unwrap();

    let sections = product_adapter_sections(&plain_manifest).unwrap();
    assert!(
        sections.is_empty(),
        "plain extension should project no product adapter sections"
    );
}

#[tokio::test]
async fn manifest_hash_mismatch_is_rejected() {
    let store = filesystem_store().await;

    let err = store
        .upsert_manifest_and_installation(
            manifest("telegram_bot_token", "sha256:different"),
            installation(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
}

#[test]
fn installation_deserialize_rejects_duplicate_bindings() {
    let json = r#"
{
  "installation_id": "acme-telegram-prod",
  "extension_id": "telegram-v2",
  "manifest_ref": { "extension_id": "telegram-v2", "manifest_hash": "sha256:abc123" },
  "credential_bindings": [
    { "credential_handle": "telegram_bot_token", "secret_handle": "secret_a" },
    { "credential_handle": "telegram_bot_token", "secret_handle": "secret_b" }
  ],
  "health": { "status": "healthy", "message": null, "checked_at": "2026-01-01T00:00:00Z" },
  "updated_at": "2026-01-01T00:00:00Z"
}
"#;
    let err = serde_json::from_str::<ExtensionInstallation>(json).unwrap_err();
    assert!(err.to_string().contains("duplicate credential binding"));
}

#[test]
fn duplicate_credential_bindings_rejected_at_construction() {
    let err = ExtensionInstallation::new(
        installation_id(),
        extension_id(),
        ExtensionManifestRef::new(extension_id(), Some(manifest_hash("sha256:abc123"))),
        vec![
            ExtensionCredentialBinding::new(
                credential("telegram_bot_token"),
                SecretHandle::new("secret_a").unwrap(),
            ),
            ExtensionCredentialBinding::new(
                credential("telegram_bot_token"),
                SecretHandle::new("secret_b").unwrap(),
            ),
        ],
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ExtensionInstallationError::DuplicateCredentialBinding { .. }
        ),
        "expected DuplicateCredentialBinding, got {err:?}"
    );
}

#[tokio::test]
async fn multiple_product_adapter_sections_all_surfaced() {
    let raw = format!(
        r#"
schema_version = "{schema}"
id = "multi-adapter"
name = "Multi Adapter"
version = "0.1.0"
description = "Extension with two product adapter sections"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/multi.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.outbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "bearer_token"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "inbound_token"

[product_adapter.outbound]
surface_kind = "external_channel"

[product_adapter.outbound.auth]
kind = "bearer_token"

[product_adapter.outbound.capabilities]
flags = ["external_final_reply_push"]

[[product_adapter.outbound.required_credentials]]
handle = "outbound_token"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    let multi_id = ExtensionId::new("multi-adapter").unwrap();
    let multi_manifest = parse_product_adapter_manifest_record(
        raw,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        Some(manifest_hash("sha256:multi")),
    )
    .unwrap();
    assert_eq!(
        product_adapter_sections(&multi_manifest).unwrap().len(),
        2,
        "manifest should project two product adapter sections"
    );

    let multi_install = ExtensionInstallation::new(
        ExtensionInstallationId::new("multi-install").unwrap(),
        multi_id.clone(),
        ExtensionManifestRef::new(multi_id, Some(manifest_hash("sha256:multi"))),
        vec![
            ExtensionCredentialBinding::new(
                credential("inbound_token"),
                SecretHandle::new("secret_inbound").unwrap(),
            ),
            ExtensionCredentialBinding::new(
                credential("outbound_token"),
                SecretHandle::new("secret_outbound").unwrap(),
            ),
        ],
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .unwrap();

    let store = filesystem_store().await;
    store
        .upsert_manifest_and_installation(multi_manifest.clone(), multi_install)
        .await
        .unwrap();

    let sections = product_adapter_sections(&multi_manifest).unwrap();
    assert_eq!(sections.len(), 2, "both PA sections should project");
    let ids: Vec<_> = sections
        .iter()
        .map(|section| section.adapter_id().as_str().to_owned())
        .collect();
    assert!(ids.contains(&"multi-adapter/inbound".to_owned()));
    assert!(ids.contains(&"multi-adapter/outbound".to_owned()));
}

#[tokio::test]
async fn arc_store_delegation_works() {
    let store = filesystem_store().await;
    let arc_store: Arc<dyn ExtensionInstallationStorePort> = Arc::new(store);
    arc_store
        .upsert_manifest_and_installation(
            manifest("telegram_bot_token", "sha256:abc123"),
            installation(),
        )
        .await
        .unwrap();

    let installed = arc_store.list_installations().await.unwrap();
    assert_eq!(installed.len(), 1);
}
