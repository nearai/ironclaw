use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthMessage, ExtensionHealthSnapshot,
    ExtensionHealthStatus, ExtensionInstallation, ExtensionInstallationError,
    ExtensionInstallationId, ExtensionInstallationStore, ExtensionManifestRecord,
    ExtensionManifestRef, InMemoryExtensionInstallationStore, MANIFEST_SCHEMA_VERSION,
    ManifestHash, ManifestSource,
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

fn manifest(hash: &str) -> ExtensionManifestRecord {
    let raw = format!(
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
    );
    ExtensionManifestRecord::from_toml(
        raw,
        ManifestSource::InstalledLocal,
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
