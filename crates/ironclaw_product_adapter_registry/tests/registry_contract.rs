use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::SecretHandle;
use ironclaw_product_adapter_registry::{
    InMemoryProductAdapterRegistryStore, ManifestHash, ProductAdapterActivationState,
    ProductAdapterComponentRef, ProductAdapterCredentialBinding, ProductAdapterHealthSnapshot,
    ProductAdapterInstallation, ProductAdapterManifest, ProductAdapterManifestRef,
    ProductAdapterRegistryStore, RegistryError,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    EgressCredentialHandle, ProductAdapterCapabilities, ProductAdapterHealth, ProductAdapterId,
    ProductSurfaceKind, RedactedString,
};

fn adapter_id() -> ProductAdapterId {
    ProductAdapterId::new("telegram-v2").unwrap()
}

fn installation_id(value: &str) -> AdapterInstallationId {
    AdapterInstallationId::new(value).unwrap()
}

fn credential(value: &str) -> EgressCredentialHandle {
    EgressCredentialHandle::new(value).unwrap()
}

fn host(value: &str) -> DeclaredEgressHost {
    DeclaredEgressHost::new(value).unwrap()
}

fn secret_handle(value: &str) -> SecretHandle {
    SecretHandle::new(format!("secret_{value}")).unwrap()
}

fn manifest() -> ProductAdapterManifest {
    let telegram = credential("telegram_bot_token");
    ProductAdapterManifest::new(
        adapter_id(),
        semver::Version::new(0, 1, 0),
        ProductSurfaceKind::ExternalChannel,
        ProductAdapterComponentRef::new("file://adapters/telegram-v2.wasm").unwrap(),
        ProductAdapterCapabilities::external_channel_default(),
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
        vec![DeclaredEgressTarget::new(
            host("api.telegram.org"),
            Some(telegram.clone()),
        )],
        vec![telegram],
        Some(ManifestHash::new("sha256:abc123").unwrap()),
    )
    .unwrap()
}

fn installation(state: ProductAdapterActivationState) -> ProductAdapterInstallation {
    ProductAdapterInstallation::new(
        installation_id("acme-telegram-prod"),
        adapter_id(),
        state,
        ProductAdapterManifestRef::new(
            adapter_id(),
            Some(ManifestHash::new("sha256:abc123").unwrap()),
        ),
        vec![ProductAdapterCredentialBinding::new(
            credential("telegram_bot_token"),
            secret_handle("telegram-prod-bot-token"),
        )],
        Utc::now(),
    )
    .unwrap()
}

#[tokio::test]
async fn default_registry_has_no_enabled_installations() {
    let store = InMemoryProductAdapterRegistryStore::default();

    assert!(store.list_manifests().await.unwrap().is_empty());
    assert!(store.list_installations().await.unwrap().is_empty());
    assert!(store.list_enabled_installations().await.unwrap().is_empty());
}

#[tokio::test]
async fn installed_state_does_not_enable_runtime_traffic() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    store
        .upsert_installation(installation(ProductAdapterActivationState::Installed))
        .await
        .unwrap();

    assert!(store.list_enabled_installations().await.unwrap().is_empty());
}

#[tokio::test]
async fn explicit_activation_makes_installation_enabled() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    let id = installation_id("acme-telegram-prod");
    store
        .upsert_installation(installation(ProductAdapterActivationState::Installed))
        .await
        .unwrap();

    store
        .set_activation_state(&id, ProductAdapterActivationState::Enabled)
        .await
        .unwrap();

    let enabled = store.list_enabled_installations().await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].installation_id(), &id);
}

#[tokio::test]
async fn no_op_activation_transition_does_not_update_timestamp() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    let id = installation_id("acme-telegram-prod");
    store
        .upsert_installation(installation(ProductAdapterActivationState::Installed))
        .await
        .unwrap();
    let before = store
        .get_installation(&id)
        .await
        .unwrap()
        .unwrap()
        .updated_at();

    store
        .set_activation_state(&id, ProductAdapterActivationState::Installed)
        .await
        .unwrap();

    let after = store
        .get_installation(&id)
        .await
        .unwrap()
        .unwrap()
        .updated_at();
    assert_eq!(before, after);
}

#[tokio::test]
async fn arc_store_delegates_to_inner_store() {
    let store = Arc::new(InMemoryProductAdapterRegistryStore::default());
    store.upsert_manifest(manifest()).await.unwrap();

    assert_eq!(store.list_manifests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn credential_binding_must_reference_declared_manifest_handle() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    // Construction is the only public bindings-mutation path now —
    // `set_credential_bindings` is crate-private so external callers cannot
    // bypass the store's manifest re-validation by patching bindings on a
    // get_installation snapshot. The invalid installation is built up front
    // and upserted; the store rejects it on cross-manifest validation.
    let invalid = ProductAdapterInstallation::new(
        installation_id("acme-telegram-prod"),
        adapter_id(),
        ProductAdapterActivationState::Installed,
        ProductAdapterManifestRef::new(
            adapter_id(),
            Some(ManifestHash::new("sha256:abc123").unwrap()),
        ),
        vec![ProductAdapterCredentialBinding::new(
            credential("slack_bot_token"),
            secret_handle("slack-prod-bot-token"),
        )],
        Utc::now(),
    )
    .unwrap();

    let err = store.upsert_installation(invalid).await.unwrap_err();
    assert!(matches!(
        err,
        RegistryError::UndeclaredCredentialHandle { .. }
    ));
}

#[test]
fn duplicate_credential_bindings_rejected_at_construction() {
    // With `set_credential_bindings` sealed to `pub(crate)`, the only public
    // path that admits binding lists is `ProductAdapterInstallation::new`.
    // The uniqueness invariant must trip there — there is no partial-update
    // path left for an external caller to corrupt state through.
    let err = ProductAdapterInstallation::new(
        installation_id("acme-telegram-prod"),
        adapter_id(),
        ProductAdapterActivationState::Installed,
        ProductAdapterManifestRef::new(
            adapter_id(),
            Some(ManifestHash::new("sha256:abc123").unwrap()),
        ),
        vec![
            ProductAdapterCredentialBinding::new(
                credential("telegram_bot_token"),
                secret_handle("telegram-prod-bot-token"),
            ),
            ProductAdapterCredentialBinding::new(
                credential("telegram_bot_token"),
                secret_handle("telegram-prod-bot-token-shadow"),
            ),
        ],
        Utc::now(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        RegistryError::DuplicateCredentialBinding { .. }
    ));
}

fn manifest_without_credential() -> ProductAdapterManifest {
    ProductAdapterManifest::new(
        adapter_id(),
        semver::Version::new(0, 1, 1),
        ProductSurfaceKind::ExternalChannel,
        ProductAdapterComponentRef::new("file://adapters/telegram-v2.wasm").unwrap(),
        ProductAdapterCapabilities::external_channel_default(),
        AuthRequirement::BearerToken,
        Vec::new(),
        Vec::new(),
        Some(ManifestHash::new("sha256:abc123").unwrap()),
    )
    .unwrap()
}

#[tokio::test]
async fn upsert_manifest_rejects_when_existing_installation_binding_revoked() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    store
        .upsert_installation(installation(ProductAdapterActivationState::Enabled))
        .await
        .unwrap();

    let err = store
        .upsert_manifest(manifest_without_credential())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::UndeclaredCredentialHandle { .. }
    ));
}

#[tokio::test]
async fn manifest_hash_mismatch_is_rejected() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    let invalid = ProductAdapterInstallation::new(
        installation_id("acme-telegram-prod"),
        adapter_id(),
        ProductAdapterActivationState::Installed,
        ProductAdapterManifestRef::new(
            adapter_id(),
            Some(ManifestHash::new("sha256:different").unwrap()),
        ),
        vec![ProductAdapterCredentialBinding::new(
            credential("telegram_bot_token"),
            secret_handle("telegram-prod-bot-token"),
        )],
        Utc::now(),
    )
    .unwrap();

    let err = store.upsert_installation(invalid).await.unwrap_err();
    assert!(matches!(err, RegistryError::ManifestHashMismatch { .. }));
}

#[tokio::test]
async fn egress_pairs_are_preserved_exactly() {
    let telegram_handle = credential("telegram_bot_token");
    let slack_handle = credential("slack_bot_token");
    let manifest = ProductAdapterManifest::new(
        adapter_id(),
        semver::Version::new(0, 1, 0),
        ProductSurfaceKind::ExternalChannel,
        ProductAdapterComponentRef::new("file://adapters/telegram-v2.wasm").unwrap(),
        ProductAdapterCapabilities::external_channel_default(),
        AuthRequirement::BearerToken,
        vec![
            DeclaredEgressTarget::new(host("api.telegram.org"), Some(telegram_handle.clone())),
            DeclaredEgressTarget::new(host("api.slack.com"), Some(slack_handle.clone())),
        ],
        vec![telegram_handle, slack_handle],
        None,
    )
    .unwrap();

    let pairs: Vec<(String, Option<String>)> = manifest
        .declared_egress()
        .iter()
        .map(|target| {
            (
                target.host.as_str().to_string(),
                target
                    .credential_handle
                    .as_ref()
                    .map(|handle| handle.as_str().to_string()),
            )
        })
        .collect();

    assert_eq!(
        pairs,
        vec![
            (
                "api.telegram.org".to_string(),
                Some("telegram_bot_token".to_string())
            ),
            (
                "api.slack.com".to_string(),
                Some("slack_bot_token".to_string())
            )
        ]
    );
}

// ---------------------------------------------------------------------------
// Direct-deserialize bypass guards (security/correctness review fix 1).
// `ProductAdapterManifest` / `ProductAdapterInstallation` no longer derive
// `Deserialize`; their manual impls route through validating constructors so
// cross-field invariants hold even for values reconstructed from a persisted
// serialized blob.
// ---------------------------------------------------------------------------

const MANIFEST_JSON_DUPLICATE_CREDENTIALS: &str = r#"{
    "adapter_id": "telegram-v2",
    "version": "0.1.0",
    "surface_kind": "external_channel",
    "component_ref": "file://adapters/telegram-v2.wasm",
    "capabilities": {"flags": ["inbound_messages"]},
    "auth_requirement": {"bearer_token": null},
    "declared_egress": [],
    "required_credentials": ["telegram_bot_token", "telegram_bot_token"],
    "manifest_hash": null
}"#;

#[test]
fn manifest_deserialize_rejects_duplicate_credentials() {
    let err = serde_json::from_str::<ProductAdapterManifest>(MANIFEST_JSON_DUPLICATE_CREDENTIALS)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("duplicate credential handle"),
        "expected DuplicateCredentialHandle propagation, got: {err}"
    );
}

const INSTALLATION_JSON_DUPLICATE_BINDINGS: &str = r#"{
    "installation_id": "acme-telegram-prod",
    "adapter_id": "telegram-v2",
    "activation_state": "installed",
    "manifest_ref": {
        "adapter_id": "telegram-v2",
        "manifest_hash": "sha256:abc123"
    },
    "credential_bindings": [
        {"credential_handle": "telegram_bot_token", "secret_handle": "telegram_bot_token"},
        {"credential_handle": "telegram_bot_token", "secret_handle": "telegram_bot_token_shadow"}
    ],
    "health": {"status": "healthy", "checked_at": null, "message": null},
    "updated_at": "2026-05-13T20:00:00Z"
}"#;

#[test]
fn installation_deserialize_rejects_duplicate_bindings() {
    let err =
        serde_json::from_str::<ProductAdapterInstallation>(INSTALLATION_JSON_DUPLICATE_BINDINGS)
            .unwrap_err()
            .to_string();
    assert!(
        err.contains("duplicate credential binding"),
        "expected DuplicateCredentialBinding propagation, got: {err}"
    );
}

const INSTALLATION_JSON_MANIFEST_ADAPTER_MISMATCH: &str = r#"{
    "installation_id": "acme-telegram-prod",
    "adapter_id": "telegram-v2",
    "activation_state": "installed",
    "manifest_ref": {
        "adapter_id": "slack-v2",
        "manifest_hash": "sha256:abc123"
    },
    "credential_bindings": [],
    "health": {"status": "healthy", "checked_at": null, "message": null},
    "updated_at": "2026-05-13T20:00:00Z"
}"#;

#[test]
fn installation_deserialize_rejects_manifest_adapter_mismatch() {
    let err = serde_json::from_str::<ProductAdapterInstallation>(
        INSTALLATION_JSON_MANIFEST_ADAPTER_MISMATCH,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("does not match manifest adapter"),
        "expected ManifestAdapterMismatch propagation, got: {err}"
    );
}

#[tokio::test]
async fn health_update_redacts_message() {
    let store = InMemoryProductAdapterRegistryStore::default();
    store.upsert_manifest(manifest()).await.unwrap();
    let id = installation_id("acme-telegram-prod");
    store
        .upsert_installation(installation(ProductAdapterActivationState::Enabled))
        .await
        .unwrap();

    store
        .update_health(
            &id,
            ProductAdapterHealthSnapshot::new(
                ProductAdapterHealth::Degraded,
                Some(Utc::now()),
                Some(RedactedString::new("super-secret-token")),
            ),
        )
        .await
        .unwrap();

    let got = store.get_installation(&id).await.unwrap().unwrap();
    let rendered = format!("{:?}", got.health());
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("super-secret-token"));
}
