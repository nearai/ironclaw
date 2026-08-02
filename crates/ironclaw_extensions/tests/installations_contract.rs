// arch-exempt: large_file, filesystem-v2 migration and compatibility contract coverage remains colocated through rollout, plan #6637
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
use ironclaw_extensions::{
    CapabilityProviderHostApiContract, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationId,
    ExtensionInstallationPersistedParts, ExtensionInstallationStore,
    ExtensionInstallationStorePort, ExtensionManifestRecord, ExtensionManifestRef,
    HostApiContractRegistry, InstallationOwner, MANIFEST_SCHEMA_VERSION, ManifestHash,
    ManifestSource, ManifestV2Error, MembershipDeactivation,
};
use ironclaw_filesystem::{
    CasExpectation, Fault, FaultInjecting, FilesystemOperation, Filter, InMemoryBackend,
    LibSqlRootFilesystem, Page, PostgresRootFilesystem, RootFilesystem,
};
use ironclaw_host_api::{
    host_port::HostPortCatalog,
    ids::{ExtensionId, SecretHandle, UserId},
    path::VirtualPath,
};

fn extension_id(value: &str) -> ExtensionId {
    ExtensionId::new(value).unwrap()
}

fn installation_id(value: &str) -> ExtensionInstallationId {
    ExtensionInstallationId::new(value).unwrap()
}

fn manifest_hash(value: &str) -> ManifestHash {
    ManifestHash::new(value).unwrap()
}

async fn installation_store() -> ExtensionInstallationStore {
    ExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/test").unwrap(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap()
}

fn raw_capability_provider_manifest() -> String {
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
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

fn contracts() -> HostApiContractRegistry {
    let mut registry = HostApiContractRegistry::new();
    registry
        .register(std::sync::Arc::new(
            CapabilityProviderHostApiContract::new().unwrap(),
        ))
        .unwrap();
    registry
}

fn manifest(hash: &str) -> ExtensionManifestRecord {
    ExtensionManifestRecord::from_toml(
        raw_capability_provider_manifest(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        Some(manifest_hash(hash)),
        &contracts(),
        None,
    )
    .unwrap()
}

fn installation(hash: &str) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id("acme-tools-prod"),
        extension_id("acme-tools"),
        ExtensionManifestRef::new(extension_id("acme-tools"), Some(manifest_hash(hash))),
        vec![],
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .unwrap()
}

fn normalized_installation(hash: &str) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id("acme-tools-prod"),
        extension_id("acme-tools"),
        ExtensionManifestRef::new(extension_id("acme-tools"), Some(manifest_hash(hash))),
        vec![ExtensionCredentialBinding::new(
            ExtensionCredentialHandle::new("api-token").unwrap(),
            SecretHandle::new("secret-api-token").unwrap(),
        )],
        chrono::DateTime::parse_from_rfc3339("2026-07-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        InstallationOwner::user(UserId::new("alice").unwrap()),
    )
    .unwrap()
}

fn installation_with_manifest_hash(hash: Option<&str>) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id("acme-tools-prod"),
        extension_id("acme-tools"),
        ExtensionManifestRef::new(extension_id("acme-tools"), hash.map(manifest_hash)),
        vec![],
        Utc::now(),
        InstallationOwner::Tenant,
    )
    .unwrap()
}

#[test]
fn top_level_capabilities_are_rejected_for_every_source() {
    // The legacy manifest form is gone: capabilities are declared under an
    // ironclaw.capability_provider/v1 host_api section, for host-bundled
    // manifests exactly as for installed ones.
    let legacy = raw_capability_provider_manifest()
        .replace(
            "[[host_api]]\nid = \"ironclaw.capability_provider/v1\"\nsection = \"capability_provider.tools\"\n\n[capability_provider.tools]\n\n",
            "",
        )
        .replace("[[capability_provider.tools.capabilities]]", "[[capabilities]]");
    for source in [
        ManifestSource::InstalledLocal,
        ManifestSource::RegistryInstalled,
        ManifestSource::HostBundled,
    ] {
        let err = ExtensionManifestRecord::from_toml(
            legacy.clone(),
            source,
            &HostPortCatalog::empty(),
            Some(manifest_hash("sha256:abc")),
            &contracts(),
            None,
        )
        .unwrap_err();
        match err {
            ExtensionInstallationError::Manifest(ManifestV2Error::Invalid { reason }) => {
                assert!(
                    reason.contains("top-level [[capabilities]] is not supported"),
                    "{source:?}: {reason}"
                );
            }
            other => panic!("{source:?}: expected Invalid, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn upsert_installation_rejects_unknown_manifest() {
    let store = installation_store().await;

    let err = store
        .upsert_installation(
            ExtensionInstallation::new(
                installation_id("missing-prod"),
                extension_id("missing-tools"),
                ExtensionManifestRef::new(
                    extension_id("missing-tools"),
                    Some(manifest_hash("sha256:missing")),
                ),
                vec![],
                Utc::now(),
                InstallationOwner::Tenant,
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
async fn upsert_installation_rejects_hash_change_against_the_pinned_definition() {
    let store = installation_store().await;
    store
        .upsert_manifest_and_installation(manifest("sha256:old"), installation("sha256:old"))
        .await
        .unwrap();

    let err = store
        .upsert_installation(installation("sha256:new"))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
}

#[tokio::test]
async fn upsert_manifest_and_installation_replaces_coherent_manifest_hash_pair() {
    let store = installation_store().await;
    store
        .upsert_manifest_and_installation(manifest("sha256:old"), installation("sha256:old"))
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
    let store = installation_store().await;

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
    let store = installation_store().await;
    let missing = installation_id("missing-prod");

    let join_err = store
        .activate_membership(&missing, &UserId::new("alice").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        join_err,
        ExtensionInstallationError::InstallationNotFound { .. }
    ));

    let leave_err = store
        .deactivate_membership(&missing, &UserId::new("alice").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        leave_err,
        ExtensionInstallationError::InstallationNotFound { .. }
    ));
}

#[tokio::test]
async fn manifest_hash_presence_mismatch_is_rejected() {
    let store = installation_store().await;
    let missing_ref_hash = store
        .upsert_manifest_and_installation(
            manifest("sha256:abc"),
            installation_with_manifest_hash(None),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing_ref_hash,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));

    let store = installation_store().await;
    let manifest_without_hash = ExtensionManifestRecord::from_toml(
        raw_capability_provider_manifest(),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        None,
        &contracts(),
        None,
    )
    .unwrap();

    let unexpected_ref_hash = store
        .upsert_manifest_and_installation(
            manifest_without_hash,
            installation_with_manifest_hash(Some("sha256:abc")),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        unexpected_ref_hash,
        ExtensionInstallationError::ManifestHashMismatch { .. }
    ));
}

/// The released aggregate row carries a diagnostic `health` object that this
/// store does not model — extension failure state is owned by the host's
/// activation record. `ExtensionInstallation` denies unknown fields, so an
/// upgrade booting against an existing storage root only works while that
/// field is explicitly accepted and discarded. Without this, every legacy row
/// fails to deserialize and the whole extension catalog disappears on upgrade.
#[test]
fn released_aggregate_rows_carrying_health_still_deserialize() {
    let json = r#"
{
  "installation_id": "acme-tools",
  "extension_id": "acme-tools",
  "manifest_ref": { "extension_id": "acme-tools", "manifest_hash": "sha256:abc" },
  "credential_bindings": [],
  "health": { "status": "healthy", "message": null, "checked_at": "2026-01-01T00:00:00Z" },
  "updated_at": "2026-01-01T00:00:00Z",
  "owner": { "kind": "users", "user_ids": ["alice"] }
}
"#;

    let installation: ExtensionInstallation = serde_json::from_str(json).unwrap();
    assert_eq!(installation.extension_id().as_str(), "acme-tools");
    assert_eq!(
        installation.owner().members().unwrap(),
        &BTreeSet::from([UserId::new("alice").unwrap()]),
        "membership must survive an upgrade from the released row shape"
    );
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
fn persisted_reconstruction_preserves_timestamp_and_bindings() {
    let extension_id = extension_id("acme-tools");
    let updated_at = chrono::DateTime::parse_from_rfc3339("2026-01-03T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let binding = ExtensionCredentialBinding::new(
        ExtensionCredentialHandle::new("api").unwrap(),
        SecretHandle::new("api-secret").unwrap(),
    );
    let owner = InstallationOwner::users(BTreeSet::from([UserId::new("alice").unwrap()])).unwrap();

    let installation =
        ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id: installation_id("acme-tools"),
            extension_id: extension_id.clone(),
            manifest_ref: ExtensionManifestRef::new(extension_id, None),
            incarnation_id: None,
            credential_bindings: vec![binding.clone()],
            updated_at,
            owner: owner.clone(),
        })
        .unwrap();

    assert_eq!(installation.updated_at(), updated_at);
    assert_eq!(installation.credential_bindings(), &[binding]);
    assert_eq!(installation.owner(), &owner);
}

#[tokio::test]
async fn installations_sort_by_id() {
    let older = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let newer = chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let store = installation_store().await;

    for (id, updated_at) in [
        ("acme-tools-b", older),
        ("acme-tools-c", newer),
        ("acme-tools-a", older),
    ] {
        store
            .upsert_manifest_and_installation(
                manifest("sha256:abc"),
                ExtensionInstallation::new(
                    installation_id(id),
                    extension_id("acme-tools"),
                    ExtensionManifestRef::new(
                        extension_id("acme-tools"),
                        Some(manifest_hash("sha256:abc")),
                    ),
                    vec![],
                    updated_at,
                    InstallationOwner::Tenant,
                )
                .unwrap(),
            )
            .await
            .unwrap();
    }

    let ids: Vec<_> = store
        .list_installations()
        .await
        .unwrap()
        .into_iter()
        .map(|installation| installation.installation_id().as_str().to_owned())
        .collect();
    assert_eq!(ids, ["acme-tools-a", "acme-tools-b", "acme-tools-c"]);
}

#[tokio::test]
async fn installation_store_persists_manifest_and_installation_as_rows() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let root = VirtualPath::new("/system/extensions/.installations/reload").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        ironclaw_extensions::HostApiContractRegistry::new(),
    )
    .await
    .unwrap();

    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installation("sha256:abc"))
        .await
        .unwrap();

    let manifest_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/manifests", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    assert_eq!(manifest_rows.len(), 1);
    assert_eq!(
        manifest_rows[0].entry.kind.as_ref().unwrap().as_str(),
        "extension_manifest_record"
    );
    let installation_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/installations", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    assert_eq!(installation_rows.len(), 1);
    assert_eq!(
        installation_rows[0].entry.kind.as_ref().unwrap().as_str(),
        "extension_installation_record"
    );

    let reloaded = ExtensionInstallationStore::load_at(
        filesystem,
        root,
        HostPortCatalog::empty(),
        ironclaw_extensions::HostApiContractRegistry::new(),
    )
    .await
    .unwrap();
    assert!(
        reloaded
            .get_installation(&installation_id("acme-tools-prod"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn normalized_v2_layout_separates_mutable_extension_state_and_reopens_the_aggregate() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let root = VirtualPath::new("/system/extensions/.installations/normalized-v2").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let expected = normalized_installation("sha256:abc");

    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), expected.clone())
        .await
        .unwrap();

    let collection_rows = [
        ("installations", "extension_installation_record_v2"),
        ("memberships", "extension_membership_record_v2"),
        (
            "credential-bindings",
            "extension_credential_binding_record_v2",
        ),
    ];
    for (collection, expected_kind) in collection_rows {
        let rows = filesystem
            .query(
                &VirtualPath::new(format!("{}/v2/{collection}", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "expected one v2 {collection} row");
        assert_eq!(
            rows[0].entry.kind.as_ref().unwrap().as_str(),
            expected_kind,
            "unexpected {collection} record kind"
        );
        let body: serde_json::Value = rows[0].entry.parse_json().unwrap();
        assert_eq!(body["schema_version"], "extension_state.v2");
    }

    let installation_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/installations", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    let core: serde_json::Value = installation_rows[0].entry.parse_json().unwrap();
    for aggregate_field in [
        "owner",
        "credential_bindings",
        "health",
        "status",
        "installed_at",
    ] {
        assert!(
            core.get(aggregate_field).is_none(),
            "installation record must not carry {aggregate_field}: {core}"
        );
    }
    assert_eq!(
        core["manifest"]["manifest_hash"], "sha256:abc",
        "the installation record embeds the hash-pinned install definition"
    );
    assert!(
        core["manifest"]["raw_toml"].is_string(),
        "the installation record is the install pin and carries the definition"
    );

    let membership_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/memberships", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    let membership: serde_json::Value = membership_rows[0].entry.parse_json().unwrap();
    assert_eq!(membership["installation_id"], "acme-tools-prod");
    assert_eq!(membership["user_id"], "alice");
    assert!(
        membership["removed_at"].is_null(),
        "an active membership has no tombstone"
    );
    assert_eq!(
        membership_rows[0]
            .path
            .as_str()
            .trim_start_matches(format!("{}/v2/memberships/", root.as_str()).as_str())
            .split('/')
            .count(),
        2,
        "membership rows must be nested by installation then user token"
    );

    let binding_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/credential-bindings", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    let binding: serde_json::Value = binding_rows[0].entry.parse_json().unwrap();
    assert_eq!(binding["installation_id"], "acme-tools-prod");
    assert_eq!(binding["credential_handle"], "api-token");
    assert_eq!(binding["secret_handle"], "secret-api-token");
    assert_eq!(
        binding_rows[0]
            .path
            .as_str()
            .trim_start_matches(format!("{}/v2/credential-bindings/", root.as_str()).as_str())
            .split('/')
            .count(),
        2,
        "binding rows must be nested by installation then binding token"
    );

    let all_v2_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(20),
        )
        .await
        .unwrap();
    assert_eq!(all_v2_rows.len(), 3);
    assert!(
        all_v2_rows
            .iter()
            .all(|row| row.entry.kind.is_some()
                && row.entry.content_type.as_str() == "application/json"),
        "v2 lifecycle state must contain records, not package bytes"
    );

    let reloaded = ExtensionInstallationStore::load_at(
        filesystem,
        root,
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    assert_eq!(
        reloaded
            .get_installation(&installation_id("acme-tools-prod"))
            .await
            .unwrap(),
        Some(expected)
    );
}

#[tokio::test]
async fn normalized_v2_soft_removal_preserves_records_and_allows_reactivation() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let root = VirtualPath::new("/system/extensions/.installations/normalized-v2-remove").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();

    store
        .delete_installation(installed.installation_id())
        .await
        .unwrap();
    assert!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none(),
        "soft-removed installations are hidden from the live store view"
    );

    for (collection, tombstoned) in [
        ("installations", true),
        ("memberships", true),
        ("credential-bindings", true),
    ] {
        let rows = filesystem
            .query(
                &VirtualPath::new(format!("{}/v2/{collection}", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "soft removal must retain the {collection} row"
        );
        if tombstoned {
            let body: serde_json::Value = rows[0].entry.parse_json().unwrap();
            assert!(
                body["removed_at"].is_string(),
                "soft removal tombstones the {collection} row: {body}"
            );
        }
    }

    store.upsert_installation(installed.clone()).await.unwrap();
    assert_eq!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap(),
        Some(installed.clone()),
        "reinstall must reactivate the retained record identities"
    );

    store
        .delete_installation(installed.installation_id())
        .await
        .unwrap();
    store
        .delete_manifest(installed.extension_id())
        .await
        .unwrap();
    assert!(
        store
            .get_manifest(installed.extension_id())
            .await
            .unwrap()
            .is_none()
    );
    let record_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/installations", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    assert_eq!(record_rows.len(), 1);
    let record_body: serde_json::Value = record_rows[0].entry.parse_json().unwrap();
    assert!(
        record_body["removed_at"].is_string(),
        "the tombstoned record keeps its removal timestamp"
    );
    assert!(
        record_body["manifest"]["raw_toml"].is_string(),
        "the tombstoned record retains its pinned definition for reactivation"
    );
}

#[tokio::test]
async fn normalized_v2_bootstrap_imports_legacy_rows_and_repairs_compatibility_views() {
    let source_filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let source_root = VirtualPath::new("/system/extensions/.installations/legacy-source").unwrap();
    let source_store = ExtensionInstallationStore::load_at(
        Arc::clone(&source_filesystem),
        source_root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    source_store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();

    let target_filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let target_root = VirtualPath::new("/system/extensions/.installations/legacy-target").unwrap();
    for collection in ["manifests", "installations"] {
        let source_prefix =
            VirtualPath::new(format!("{}/{collection}", source_root.as_str())).unwrap();
        let rows = source_filesystem
            .query(&source_prefix, &Filter::All, Page::first(10))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "legacy fixture must contain {collection}");
        let relative = rows[0]
            .path
            .as_str()
            .strip_prefix(source_root.as_str())
            .unwrap();
        let target_path = VirtualPath::new(format!("{}{relative}", target_root.as_str())).unwrap();
        target_filesystem
            .put(&target_path, rows[0].entry.clone(), CasExpectation::Absent)
            .await
            .unwrap();
    }

    let migrated = ExtensionInstallationStore::load_at(
        Arc::clone(&target_filesystem),
        target_root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    assert_eq!(
        migrated
            .get_installation(installed.installation_id())
            .await
            .unwrap(),
        Some(installed)
    );
    let v2_rows = target_filesystem
        .query(
            &VirtualPath::new(format!("{}/v2", target_root.as_str())).unwrap(),
            &Filter::All,
            Page::first(20),
        )
        .await
        .unwrap();
    assert_eq!(v2_rows.len(), 3, "legacy aggregate must expand into v2");

    let compatibility_rows = target_filesystem
        .query(
            &VirtualPath::new(format!("{}/installations", target_root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    assert_eq!(compatibility_rows.len(), 1);
    target_filesystem
        .delete(&compatibility_rows[0].path)
        .await
        .unwrap();

    let reopened = ExtensionInstallationStore::load_at(
        Arc::clone(&target_filesystem),
        target_root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    assert!(
        reopened
            .get_installation(&installation_id("acme-tools-prod"))
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        target_filesystem
            .query(
                &VirtualPath::new(format!("{}/installations", target_root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .len(),
        1,
        "startup must repair the compatibility aggregate from v2"
    );
}

#[tokio::test]
async fn interrupted_v2_install_stays_invisible_and_retry_repairs_it() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/v2/installations/")
                .nth(1)
                .backend("interrupt before installation commit"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root = VirtualPath::new("/system/extensions/.installations/interrupted-v2").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let expected = normalized_installation("sha256:abc");

    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), expected.clone())
        .await
        .expect_err("the injected core-row failure must interrupt the first install");

    assert!(
        store
            .get_manifest(&extension_id("acme-tools"))
            .await
            .unwrap()
            .is_none(),
        "a failed paired install must restore the prior manifest state"
    );
    assert!(
        store
            .get_installation(expected.installation_id())
            .await
            .unwrap()
            .is_none(),
        "children written before the core commit must not expose a partial installation"
    );
    assert_eq!(
        filesystem
            .query(
                &VirtualPath::new(format!("{}/v2/memberships", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .len(),
        1,
        "the interrupted child row remains available for an idempotent repair"
    );

    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), expected.clone())
        .await
        .expect("retry completes the normalized installation");
    assert_eq!(
        store
            .get_installation(expected.installation_id())
            .await
            .unwrap()
            .unwrap(),
        expected
    );
}

#[tokio::test]
async fn interrupted_active_aggregate_update_rolls_back_before_readers_resume() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/v2/credential-bindings/")
                .nth(2)
                .backend("interrupt an active aggregate update after membership writes"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root = VirtualPath::new("/system/extensions/.installations/interrupted-update").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();
    let updated = installed.clone().with_owner(
        InstallationOwner::users(BTreeSet::from([
            UserId::new("alice").unwrap(),
            UserId::new("bob").unwrap(),
        ]))
        .unwrap(),
    );

    store
        .upsert_installation(updated)
        .await
        .expect_err("the injected binding failure must interrupt the aggregate update");

    let visible = store
        .get_installation(installed.installation_id())
        .await
        .unwrap()
        .expect("the prior aggregate must be restored before the error returns");
    assert_eq!(
        visible.owner().members().unwrap(),
        &BTreeSet::from([UserId::new("alice").unwrap()])
    );
    let membership_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/memberships", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    let bob = membership_rows
        .iter()
        .map(|row| row.entry.parse_json::<serde_json::Value>().unwrap())
        .find(|body| body["user_id"] == "bob")
        .expect("the interrupted child remains as a tombstone");
    assert!(
        bob["removed_at"].is_string(),
        "bob remains a tombstone: {bob}"
    );
}

#[tokio::test]
async fn interrupted_soft_removal_is_idempotently_completed_by_same_store_retry() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/v2/memberships/")
                .nth(2)
                .backend("interrupt after installation tombstone"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root = VirtualPath::new("/system/extensions/.installations/interrupted-remove").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();

    store
        .delete_installation(installed.installation_id())
        .await
        .expect_err("the injected child-row failure must interrupt removal");
    assert!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none(),
        "the authoritative core tombstone hides an interrupted removal"
    );

    store
        .delete_installation(installed.installation_id())
        .await
        .expect("an immediate retry resumes the removal");
    assert!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none()
    );
    for collection in ["memberships", "credential-bindings"] {
        let rows = filesystem
            .query(
                &VirtualPath::new(format!("{}/v2/{collection}", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let body: serde_json::Value = rows[0].entry.parse_json().unwrap();
        assert!(
            body["removed_at"].is_string(),
            "the retry must finish the {collection} tombstone: {body}"
        );
    }
}

#[tokio::test]
async fn interrupted_soft_removal_rolls_back_from_compatibility_snapshot_on_reopen() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/v2/memberships/")
                .nth(2)
                .backend("interrupt after removal reservation"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root =
        VirtualPath::new("/system/extensions/.installations/interrupted-remove-reopen").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();
    store
        .delete_installation(installed.installation_id())
        .await
        .expect_err("the injected child-row failure must interrupt removal");
    drop(store);

    let reopened = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .expect("startup rolls the reserved removal back to its compatibility snapshot");
    assert_eq!(
        reopened
            .get_installation(installed.installation_id())
            .await
            .unwrap(),
        Some(installed)
    );
    for collection in ["memberships", "credential-bindings"] {
        let rows = filesystem
            .query(
                &VirtualPath::new(format!("{}/v2/{collection}", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap();
        let body: serde_json::Value = rows[0].entry.parse_json().unwrap();
        assert!(
            body["removed_at"].is_null(),
            "startup must restore the {collection} row from the compatibility snapshot: {body}"
        );
    }
}

async fn assert_normalized_backend_contract(
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
) {
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let expected = normalized_installation("sha256:abc");
    let alice = UserId::new("alice").unwrap();
    let bob = UserId::new("bob").unwrap();
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), expected.clone())
        .await
        .unwrap();
    store
        .activate_membership(expected.installation_id(), &bob)
        .await
        .unwrap();
    store
        .deactivate_membership(expected.installation_id(), &alice)
        .await
        .unwrap();

    let before_refresh = store
        .get_installation(expected.installation_id())
        .await
        .unwrap()
        .unwrap();
    let expected_ref = before_refresh.manifest_ref().clone();
    let expected_updated_at = before_refresh.updated_at();
    store
        .upsert_manifest_only(
            expected.installation_id(),
            &expected_ref,
            expected_updated_at,
            manifest("sha256:refresh"),
        )
        .await
        .unwrap();
    let refreshed = store
        .get_installation(expected.installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.installation_id(), expected.installation_id());
    assert_eq!(refreshed.extension_id(), expected.extension_id());
    assert_eq!(refreshed.incarnation_id(), expected.incarnation_id());
    assert_eq!(
        refreshed.credential_bindings(),
        expected.credential_bindings()
    );
    assert_eq!(
        refreshed.owner().members().unwrap(),
        &BTreeSet::from([bob.clone()])
    );

    let mismatched_ref = ExtensionManifestRef::new(extension_id("other-tools"), None);
    let mismatched = store
        .upsert_manifest_only(
            expected.installation_id(),
            &mismatched_ref,
            refreshed.updated_at(),
            manifest("sha256:stale"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        mismatched,
        ExtensionInstallationError::ManifestExtensionMismatch { .. }
    ));

    let stale = store
        .upsert_manifest_only(
            expected.installation_id(),
            &expected_ref,
            expected_updated_at,
            manifest("sha256:stale"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        ExtensionInstallationError::PreparationFinalizationRejected { .. }
    ));

    let refresh_ref = refreshed.manifest_ref().clone();
    let refresh_updated_at = refreshed.updated_at();
    store
        .activate_membership(expected.installation_id(), &alice)
        .await
        .unwrap();
    store
        .upsert_manifest_only(
            expected.installation_id(),
            &refresh_ref,
            refresh_updated_at,
            manifest("sha256:after-membership"),
        )
        .await
        .unwrap();
    let after_concurrent_refresh = store
        .get_installation(expected.installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_concurrent_refresh.owner().members().unwrap(),
        &BTreeSet::from([alice.clone(), bob.clone()])
    );
    assert_eq!(
        after_concurrent_refresh.manifest_ref().manifest_hash(),
        Some(&manifest_hash("sha256:after-membership"))
    );
    drop(store);

    let reopened = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installation = reopened
        .get_installation(expected.installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        installation.owner().members().unwrap(),
        &BTreeSet::from([alice, bob])
    );
    assert_eq!(
        reopened
            .get_manifest(expected.extension_id())
            .await
            .unwrap()
            .unwrap()
            .manifest_hash(),
        Some(&manifest_hash("sha256:after-membership"))
    );

    reopened
        .delete_installation(expected.installation_id())
        .await
        .unwrap();
    reopened
        .delete_manifest(expected.extension_id())
        .await
        .unwrap();
    assert!(
        reopened
            .get_installation(expected.installation_id())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        reopened
            .get_manifest(expected.extension_id())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        filesystem
            .query(
                &VirtualPath::new(format!("{}/v2/memberships", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .len(),
        2,
        "both users' soft-removed membership history remains queryable"
    );
}

#[tokio::test]
async fn normalized_v2_contract_runs_on_libsql() {
    let dir = tempfile::tempdir().unwrap();
    let database = Arc::new(
        libsql::Builder::new_local(dir.path().join("extensions.db"))
            .build()
            .await
            .unwrap(),
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(database).expect("filesystem runtime"));
    filesystem.run_migrations().await.unwrap();
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    assert_normalized_backend_contract(
        filesystem,
        VirtualPath::new("/system/extensions/.installations/libsql-v2").unwrap(),
    )
    .await;
}

#[tokio::test]
async fn normalized_v2_contract_runs_on_postgres_when_configured() {
    let Ok(url) = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL") else {
        eprintln!(
            "skipping Postgres extension-state contract: \
             IRONCLAW_FILESYSTEM_POSTGRES_URL not set"
        );
        return;
    };
    let config = url.parse::<tokio_postgres::Config>().unwrap();
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .unwrap();
    if let Err(error) = pool.get().await {
        eprintln!("skipping Postgres extension-state contract: database unavailable ({error})");
        return;
    }
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    filesystem.run_migrations().await.unwrap();
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    assert_normalized_backend_contract(
        filesystem,
        VirtualPath::new(format!(
            "/system/extensions/.installations/postgres-v2-{suffix}"
        ))
        .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn membership_v2_activation_and_deactivation_mutate_only_the_target_user_row() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let root = VirtualPath::new("/system/extensions/.installations/membership-v2").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();
    let bob = UserId::new("bob").unwrap();
    let alice = UserId::new("alice").unwrap();

    store
        .activate_membership(installed.installation_id(), &bob)
        .await
        .unwrap();
    store
        .activate_membership(installed.installation_id(), &bob)
        .await
        .unwrap();
    let joined = store
        .get_installation(installed.installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        joined.owner().members().unwrap(),
        &BTreeSet::from([alice.clone(), bob.clone()])
    );

    store
        .deactivate_membership(installed.installation_id(), &alice)
        .await
        .unwrap();
    store
        .deactivate_membership(installed.installation_id(), &alice)
        .await
        .unwrap();
    let remaining = store
        .get_installation(installed.installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        remaining.owner().members().unwrap(),
        &BTreeSet::from([bob.clone()])
    );

    let rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/memberships", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    let tombstoned = rows
        .iter()
        .map(|row| {
            let body: serde_json::Value = row.entry.parse_json().unwrap();
            (
                body["user_id"].as_str().unwrap().to_string(),
                body["removed_at"].is_string(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(tombstoned.get("alice"), Some(&true));
    assert_eq!(tombstoned.get("bob"), Some(&false));

    let final_member_result = store
        .deactivate_membership(installed.installation_id(), &bob)
        .await
        .unwrap();
    assert_eq!(
        final_member_result,
        MembershipDeactivation::FinalMemberReserved
    );
    assert!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none(),
        "the final-member reservation must hide the aggregate until removal or rollback"
    );
    store
        .upsert_installation(remaining)
        .await
        .expect("restoring the reserved aggregate must be idempotent");
}

#[tokio::test]
async fn membership_deactivation_backend_failure_restores_the_prior_aggregate() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::Query)
                .path("/v2/memberships/")
                .nth(3)
                .backend("interrupt after the removal reservation"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root =
        VirtualPath::new("/system/extensions/.installations/deactivation-compensation").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root,
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let alice = UserId::new("alice").unwrap();
    let bob = UserId::new("bob").unwrap();
    let installed = normalized_installation("sha256:abc").with_owner(
        InstallationOwner::users(BTreeSet::from([alice.clone(), bob.clone()])).unwrap(),
    );
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();

    assert!(matches!(
        store
            .deactivate_membership(installed.installation_id(), &alice)
            .await
            .unwrap_err(),
        ExtensionInstallationError::StoreUnavailable { .. }
    ));
    assert_eq!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .unwrap()
            .owner()
            .members()
            .unwrap(),
        &BTreeSet::from([alice.clone(), bob.clone()])
    );
    assert!(matches!(
        store
            .deactivate_membership(installed.installation_id(), &alice)
            .await
            .unwrap(),
        MembershipDeactivation::MembershipRemoved(_)
    ));
}

#[tokio::test]
async fn membership_v2_concurrent_distinct_activations_lose_no_members() {
    let store = Arc::new(installation_store().await);
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();
    let bob = UserId::new("bob").unwrap();
    let carol = UserId::new("carol").unwrap();

    let (bob_result, carol_result) = tokio::join!(
        store.activate_membership(installed.installation_id(), &bob),
        store.activate_membership(installed.installation_id(), &carol),
    );
    bob_result.unwrap();
    carol_result.unwrap();

    let reloaded = store
        .get_installation(installed.installation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded.owner().members().unwrap(),
        &BTreeSet::from([UserId::new("alice").unwrap(), bob, carol])
    );
}

#[tokio::test]
async fn final_member_reservation_blocks_a_join_from_a_second_store() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let root = VirtualPath::new("/system/extensions/.installations/final-member-race").unwrap();
    let first = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let second = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root,
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    let bob = UserId::new("bob").unwrap();
    first
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();

    assert_eq!(
        first
            .deactivate_membership(installed.installation_id(), &UserId::new("alice").unwrap())
            .await
            .unwrap(),
        MembershipDeactivation::FinalMemberReserved
    );
    assert!(matches!(
        second
            .activate_membership(installed.installation_id(), &bob)
            .await
            .unwrap_err(),
        ExtensionInstallationError::MembershipMutationInProgress { .. }
    ));
    first
        .delete_installation(installed.installation_id())
        .await
        .unwrap();
    assert!(
        first
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none()
    );
}

/// Review finding: once the v2 records (the authority) are committed, a
/// failure writing the legacy compatibility projection must not fail the
/// operation — the caller would compensate against a live install and leave a
/// ghost. The projection is repaired at the next startup instead.
#[tokio::test]
async fn aggregate_projection_write_failure_does_not_fail_a_committed_v2_install() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("projection-loss/installations/")
                .nth(1)
                .backend("interrupt the compatibility projection write"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root = VirtualPath::new("/system/extensions/.installations/projection-loss").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let expected = normalized_installation("sha256:abc");

    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), expected.clone())
        .await
        .expect("a committed v2 install succeeds even when the projection write fails");
    assert_eq!(
        store
            .get_installation(expected.installation_id())
            .await
            .unwrap(),
        Some(expected.clone())
    );
    assert!(
        filesystem
            .query(
                &VirtualPath::new(format!("{}/installations", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .is_empty(),
        "the failed projection write leaves no legacy row behind"
    );

    drop(store);
    let reopened = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened
            .get_installation(expected.installation_id())
            .await
            .unwrap(),
        Some(expected)
    );
    assert_eq!(
        filesystem
            .query(
                &VirtualPath::new(format!("{}/installations", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .len(),
        1,
        "startup must repair the compatibility projection from v2"
    );
}

/// A removal that has not converged keeps the extension's definition
/// authoritative: `delete_installation` leaves a cleanup-pending tombstone
/// whose manifest stays visible (so removal retries work without the catalog
/// and imports stay blocked), and `delete_manifest` marks convergence, after
/// which the definition stops being served. `persist_removal_tombstone` seeds
/// the same state for orphan cleanups that never had an installation row.
#[tokio::test]
async fn removal_tombstone_keeps_manifest_authoritative_until_convergence() {
    let store = installation_store().await;
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();

    store
        .delete_installation(installed.installation_id())
        .await
        .unwrap();
    assert!(
        store
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_manifest(installed.extension_id())
            .await
            .unwrap()
            .is_some(),
        "the definition stays authoritative while removal cleanup is pending"
    );

    store
        .delete_manifest(installed.extension_id())
        .await
        .unwrap();
    assert!(
        store
            .get_manifest(installed.extension_id())
            .await
            .unwrap()
            .is_none(),
        "convergence stops serving the definition"
    );

    let orphan_store = installation_store().await;
    orphan_store
        .persist_removal_tombstone(manifest("sha256:abc"))
        .await
        .unwrap();
    assert!(
        orphan_store
            .get_manifest(&extension_id("acme-tools"))
            .await
            .unwrap()
            .is_some(),
        "an orphan cleanup tombstone serves its retained definition"
    );
    assert!(
        orphan_store
            .get_installation(&installation_id("acme-tools"))
            .await
            .unwrap()
            .is_none(),
        "a cleanup tombstone is not an installation"
    );
    orphan_store
        .delete_manifest(&extension_id("acme-tools"))
        .await
        .unwrap();
    assert!(
        orphan_store
            .get_manifest(&extension_id("acme-tools"))
            .await
            .unwrap()
            .is_none()
    );
}

/// Review finding: an orphaned legacy manifest row with no installation row is
/// the legacy flow's durable removal-cleanup marker. Bootstrap must import it
/// as a cleanup-pending tombstone so the interrupted removal stays retryable
/// (and imports stay blocked) after the migration, exactly as it was before.
#[tokio::test]
async fn bootstrap_imports_orphan_legacy_manifest_as_cleanup_pending_tombstone() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let root = VirtualPath::new("/system/extensions/.installations/orphan-manifest").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .unwrap();
    store
        .delete_installation(installed.installation_id())
        .await
        .unwrap();
    drop(store);

    // Reduce the root to the exact legacy-crash shape: an orphan legacy
    // manifest row with no v2 authority record behind it.
    let v2_rows = filesystem
        .query(
            &VirtualPath::new(format!("{}/v2/installations", root.as_str())).unwrap(),
            &Filter::All,
            Page::first(10),
        )
        .await
        .unwrap();
    assert_eq!(v2_rows.len(), 1);
    filesystem.delete(&v2_rows[0].path).await.unwrap();
    assert_eq!(
        filesystem
            .query(
                &VirtualPath::new(format!("{}/manifests", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .len(),
        1,
        "the orphan legacy manifest row is the migration input"
    );

    let migrated = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    assert!(
        migrated
            .get_manifest(installed.extension_id())
            .await
            .unwrap()
            .is_some(),
        "the orphan legacy manifest imports as an authoritative cleanup tombstone"
    );
    assert!(
        migrated
            .get_installation(installed.installation_id())
            .await
            .unwrap()
            .is_none(),
        "the imported tombstone is not a live installation"
    );
    migrated
        .delete_manifest(installed.extension_id())
        .await
        .unwrap();
    assert!(
        migrated
            .get_manifest(installed.extension_id())
            .await
            .unwrap()
            .is_none(),
        "convergence retires the imported tombstone"
    );
}

/// Review finding: a missing compatibility row is not proof of removal —
/// successful v2 writes deliberately tolerate failed projection writes. A
/// record caught with a dangling lease and no compatibility snapshot must
/// converge by clearing the lease over its surviving children, never by
/// tombstoning live state.
#[tokio::test]
async fn interrupted_update_without_compatibility_snapshot_stays_live_on_reopen() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new())
            .with_fault(
                Fault::on(FilesystemOperation::WriteFile)
                    .path("no-snapshot/installations/")
                    .nth(1)
                    .backend("suppress the compatibility projection at install"),
            )
            .with_fault(
                Fault::on(FilesystemOperation::WriteFile)
                    .path("/v2/credential-bindings/")
                    .nth(2)
                    .backend("interrupt the aggregate update forward pass"),
            )
            .with_fault(
                Fault::on(FilesystemOperation::WriteFile)
                    .path("/v2/installations/")
                    .nth(3)
                    .backend("interrupt the aggregate update rollback at its record commit"),
            ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root = VirtualPath::new("/system/extensions/.installations/no-snapshot").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let installed = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), installed.clone())
        .await
        .expect("install succeeds despite the suppressed projection write");
    assert!(
        filesystem
            .query(
                &VirtualPath::new(format!("{}/installations", root.as_str())).unwrap(),
                &Filter::All,
                Page::first(10),
            )
            .await
            .unwrap()
            .is_empty(),
        "the compatibility snapshot is genuinely absent"
    );

    let updated = installed.clone().with_owner(
        InstallationOwner::users(BTreeSet::from([
            UserId::new("alice").unwrap(),
            UserId::new("bob").unwrap(),
        ]))
        .unwrap(),
    );
    store
        .upsert_installation(updated)
        .await
        .expect_err("the injected faults interrupt the update and its rollback");
    drop(store);

    let reopened = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .expect("startup converges the dangling lease");
    let survivor = reopened
        .get_installation(installed.installation_id())
        .await
        .unwrap()
        .expect("live state survives: a missing snapshot must not tombstone the record");
    assert!(
        survivor
            .owner()
            .members()
            .expect("member-owned installation")
            .contains(&UserId::new("alice").unwrap()),
        "the prior member survives lease recovery"
    );
}

/// Review finding: a fresh install that fails after writing its membership
/// row leaves debris (no installation record ever committed). A later
/// installer's successful install must not merge that debris into its member
/// set — a user whose install failed would silently gain membership.
#[tokio::test]
async fn failed_install_debris_is_not_granted_to_a_later_installer() {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/v2/installations/")
                .nth(1)
                .backend("interrupt alice's install before its record commit"),
        ),
    );
    let filesystem: Arc<dyn RootFilesystem> = backend;
    let root = VirtualPath::new("/system/extensions/.installations/install-debris").unwrap();
    let store = ExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        root.clone(),
        HostPortCatalog::empty(),
        contracts(),
    )
    .await
    .unwrap();
    let alice_install = normalized_installation("sha256:abc");
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), alice_install.clone())
        .await
        .expect_err("alice's install is interrupted before the record commits");

    let bob = UserId::new("bob").unwrap();
    let bob_install = alice_install
        .clone()
        .with_owner(InstallationOwner::user(bob.clone()));
    store
        .upsert_manifest_and_installation(manifest("sha256:abc"), bob_install)
        .await
        .expect("bob's fresh install succeeds");

    let installed = store
        .get_installation(&installation_id("acme-tools-prod"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        installed.owner().members().unwrap(),
        &BTreeSet::from([bob]),
        "debris from alice's failed install must not grant her membership"
    );
}

/// Perf contract: listing installations must not issue a query per
/// installation. The normalized layout split one aggregate row into an
/// installation row plus child membership/binding rows, and reconstructing
/// each installation with its own child queries makes `list_installations`
/// O(N) round trips. On a local SSD that is invisible (fractions of a
/// millisecond); on network-attached storage it dominates -- it took the
/// hosted `/api/webchat/v2/extensions` read to ~2s. Pinning "the query count
/// does not grow with the number of installations" is what catches a
/// regression here, because wall-clock on a dev machine never will.
#[tokio::test]
async fn listing_installations_does_not_issue_a_query_per_installation() {
    async fn queries_to_list(installation_count: usize) -> usize {
        let counting = Arc::new(QueryCountingBackend::new(InMemoryBackend::new()));
        let filesystem: Arc<dyn RootFilesystem> = counting.clone();
        let store = ExtensionInstallationStore::load_at(
            Arc::clone(&filesystem),
            VirtualPath::new("/system/extensions/.installations/query-count").unwrap(),
            HostPortCatalog::empty(),
            contracts(),
        )
        .await
        .unwrap();

        for index in 0..installation_count {
            let ext = format!("acme-tools-{index}");
            let toml = raw_capability_provider_manifest()
                .replace("id = \"acme-tools\"", &format!("id = \"{ext}\""))
                .replace("acme-tools.echo", &format!("{ext}.echo"));
            let record = ExtensionManifestRecord::from_toml(
                toml,
                ManifestSource::HostBundled,
                &HostPortCatalog::empty(),
                Some(manifest_hash("sha256:abc")),
                &contracts(),
                None,
            )
            .unwrap();
            let installation = ExtensionInstallation::new(
                installation_id(&ext),
                extension_id(&ext),
                ExtensionManifestRef::new(extension_id(&ext), Some(manifest_hash("sha256:abc"))),
                vec![ExtensionCredentialBinding::new(
                    ExtensionCredentialHandle::new("api-token").unwrap(),
                    SecretHandle::new("secret-api-token").unwrap(),
                )],
                Utc::now(),
                InstallationOwner::user(UserId::new("alice").unwrap()),
            )
            .unwrap();
            store
                .upsert_manifest_and_installation(record, installation)
                .await
                .unwrap();
        }

        counting.reset();
        let listed = store.list_installations().await.unwrap();
        assert_eq!(listed.len(), installation_count, "all installations listed");
        // every listed aggregate is still fully reconstructed
        for installation in &listed {
            assert_eq!(installation.credential_bindings().len(), 1);
            assert_eq!(installation.owner().members().unwrap().len(), 1);
        }
        counting.queries()
    }

    let few = queries_to_list(2).await;
    let many = queries_to_list(8).await;
    assert_eq!(
        few, many,
        "list_installations must issue a constant number of queries; \
         got {few} for 2 installations and {many} for 8 (O(N) round trips)"
    );
}

/// Counts `query` calls so a test can assert round-trip complexity rather
/// than wall-clock, which is meaningless on a local disk.
struct QueryCountingBackend {
    inner: InMemoryBackend,
    queries: std::sync::atomic::AtomicUsize,
}

impl QueryCountingBackend {
    fn new(inner: InMemoryBackend) -> Self {
        Self {
            inner,
            queries: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn reset(&self) {
        self.queries.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    fn queries(&self) -> usize {
        self.queries.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl RootFilesystem for QueryCountingBackend {
    fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: ironclaw_filesystem::Entry,
        cas: CasExpectation,
    ) -> Result<ironclaw_filesystem::RecordVersion, ironclaw_filesystem::FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, ironclaw_filesystem::FilesystemError>
    {
        self.inner.get(path).await
    }

    async fn list_dir(
        &self,
        path: &VirtualPath,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, ironclaw_filesystem::FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<ironclaw_filesystem::VersionedEntry>, ironclaw_filesystem::FilesystemError>
    {
        self.queries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.query(path, filter, page).await
    }

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, ironclaw_filesystem::FilesystemError> {
        self.inner.list_dir_bounded(path, max_entries).await
    }

    async fn ensure_index(
        &self,
        prefix: &VirtualPath,
        spec: &ironclaw_filesystem::IndexSpec,
    ) -> Result<(), ironclaw_filesystem::FilesystemError> {
        self.inner.ensure_index(prefix, spec).await
    }

    async fn stat(
        &self,
        path: &VirtualPath,
    ) -> Result<ironclaw_filesystem::FileStat, ironclaw_filesystem::FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), ironclaw_filesystem::FilesystemError> {
        self.inner.delete(path).await
    }

    async fn begin(
        &self,
        path: &VirtualPath,
    ) -> Result<Box<dyn ironclaw_filesystem::StorageTxn>, ironclaw_filesystem::FilesystemError>
    {
        self.inner.begin(path).await
    }

    async fn reserve_sequence(
        &self,
        path: &VirtualPath,
    ) -> Result<ironclaw_filesystem::SeqNo, ironclaw_filesystem::FilesystemError> {
        self.inner.reserve_sequence(path).await
    }
}
