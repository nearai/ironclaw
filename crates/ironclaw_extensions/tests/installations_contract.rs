use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthMessage, ExtensionHealthSnapshot,
    ExtensionHealthStatus, ExtensionInstallation, ExtensionInstallationError,
    ExtensionInstallationId, ExtensionInstallationStore, ExtensionManifestRecord,
    ExtensionManifestRef, InMemoryExtensionInstallationStore, MANIFEST_SCHEMA_VERSION,
    ManifestHash, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{ExtensionId, HostPortCatalog};

fn extension_id(value: &str) -> ExtensionId {
    ExtensionId::new(value).unwrap()
}

fn installation_id(value: &str) -> ExtensionInstallationId {
    ExtensionInstallationId::new(value).unwrap()
}

fn manifest_hash(value: &str) -> ManifestHash {
    ManifestHash::new(value).unwrap()
}

fn raw_legacy_capability_manifest() -> String {
    format!(
        r#"
schema_version = "{schema}"
id = "acme-tools"
name = "Acme Tools"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme.wasm"

[[capabilities]]
id = "acme-tools.echo"
description = "Echoes input"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/acme/echo.input.v1.json"
output_schema_ref = "schemas/acme/echo.output.v1.json"
prompt_doc_ref = "prompts/acme/echo.md"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    )
}

fn manifest(hash: &str) -> ExtensionManifestRecord {
    ExtensionManifestRecord::from_toml(
        raw_legacy_capability_manifest(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        Some(manifest_hash(hash)),
    )
    .unwrap()
}

fn installation(hash: &str) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id("acme-tools-prod"),
        extension_id("acme-tools"),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(extension_id("acme-tools"), Some(manifest_hash(hash))),
        vec![],
        Utc::now(),
    )
    .unwrap()
}

fn installation_with_manifest_hash(hash: Option<&str>) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id("acme-tools-prod"),
        extension_id("acme-tools"),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(extension_id("acme-tools"), hash.map(manifest_hash)),
        vec![],
        Utc::now(),
    )
    .unwrap()
}

#[test]
fn installed_legacy_top_level_capabilities_are_rejected() {
    let err = ExtensionManifestRecord::from_toml(
        raw_legacy_capability_manifest(),
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        Some(manifest_hash("sha256:abc")),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExtensionInstallationError::Manifest(
            ManifestV2Error::LegacyTopLevelCapabilitiesForInstalledSource {
                manifest_source: ManifestSource::InstalledLocal
            }
        )
    ));
}

#[tokio::test]
async fn upsert_installation_rejects_unknown_manifest() {
    let store = InMemoryExtensionInstallationStore::default();

    let err = store
        .upsert_installation(
            ExtensionInstallation::new(
                installation_id("missing-prod"),
                extension_id("missing-tools"),
                ExtensionActivationState::Installed,
                ExtensionManifestRef::new(
                    extension_id("missing-tools"),
                    Some(manifest_hash("sha256:missing")),
                ),
                vec![],
                Utc::now(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionInstallationError::UnknownManifest { .. }
    ));
}

#[tokio::test]
async fn upsert_manifest_rejects_manifest_hash_change_for_existing_installation() {
    let store = InMemoryExtensionInstallationStore::default();
    store.upsert_manifest(manifest("sha256:old")).await.unwrap();
    store
        .upsert_installation(installation("sha256:old"))
        .await
        .unwrap();

    let err = store
        .upsert_manifest(manifest("sha256:new"))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
}

#[tokio::test]
async fn upsert_manifest_and_installation_replaces_coherent_manifest_hash_pair() {
    let store = InMemoryExtensionInstallationStore::default();
    store.upsert_manifest(manifest("sha256:old")).await.unwrap();
    store
        .upsert_installation(installation("sha256:old"))
        .await
        .unwrap();

    store
        .upsert_manifest_and_installation(manifest("sha256:new"), installation("sha256:new"))
        .await
        .unwrap();

    let manifest = store
        .get_manifest(&extension_id("acme-tools"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(manifest.manifest_hash(), Some(&manifest_hash("sha256:new")));
    let installation = store
        .get_installation(&installation_id("acme-tools-prod"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        installation.manifest_ref().manifest_hash(),
        Some(&manifest_hash("sha256:new"))
    );
}

#[tokio::test]
async fn upsert_manifest_and_installation_rejects_mismatched_manifest_hash_pair() {
    let store = InMemoryExtensionInstallationStore::default();

    let err = store
        .upsert_manifest_and_installation(manifest("sha256:new"), installation("sha256:old"))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
    assert!(
        store
            .get_manifest(&extension_id("acme-tools"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_installation(&installation_id("acme-tools-prod"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn missing_installation_mutations_return_not_found() {
    let store = InMemoryExtensionInstallationStore::default();
    let missing = installation_id("missing-prod");

    let activation_err = store
        .set_activation_state(&missing, ExtensionActivationState::Enabled)
        .await
        .unwrap_err();
    assert!(matches!(
        activation_err,
        ExtensionInstallationError::InstallationNotFound { .. }
    ));

    let health_err = store
        .update_health(&missing, ExtensionHealthSnapshot::healthy())
        .await
        .unwrap_err();
    assert!(matches!(
        health_err,
        ExtensionInstallationError::InstallationNotFound { .. }
    ));
}

#[tokio::test]
async fn manifest_hash_presence_mismatch_is_rejected() {
    let store = InMemoryExtensionInstallationStore::default();
    store.upsert_manifest(manifest("sha256:abc")).await.unwrap();

    let missing_ref_hash = store
        .upsert_installation(installation_with_manifest_hash(None))
        .await
        .unwrap_err();
    assert!(matches!(
        missing_ref_hash,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));

    let store = InMemoryExtensionInstallationStore::default();
    let manifest_without_hash = ExtensionManifestRecord::from_toml(
        raw_legacy_capability_manifest(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        None,
    )
    .unwrap();
    store.upsert_manifest(manifest_without_hash).await.unwrap();

    let unexpected_ref_hash = store
        .upsert_installation(installation_with_manifest_hash(Some("sha256:abc")))
        .await
        .unwrap_err();
    assert!(matches!(
        unexpected_ref_hash,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
}

#[test]
fn extension_health_message_redacts_public_renderings() {
    let message = ExtensionHealthMessage::new("provider stack trace with /host/path secret-token");
    let snapshot = ExtensionHealthSnapshot::new(
        ExtensionHealthStatus::Degraded,
        Some(message),
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );

    assert_eq!(
        format!("{:?}", snapshot.message().unwrap()),
        ExtensionHealthMessage::placeholder()
    );
    assert_eq!(
        snapshot.message().unwrap().to_string(),
        ExtensionHealthMessage::placeholder()
    );
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(json.contains(ExtensionHealthMessage::placeholder()));
    assert!(!json.contains("secret-token"));
    assert!(!json.contains("/host/path"));
}

#[test]
fn extension_health_message_round_trip_stays_redacted() {
    let json = r#"
{
  "status": "degraded",
  "message": "provider stack trace with /host/path secret-token",
  "checked_at": "2026-01-01T00:00:00Z"
}
"#;

    let snapshot: ExtensionHealthSnapshot = serde_json::from_str(json).unwrap();
    assert_eq!(
        snapshot.message().unwrap().to_string(),
        ExtensionHealthMessage::placeholder()
    );

    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(serialized.contains(ExtensionHealthMessage::placeholder()));
    assert!(!serialized.contains("secret-token"));
    assert!(!serialized.contains("/host/path"));
}

#[test]
fn extension_installation_identifiers_reject_empty_and_control_chars() {
    assert!(matches!(
        ManifestHash::new(""),
        Err(ExtensionInstallationError::InvalidValue { .. })
    ));
    assert!(matches!(
        ExtensionInstallationId::new("install\nbad"),
        Err(ExtensionInstallationError::InvalidValue { .. })
    ));
    assert!(matches!(
        ironclaw_extensions::ExtensionCredentialHandle::new("credential\rbad"),
        Err(ExtensionInstallationError::InvalidValue { .. })
    ));

    assert!(serde_json::from_str::<ManifestHash>("\"\"").is_err());
    assert!(serde_json::from_str::<ExtensionInstallationId>(r#""install\nbad""#).is_err());
    assert!(
        serde_json::from_str::<ironclaw_extensions::ExtensionCredentialHandle>(
            r#""credential\rbad""#
        )
        .is_err()
    );
}

#[test]
fn new_installation_uses_updated_at_for_initial_health_timestamp() {
    let updated_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let installation = ExtensionInstallation::new(
        installation_id("acme-tools-prod"),
        extension_id("acme-tools"),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(
            extension_id("acme-tools"),
            Some(manifest_hash("sha256:abc")),
        ),
        vec![],
        updated_at,
    )
    .unwrap();

    assert_eq!(installation.health().checked_at(), updated_at);
}

#[tokio::test]
async fn enabled_installations_sort_by_updated_at_desc_then_id() {
    let older = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let newer = chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let store = InMemoryExtensionInstallationStore::default();
    store.upsert_manifest(manifest("sha256:abc")).await.unwrap();

    for (id, updated_at) in [
        ("acme-tools-b", older),
        ("acme-tools-c", newer),
        ("acme-tools-a", older),
    ] {
        store
            .upsert_installation(
                ExtensionInstallation::new(
                    installation_id(id),
                    extension_id("acme-tools"),
                    ExtensionActivationState::Enabled,
                    ExtensionManifestRef::new(
                        extension_id("acme-tools"),
                        Some(manifest_hash("sha256:abc")),
                    ),
                    vec![],
                    updated_at,
                )
                .unwrap(),
            )
            .await
            .unwrap();
    }

    let ids: Vec<_> = store
        .list_enabled_installations()
        .await
        .unwrap()
        .into_iter()
        .map(|installation| installation.installation_id().as_str().to_owned())
        .collect();
    assert_eq!(ids, ["acme-tools-c", "acme-tools-a", "acme-tools-b"]);
}
