use std::sync::Arc;

use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionHealthMessage, ExtensionHealthSnapshot, ExtensionHealthStatus, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionManifestRef, InMemoryExtensionInstallationStore,
    MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_host_api::{ExtensionId, HostPortCatalog, SecretHandle};
use ironclaw_product_adapter_registry::{
    ManifestHash, RegistryError, list_enabled_product_adapter_entries,
    parse_product_adapter_manifest_record, product_adapter_sections,
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

fn installation(state: ExtensionActivationState) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id(),
        extension_id(),
        state,
        ExtensionManifestRef::new(extension_id(), Some(manifest_hash("sha256:abc123"))),
        vec![ExtensionCredentialBinding::new(
            credential("telegram_bot_token"),
            SecretHandle::new("secret_telegram_bot_token").unwrap(),
        )],
        Utc::now(),
    )
    .unwrap()
}

#[tokio::test]
async fn default_store_has_no_enabled_installations() {
    let store = InMemoryExtensionInstallationStore::default();

    assert!(store.list_manifests().await.unwrap().is_empty());
    assert!(store.list_enabled_installations().await.unwrap().is_empty());
}

#[tokio::test]
async fn explicit_activation_surfaces_in_product_adapter_runtime_entries() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Installed))
        .await
        .unwrap();

    store
        .set_activation_state(&installation_id(), ExtensionActivationState::Enabled)
        .await
        .unwrap();

    let enabled = store.list_enabled_installations().await.unwrap();
    assert_eq!(enabled.len(), 1);

    let entries = list_enabled_product_adapter_entries(&store).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].adapter().adapter_id().as_str(),
        "telegram-v2/inbound"
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

[[capabilities]]
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
    let plain_manifest = ExtensionManifestRecord::from_toml(
        plain_raw,
        ManifestSource::InstalledLocal,
        &ironclaw_host_api::HostPortCatalog::empty(),
        Some(manifest_hash("sha256:plain")),
    )
    .unwrap();
    let plain_install = ExtensionInstallation::new(
        ExtensionInstallationId::new("plain-install").unwrap(),
        plain_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(plain_id, Some(manifest_hash("sha256:plain"))),
        vec![],
        Utc::now(),
    )
    .unwrap();

    let store = InMemoryExtensionInstallationStore::default();
    store.upsert_manifest(plain_manifest).await.unwrap();
    store.upsert_installation(plain_install).await.unwrap();

    let pa_entries = list_enabled_product_adapter_entries(&store).await.unwrap();
    assert!(
        pa_entries.is_empty(),
        "plain extension should not appear in product adapter entries"
    );
}

#[tokio::test]
async fn credential_binding_must_reference_declared_manifest_handle() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();

    let invalid = ExtensionInstallation::new(
        installation_id(),
        extension_id(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(extension_id(), Some(manifest_hash("sha256:abc123"))),
        vec![ExtensionCredentialBinding::new(
            credential("slack_bot_token"),
            SecretHandle::new("secret_slack_bot_token").unwrap(),
        )],
        Utc::now(),
    )
    .unwrap();

    store.upsert_installation(invalid).await.unwrap();
    let err = list_enabled_product_adapter_entries(&store)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::UndeclaredCredentialHandle { .. }
    ));
}

#[tokio::test]
async fn manifest_hash_mismatch_is_rejected() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:different"))
        .await
        .unwrap();

    let err = store
        .upsert_installation(installation(ExtensionActivationState::Installed))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
}

#[tokio::test]
async fn upsert_manifest_rejects_when_existing_installation_binding_revoked() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Enabled))
        .await
        .unwrap();

    store
        .upsert_manifest(manifest("other_token", "sha256:abc123"))
        .await
        .unwrap();
    let err = list_enabled_product_adapter_entries(&store)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::UndeclaredCredentialHandle { .. }
    ));
}

#[test]
fn installation_deserialize_rejects_duplicate_bindings() {
    let json = r#"
{
  "installation_id": "acme-telegram-prod",
  "extension_id": "telegram-v2",
  "activation_state": "installed",
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
        ExtensionActivationState::Installed,
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
async fn no_op_activation_transition_does_not_update_timestamp() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Enabled))
        .await
        .unwrap();

    let before = store
        .get_installation(&installation_id())
        .await
        .unwrap()
        .unwrap();
    let before_ts = before.updated_at();

    // Set the same state again — should be a no-op.
    store
        .set_activation_state(&installation_id(), ExtensionActivationState::Enabled)
        .await
        .unwrap();

    let after = store
        .get_installation(&installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        before_ts,
        after.updated_at(),
        "no-op activation transition should not update timestamp"
    );
}

#[tokio::test]
async fn installed_state_does_not_surface_in_enabled_installations() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Installed))
        .await
        .unwrap();

    let enabled = store.list_enabled_installations().await.unwrap();
    assert!(
        enabled.is_empty(),
        "installed (not enabled) installation should not appear in list_enabled_installations"
    );

    let entries = list_enabled_product_adapter_entries(&store).await.unwrap();
    assert!(
        entries.is_empty(),
        "installed (not enabled) installation should not appear in PA runtime entries"
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
        ExtensionActivationState::Enabled,
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
    )
    .unwrap();

    let store = InMemoryExtensionInstallationStore::default();
    store.upsert_manifest(multi_manifest).await.unwrap();
    store.upsert_installation(multi_install).await.unwrap();

    let entries = list_enabled_product_adapter_entries(&store).await.unwrap();
    assert_eq!(entries.len(), 2, "both PA sections should be surfaced");

    let ids: Vec<_> = entries
        .iter()
        .map(|e| e.adapter().adapter_id().as_str().to_owned())
        .collect();
    assert!(ids.contains(&"multi-adapter/inbound".to_owned()));
    assert!(ids.contains(&"multi-adapter/outbound".to_owned()));
}

#[tokio::test]
async fn arc_store_delegation_works() {
    let store = InMemoryExtensionInstallationStore::default();
    let arc_store: Arc<dyn ExtensionInstallationStore> = Arc::new(store);
    arc_store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    arc_store
        .upsert_installation(installation(ExtensionActivationState::Enabled))
        .await
        .unwrap();

    let entries = list_enabled_product_adapter_entries(arc_store.as_ref())
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn update_health_uses_redacted_string() {
    let store = InMemoryExtensionInstallationStore::default();
    store
        .upsert_manifest(manifest("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Enabled))
        .await
        .unwrap();

    let health = ExtensionHealthSnapshot::new(
        ExtensionHealthStatus::Degraded,
        Some(ExtensionHealthMessage::new("timeout after 5s")),
        Utc::now(),
    );
    store
        .update_health(&installation_id(), health)
        .await
        .unwrap();

    let inst = store
        .get_installation(&installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inst.health().status(), ExtensionHealthStatus::Degraded);
    // ExtensionHealthMessage Debug impl should redact the value.
    let debug = format!("{:?}", inst.health().message().unwrap());
    assert!(
        !debug.contains("timeout after 5s"),
        "ExtensionHealthMessage should redact the message in Debug output"
    );
}
