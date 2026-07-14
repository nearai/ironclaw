use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionHealthMessage, ExtensionHealthSnapshot, ExtensionHealthStatus, ExtensionInstallation,
    ExtensionInstallationId, ExtensionInstallationPersistedParts, ExtensionManifestRecord,
    ExtensionManifestRef, ExtensionRemovalChannelId, ExtensionRemovalCleanupAdapterId,
    ExtensionRemovalCleanupRequirement, InstallationOwner, MANIFEST_SCHEMA_VERSION,
};
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemOperation,
    InMemoryBackend, RecordVersion, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{HostPortCatalog, SecretHandle};

use super::*;

#[test]
fn persisted_removal_cleanup_metadata_rejects_invalid_identifiers() {
    for (adapter_id, channel) in [("", "slack"), ("slack.connection", "bad channel")] {
        let wire = serde_json::json!({
            "manifests": [{
                "raw_toml": "",
                "source": "host_bundled",
                "removal_cleanup_requirements": [{
                    "adapter_id": adapter_id,
                    "binding": { "kind": "channel_connection", "channel": channel }
                }]
            }],
            "installations": []
        });
        assert!(
            serde_json::from_value::<WireState>(wire).is_err(),
            "invalid durable cleanup metadata must fail closed"
        );
    }
}

#[tokio::test]
async fn load_at_treats_not_found_as_empty_state() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();
    let state_path =
        VirtualPath::new("/tenants/acme/system/extensions/.installations/missing-state.json")
            .expect("valid state path");

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
        .await
        .expect("missing state file loads as empty");

    assert!(
        store
            .list_installations()
            .await
            .expect("list installations")
            .is_empty()
    );
    assert!(
        backend.get(&state_path).await.unwrap().is_none(),
        "loading an absent snapshot must not create one"
    );
}

#[tokio::test]
async fn load_at_sanitizes_filesystem_read_errors() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(ReadFailureFilesystem::new());
    let state_path = VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json")
        .expect("valid state path");

    let error = match FilesystemExtensionInstallationStore::load_at(filesystem, state_path).await {
        Ok(_) => panic!("backend read failure should surface as invalid installation"),
        Err(error) => error,
    };

    let rendered = error.to_string();
    assert!(rendered.contains("failed to load extension installation state"));
    assert!(!rendered.contains("/tenants/acme"));
    assert!(!rendered.contains("raw backend detail"));
}

#[test]
fn public_manifest_ingestion_stays_strict_about_top_level_capabilities() {
    let contracts = product_extension_host_api_contract_registry().expect("host api contracts");
    for source in [
        ManifestSource::InstalledLocal,
        ManifestSource::RegistryInstalled,
        ManifestSource::HostBundled,
    ] {
        let error = ExtensionManifestRecord::from_toml(
            legacy_manifest_toml("legacy-tools"),
            source,
            &HostPortCatalog::empty(),
            Some(ManifestHash::new("sha256:legacy-tools").unwrap()),
            &contracts,
        )
        .expect_err("public ingestion must reject the retired persisted shape");
        assert!(
            error
                .to_string()
                .contains("top-level [[capabilities]] is not supported"),
            "{source:?}: {error}"
        );
    }
}

#[tokio::test]
async fn load_at_converts_persisted_host_bundled_legacy_manifest_before_strict_parsing() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let cleanup = test_cleanup_requirement();
    let legacy_hash = ManifestHash::new("sha256:legacy-tools").unwrap();
    let legacy_raw = legacy_manifest_toml("legacy-tools");
    let original_capabilities =
        toml::from_str::<toml::Value>(&legacy_raw).unwrap()["capabilities"].clone();
    let state = WireState {
        manifests: vec![WireManifestRecord {
            raw_toml: legacy_raw,
            source: WireManifestSource::HostBundled,
            manifest_hash: Some(legacy_hash.clone()),
            removal_cleanup_requirements: vec![cleanup.clone()],
        }],
        installations: vec![named_installation(
            "legacy-tools-old",
            "legacy-tools",
            ExtensionActivationState::Enabled,
            InstallationOwner::Tenant,
            Some("sha256:legacy-tools"),
            Vec::new(),
            "2026-01-02T00:00:00Z",
            "2026-01-03T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        )],
    };
    let seeded = seed_wire_state(&backend, &state_path, &state).await;
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
        .await
        .expect("persisted host-bundled legacy manifest is converted");

    let manifest = store
        .get_manifest(&ExtensionId::new("legacy-tools").unwrap())
        .await
        .unwrap()
        .expect("converted manifest is accepted by the strict parser");
    assert!(!manifest.raw_toml().contains("[[capabilities]]"));
    assert!(
        manifest
            .raw_toml()
            .contains("[[capability_provider.tools.capabilities]]")
    );
    assert!(manifest.raw_toml().contains("legacy-tools.echo"));
    assert!(manifest.raw_toml().contains("legacy-tools.inspect"));
    let converted = toml::from_str::<toml::Value>(manifest.raw_toml()).unwrap();
    assert_eq!(
        converted["capability_provider"]["tools"]["capabilities"],
        original_capabilities
    );
    assert_eq!(manifest.manifest_hash(), Some(&legacy_hash));
    assert_eq!(manifest.removal_cleanup_requirements(), &[cleanup]);
    assert_eq!(manifest.manifest().source, ManifestSource::HostBundled);

    let persisted = backend.get(&state_path).await.unwrap().unwrap();
    assert_ne!(persisted.version, seeded.version);
    let persisted_state: WireState = serde_json::from_slice(&persisted.entry.body).unwrap();
    assert_eq!(
        persisted_state.manifests[0].manifest_hash,
        Some(legacy_hash)
    );
    assert!(matches!(
        persisted_state.manifests[0].source,
        WireManifestSource::HostBundled
    ));
}

#[tokio::test]
async fn load_at_does_not_advance_version_for_current_or_second_load() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = WireState {
        manifests: vec![WireManifestRecord::from(test_manifest_record())],
        installations: vec![stored_installation(
            "canonical-tools",
            ExtensionActivationState::Enabled,
            InstallationOwner::Tenant,
            None,
        )],
    };
    let seeded = seed_wire_state(&backend, &state_path, &state).await;

    for expected_version in [seeded.version, seeded.version] {
        let filesystem: Arc<dyn RootFilesystem> = backend.clone();
        FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
            .await
            .expect("current snapshot loads without a write");
        let current = backend.get(&state_path).await.unwrap().unwrap();
        assert_eq!(current.version, expected_version);
        assert_eq!(current.entry.body, seeded.entry.body);
    }
}

#[tokio::test]
async fn load_at_rejects_non_exact_legacy_manifest_shapes_without_changes() {
    let cases = [
        (
            "installed-local",
            WireManifestSource::InstalledLocal,
            legacy_manifest_toml("legacy-tools"),
        ),
        (
            "registry-installed",
            WireManifestSource::RegistryInstalled,
            legacy_manifest_toml("legacy-tools"),
        ),
        (
            "installed-local-retired-slack",
            WireManifestSource::InstalledLocal,
            legacy_manifest_toml("slack_bot"),
        ),
        (
            "registry-installed-retired-slack",
            WireManifestSource::RegistryInstalled,
            legacy_manifest_toml("slack_bot"),
        ),
        (
            "strict-invalid-host-bundled-retired-slack",
            WireManifestSource::HostBundled,
            legacy_manifest_toml("slack_bot").replace("slack_bot.echo", "wrong-provider.echo"),
        ),
        (
            "wrong-schema",
            WireManifestSource::HostBundled,
            legacy_manifest_toml("legacy-tools").replace(
                &format!("schema_version = '{MANIFEST_SCHEMA_VERSION}'"),
                "schema_version = '1'",
            ),
        ),
        (
            "mixed-current-and-legacy",
            WireManifestSource::HostBundled,
            format!(
                "{}\n{}",
                legacy_manifest_toml("legacy-tools"),
                capability_provider_section("legacy-tools")
            ),
        ),
        (
            "invalid-capabilities",
            WireManifestSource::HostBundled,
            legacy_manifest_toml("legacy-tools")
                .replace("[[capabilities]]", "capabilities = 'invalid'"),
        ),
    ];

    for (case, source, raw_toml) in cases {
        let backend = Arc::new(InMemoryBackend::new());
        let state_path = test_state_path();
        let state = WireState {
            manifests: vec![WireManifestRecord {
                raw_toml,
                source,
                manifest_hash: None,
                removal_cleanup_requirements: Vec::new(),
            }],
            installations: Vec::new(),
        };
        let seeded = seed_wire_state(&backend, &state_path, &state).await;
        let filesystem: Arc<dyn RootFilesystem> = backend.clone();

        if FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
            .await
            .is_ok()
        {
            panic!("{case}: non-exact legacy shape must fail closed");
        }

        let after = backend.get(&state_path).await.unwrap().unwrap();
        assert_eq!(after.version, seeded.version, "{case}");
        assert_eq!(after.entry.body, seeded.entry.body, "{case}");
    }
}

#[tokio::test]
async fn load_at_slack_fold_preserves_canonical_fields_and_uses_enabled_wins() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let cleanup = test_cleanup_requirement();
    let state = WireState {
        manifests: vec![
            WireManifestRecord {
                raw_toml: structural_manifest_toml("slack_bot"),
                source: WireManifestSource::HostBundled,
                manifest_hash: Some(ManifestHash::new("sha256:retired-slack").unwrap()),
                removal_cleanup_requirements: Vec::new(),
            },
            WireManifestRecord {
                raw_toml: current_manifest_toml("slack"),
                source: WireManifestSource::RegistryInstalled,
                manifest_hash: Some(ManifestHash::new("sha256:unified-slack").unwrap()),
                removal_cleanup_requirements: vec![cleanup.clone()],
            },
        ],
        installations: vec![
            named_installation(
                "unified-alice",
                "slack",
                ExtensionActivationState::Disabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("alice").unwrap()),
                Some("sha256:unified-slack"),
                vec![test_binding("bot", "shared-secret")],
                "2026-01-02T00:00:00Z",
                "2026-01-06T00:00:00Z",
                ExtensionHealthStatus::Healthy,
            ),
            named_installation(
                "retired-bob",
                "slack_bot",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("bob").unwrap()),
                Some("sha256:retired-slack"),
                vec![
                    test_binding("bot", "shared-secret"),
                    test_binding("signing", "signing-secret"),
                ],
                "2026-01-04T00:00:00Z",
                "2026-01-05T00:00:00Z",
                ExtensionHealthStatus::Degraded,
            ),
        ],
    };
    seed_wire_state(&backend, &state_path, &state).await;
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
        .await
        .expect("retired and unified Slack rows fold");
    let installation = store.list_installations().await.unwrap().pop().unwrap();
    assert_eq!(installation.installation_id().as_str(), "slack");
    assert_eq!(installation.extension_id().as_str(), "slack");
    assert_eq!(
        installation.activation_state(),
        ExtensionActivationState::Enabled
    );
    assert_eq!(
        installation.owner().members(),
        Some(&BTreeSet::from([
            ironclaw_host_api::UserId::new("alice").unwrap(),
            ironclaw_host_api::UserId::new("bob").unwrap(),
        ]))
    );
    assert_eq!(installation.credential_bindings().len(), 2);
    assert_eq!(
        installation.health().status(),
        ExtensionHealthStatus::Degraded
    );
    assert_eq!(
        installation.health().checked_at().to_rfc3339(),
        "2026-01-04T00:00:00+00:00"
    );
    assert_eq!(
        installation.updated_at().to_rfc3339(),
        "2026-01-06T00:00:00+00:00"
    );
    assert_eq!(
        installation.manifest_ref().manifest_hash(),
        Some(&ManifestHash::new("sha256:unified-slack").unwrap())
    );

    let manifest = store
        .get_manifest(&ExtensionId::new("slack").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        manifest.manifest().source,
        ManifestSource::RegistryInstalled
    );
    assert_eq!(
        manifest.manifest_hash(),
        Some(&ManifestHash::new("sha256:unified-slack").unwrap())
    );
    assert_eq!(manifest.removal_cleanup_requirements(), &[cleanup]);
    assert!(
        store
            .get_manifest(&ExtensionId::new("slack_bot").unwrap())
            .await
            .unwrap()
            .is_none()
    );

    let after_first = backend.get(&state_path).await.unwrap().unwrap();
    let second_filesystem: Arc<dyn RootFilesystem> = backend.clone();
    FilesystemExtensionInstallationStore::load_at(second_filesystem, state_path.clone())
        .await
        .expect("Slack fold is rerunnable");
    let after_second = backend.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after_second.version, after_first.version);
    assert_eq!(after_second.entry.body, after_first.entry.body);
}

#[tokio::test]
async fn load_at_slack_fold_uses_tenant_dominance_and_canonical_mixed_policy_without_enabled() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = slack_wire_state(vec![
        named_installation(
            "unified-tenant",
            "slack",
            ExtensionActivationState::Installed,
            InstallationOwner::Tenant,
            Some("sha256:unified-slack"),
            Vec::new(),
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        ),
        named_installation(
            "retired-member",
            "slack_bot",
            ExtensionActivationState::Disabled,
            InstallationOwner::user(ironclaw_host_api::UserId::new("alice").unwrap()),
            Some("sha256:retired-slack"),
            Vec::new(),
            "2026-01-02T00:00:00Z",
            "2026-01-02T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        ),
    ]);
    seed_wire_state(&backend, &state_path, &state).await;
    let filesystem: Arc<dyn RootFilesystem> = backend;

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .expect("Slack rows fold through the canonical reducer");
    let installation = store.list_installations().await.unwrap().pop().unwrap();
    assert_eq!(installation.owner(), &InstallationOwner::Tenant);
    assert_eq!(
        installation.activation_state(),
        ExtensionActivationState::Disabled
    );
}

#[tokio::test]
async fn load_at_slack_fold_propagates_credential_conflict_without_changes() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = slack_wire_state(vec![
        named_installation(
            "unified",
            "slack",
            ExtensionActivationState::Installed,
            InstallationOwner::Tenant,
            Some("sha256:unified-slack"),
            vec![test_binding("bot", "unified-secret")],
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        ),
        named_installation(
            "retired",
            "slack_bot",
            ExtensionActivationState::Installed,
            InstallationOwner::Tenant,
            Some("sha256:retired-slack"),
            vec![test_binding("bot", "retired-secret")],
            "2026-01-02T00:00:00Z",
            "2026-01-02T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        ),
    ]);
    let seeded = seed_wire_state(&backend, &state_path, &state).await;
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();

    let error =
        match FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone()).await {
            Ok(_) => panic!("conflicting Slack credential handles must fail closed"),
            Err(error) => error,
        };
    assert!(matches!(
        error,
        ExtensionInstallationError::ConflictingCredentialBinding { .. }
    ));
    let after = backend.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after.version, seeded.version);
    assert_eq!(after.entry.body, seeded.entry.body);
}

#[cfg(feature = "slack-v2-host-beta")]
#[tokio::test]
async fn load_at_seeds_unified_slack_manifest_for_multiple_retired_rows() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = WireState {
        manifests: vec![WireManifestRecord {
            raw_toml: legacy_manifest_toml("slack_bot"),
            source: WireManifestSource::HostBundled,
            manifest_hash: Some(ManifestHash::new("sha256:retired-slack").unwrap()),
            removal_cleanup_requirements: Vec::new(),
        }],
        installations: vec![
            named_installation(
                "retired-alice",
                "slack_bot",
                ExtensionActivationState::Installed,
                InstallationOwner::user(ironclaw_host_api::UserId::new("alice").unwrap()),
                Some("sha256:retired-slack"),
                Vec::new(),
                "2026-01-01T00:00:00Z",
                "2026-01-01T00:00:00Z",
                ExtensionHealthStatus::Healthy,
            ),
            named_installation(
                "retired-bob",
                "slack_bot",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("bob").unwrap()),
                Some("sha256:retired-slack"),
                Vec::new(),
                "2026-01-02T00:00:00Z",
                "2026-01-02T00:00:00Z",
                ExtensionHealthStatus::Healthy,
            ),
        ],
    };
    seed_wire_state(&backend, &state_path, &state).await;
    let filesystem: Arc<dyn RootFilesystem> = backend;

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .expect("feature-enabled build seeds the bundled unified manifest");
    assert!(
        store
            .get_manifest(&ExtensionId::new("slack").unwrap())
            .await
            .unwrap()
            .is_some()
    );
    let manifest = store
        .get_manifest(&ExtensionId::new("slack").unwrap())
        .await
        .unwrap()
        .unwrap();
    let expected_hash =
        ManifestHash::new(crate::extension_host::available_extensions::slack_manifest_digest())
            .unwrap();
    assert_eq!(manifest.manifest_hash(), Some(&expected_hash));
    assert_eq!(
        manifest.removal_cleanup_requirements(),
        &[ExtensionRemovalCleanupRequirement::channel_connection(
            ExtensionRemovalCleanupAdapterId::new(
                crate::extension_host::extension_removal_cleanup::SLACK_PERSONAL_CONNECTION_CLEANUP_ADAPTER_ID,
            )
            .unwrap(),
            ExtensionRemovalChannelId::new(
                crate::extension_host::extension_removal_cleanup::SLACK_EXTENSION_REMOVAL_CHANNEL_ID,
            )
            .unwrap(),
        )]
    );
    let installation = store.list_installations().await.unwrap().pop().unwrap();
    assert_eq!(installation.installation_id().as_str(), "slack");
    assert_eq!(
        installation.manifest_ref().manifest_hash(),
        Some(&expected_hash)
    );
    assert_eq!(
        installation.owner().members(),
        Some(&BTreeSet::from([
            ironclaw_host_api::UserId::new("alice").unwrap(),
            ironclaw_host_api::UserId::new("bob").unwrap(),
        ]))
    );
}

#[cfg(not(feature = "slack-v2-host-beta"))]
#[tokio::test]
async fn load_at_without_slack_feature_never_deletes_retired_state() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = WireState {
        manifests: vec![WireManifestRecord {
            raw_toml: legacy_manifest_toml("slack_bot"),
            source: WireManifestSource::HostBundled,
            manifest_hash: Some(ManifestHash::new("sha256:retired-slack").unwrap()),
            removal_cleanup_requirements: Vec::new(),
        }],
        installations: vec![named_installation(
            "retired",
            "slack_bot",
            ExtensionActivationState::Enabled,
            InstallationOwner::Tenant,
            Some("sha256:retired-slack"),
            Vec::new(),
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        )],
    };
    let seeded = seed_wire_state(&backend, &state_path, &state).await;
    let filesystem: Arc<dyn RootFilesystem> = backend.clone();

    let error =
        match FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone()).await {
            Ok(_) => panic!("feature-disabled build cannot safely fold the retired identity"),
            Err(error) => error,
        };
    assert!(matches!(
        error,
        ExtensionInstallationError::InvalidInstallation { ref reason }
            if reason.contains("unified Slack manifest is unavailable")
    ));

    let after = backend.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after.version, seeded.version);
    assert_eq!(after.entry.body, seeded.entry.body);
}

#[tokio::test]
async fn load_at_retries_cas_conflict_and_returns_the_winning_snapshot() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = WireState {
        manifests: vec![WireManifestRecord {
            raw_toml: legacy_manifest_toml("legacy-tools"),
            source: WireManifestSource::HostBundled,
            manifest_hash: None,
            removal_cleanup_requirements: Vec::new(),
        }],
        installations: Vec::new(),
    };
    seed_wire_state(&backend, &state_path, &state).await;
    let racing = Arc::new(ConflictOnceFilesystem::new(backend.clone()));
    let filesystem: Arc<dyn RootFilesystem> = racing.clone();

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
        .await
        .expect("CAS contention is retried");

    assert!(racing.injected.load(Ordering::SeqCst));
    assert!(
        store
            .get_manifest(&ExtensionId::new("concurrent-tools").unwrap())
            .await
            .unwrap()
            .is_some(),
        "returned in-memory state must be loaded from the winning reread"
    );
    let persisted: WireState =
        serde_json::from_slice(&backend.get(&state_path).await.unwrap().unwrap().entry.body)
            .unwrap();
    assert!(
        persisted
            .manifests
            .iter()
            .any(|record| manifest_id(&record.raw_toml).as_deref() == Some("concurrent-tools"))
    );
}

#[tokio::test]
async fn load_at_non_cas_backend_normalizes_in_memory_without_putting() {
    let backend = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let state = WireState {
        manifests: vec![WireManifestRecord {
            raw_toml: legacy_manifest_toml("legacy-tools"),
            source: WireManifestSource::HostBundled,
            manifest_hash: None,
            removal_cleanup_requirements: Vec::new(),
        }],
        installations: Vec::new(),
    };
    let seeded = seed_wire_state(&backend, &state_path, &state).await;
    let non_cas = Arc::new(NonCasFilesystem::new(backend.clone()));
    let filesystem: Arc<dyn RootFilesystem> = non_cas.clone();

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path.clone())
        .await
        .expect("non-CAS backend uses the normalized snapshot in memory");

    assert_eq!(non_cas.puts.load(Ordering::SeqCst), 0);
    assert!(
        store
            .get_manifest(&ExtensionId::new("legacy-tools").unwrap())
            .await
            .unwrap()
            .is_some()
    );
    let after = backend.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after.version, seeded.version);
    assert_eq!(after.entry.body, seeded.entry.body);
}

#[tokio::test]
async fn load_at_persists_state_to_custom_path() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path = VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json")
        .expect("valid state path");
    let store =
        FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path.clone())
            .await
            .expect("store loads");
    let installation_id =
        ExtensionInstallationId::new("gmail".to_string()).expect("valid installation id");
    let extension_id = ExtensionId::new("gmail").expect("valid extension id");
    let manifest_ref = ExtensionManifestRef::new(extension_id.clone(), None);
    let contracts = product_extension_host_api_contract_registry().expect("host api contracts");
    let manifest = ExtensionManifestRecord::from_toml(
        format!(
            r#"
schema_version = "{schema}"
id = "gmail"
name = "Gmail"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/gmail.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "gmail.echo"
description = "Echoes input"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/gmail/echo.input.v1.json"
output_schema_ref = "schemas/gmail/echo.output.v1.json"
prompt_doc_ref = "prompts/gmail/echo.md"
"#,
            schema = MANIFEST_SCHEMA_VERSION,
        ),
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        None,
        &contracts,
    )
    .expect("valid manifest");
    store
        .upsert_manifest_and_installation(
            manifest,
            ExtensionInstallation::new(
                installation_id.clone(),
                extension_id,
                ExtensionActivationState::Installed,
                manifest_ref,
                Vec::new(),
                Utc::now(),
                InstallationOwner::Tenant,
            )
            .expect("valid installation"),
        )
        .await
        .expect("installation saved");

    assert!(
        filesystem
            .read_file(&state_path)
            .await
            .expect("state file exists")
            .starts_with(b"{")
    );

    let reloaded = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .expect("store reloads");
    assert!(
        reloaded
            .get_installation(&installation_id)
            .await
            .expect("installation read")
            .is_some()
    );
}

#[tokio::test]
async fn load_at_canonicalizes_duplicate_rows_and_preserves_complete_snapshot() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let original = seed_state(
        &filesystem,
        &state_path,
        vec![
            stored_installation(
                "legacy-alice",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("alice").unwrap()),
                None,
            ),
            stored_installation(
                "legacy-bob",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("bob").unwrap()),
                None,
            ),
        ],
    )
    .await;
    let store =
        FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path.clone())
            .await
            .expect("duplicate rows load");
    let installation = store.list_installations().await.unwrap().pop().unwrap();
    assert_eq!(installation.installation_id().as_str(), "canonical-tools");
    assert_eq!(
        installation.owner().members(),
        Some(&BTreeSet::from([
            ironclaw_host_api::UserId::new("alice").unwrap(),
            ironclaw_host_api::UserId::new("bob").unwrap(),
        ]))
    );

    let rewritten = filesystem.read_file(&state_path).await.unwrap();
    assert_ne!(rewritten, original);
    let state: WireState = serde_json::from_slice(&rewritten).unwrap();
    assert_eq!(state.manifests.len(), 1);
    assert_eq!(state.installations.len(), 1);
}

#[tokio::test]
async fn load_at_applies_tenant_dominance_and_mixed_activation_policy() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    seed_state(
        &filesystem,
        &state_path,
        vec![
            stored_installation(
                "legacy-member",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("alice").unwrap()),
                None,
            ),
            stored_installation(
                "legacy-tenant",
                ExtensionActivationState::Installed,
                InstallationOwner::Tenant,
                None,
            ),
        ],
    )
    .await;

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .expect("tenant/member rows load");
    let installation = store.list_installations().await.unwrap().pop().unwrap();
    assert_eq!(installation.owner(), &InstallationOwner::Tenant);
    assert_eq!(
        installation.activation_state(),
        ExtensionActivationState::Disabled
    );
}

#[tokio::test]
async fn load_at_rekeys_single_row_and_is_idempotent() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    seed_state(
        &filesystem,
        &state_path,
        vec![stored_installation(
            "legacy-single",
            ExtensionActivationState::Enabled,
            InstallationOwner::Tenant,
            None,
        )],
    )
    .await;

    FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path.clone())
        .await
        .expect("first load");
    let once = filesystem.read_file(&state_path).await.unwrap();
    assert!(String::from_utf8_lossy(&once).contains("canonical-tools"));

    FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path.clone())
        .await
        .expect("second load");
    let twice = filesystem.read_file(&state_path).await.unwrap();
    assert_eq!(once, twice);
}

#[tokio::test]
async fn load_at_rejects_manifest_conflicts_without_rewriting_state() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let original = seed_state(
        &filesystem,
        &state_path,
        vec![
            stored_installation(
                "legacy-manifest-a",
                ExtensionActivationState::Installed,
                InstallationOwner::Tenant,
                Some("manifest-a"),
            ),
            stored_installation(
                "legacy-manifest-b",
                ExtensionActivationState::Installed,
                InstallationOwner::Tenant,
                Some("manifest-b"),
            ),
        ],
    )
    .await;
    let original_version = filesystem.get(&state_path).await.unwrap().unwrap().version;

    let error = match FilesystemExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        state_path.clone(),
    )
    .await
    {
        Ok(_) => panic!("manifest conflict fails closed"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ExtensionInstallationError::ConflictingManifestReference { .. }
    ));
    let after = filesystem.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after.version, original_version);
    assert_eq!(after.entry.body, original);
}

#[tokio::test]
async fn load_at_rejects_credential_conflicts_without_rewriting_state() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path = test_state_path();
    let original = seed_state(
        &filesystem,
        &state_path,
        vec![
            stored_installation_with_credential(
                "legacy-credential-a",
                ExtensionActivationState::Installed,
                InstallationOwner::Tenant,
                "secret-a",
            ),
            stored_installation_with_credential(
                "legacy-credential-b",
                ExtensionActivationState::Installed,
                InstallationOwner::Tenant,
                "secret-b",
            ),
        ],
    )
    .await;
    let original_version = filesystem.get(&state_path).await.unwrap().unwrap().version;

    let error = match FilesystemExtensionInstallationStore::load_at(
        Arc::clone(&filesystem),
        state_path.clone(),
    )
    .await
    {
        Ok(_) => panic!("credential conflict fails closed"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ExtensionInstallationError::ConflictingCredentialBinding { .. }
    ));
    let after = filesystem.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after.version, original_version);
    assert_eq!(after.entry.body, original);
}

#[tokio::test]
async fn load_at_returns_rewrite_failure_without_exposing_normalized_store() {
    let backend = Arc::new(InMemoryBackend::new());
    let seed_filesystem: Arc<dyn RootFilesystem> = backend.clone();
    let state_path = test_state_path();
    let original = seed_state(
        &seed_filesystem,
        &state_path,
        vec![
            stored_installation(
                "legacy-rewrite-a",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("alice").unwrap()),
                None,
            ),
            stored_installation(
                "legacy-rewrite-b",
                ExtensionActivationState::Enabled,
                InstallationOwner::user(ironclaw_host_api::UserId::new("bob").unwrap()),
                None,
            ),
        ],
    )
    .await;
    let original_version = backend.get(&state_path).await.unwrap().unwrap().version;
    let failing_filesystem: Arc<dyn RootFilesystem> = Arc::new(WriteFailureFilesystem {
        inner: backend.clone(),
    });

    let error =
        match FilesystemExtensionInstallationStore::load_at(failing_filesystem, state_path.clone())
            .await
        {
            Ok(_) => panic!("canonical rewrite failure must fail closed"),
            Err(error) => error,
        };
    assert_eq!(
        error,
        ExtensionInstallationError::InvalidInstallation {
            reason: "failed to load extension installation state".to_string(),
        }
    );
    assert!(!error.to_string().contains("/tenants/acme"));
    assert!(
        !error
            .to_string()
            .contains("injected extension installation write failure")
    );
    let after = backend.get(&state_path).await.unwrap().unwrap();
    assert_eq!(after.version, original_version);
    assert_eq!(after.entry.body, original);
}

#[test]
fn canonicalization_rejects_conflicting_credential_mappings() {
    let extension_id = ExtensionId::new("canonical-tools").unwrap();
    let make_installation = |installation_id: &str, secret: &str| {
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id: ExtensionInstallationId::new(installation_id).unwrap(),
            extension_id: extension_id.clone(),
            activation_state: ExtensionActivationState::Installed,
            manifest_ref: ExtensionManifestRef::new(extension_id.clone(), None),
            credential_bindings: vec![ExtensionCredentialBinding::new(
                ExtensionCredentialHandle::new("api").unwrap(),
                SecretHandle::new(secret).unwrap(),
            )],
            health: ExtensionHealthSnapshot::new(ExtensionHealthStatus::Healthy, None, timestamp),
            updated_at: timestamp,
            owner: InstallationOwner::Tenant,
        })
        .unwrap()
    };

    let error = canonicalize_installation_rows(vec![
        make_installation("legacy-a", "secret-a"),
        make_installation("legacy-b", "secret-b"),
    ])
    .expect_err("credential mapping conflict fails closed");
    assert!(matches!(
        error,
        ExtensionInstallationError::ConflictingCredentialBinding { .. }
    ));
}

#[test]
fn canonicalization_preserves_newest_health_max_updated_at_and_agreeing_bindings() {
    let extension_id = ExtensionId::new("canonical-tools").unwrap();
    let binding = ExtensionCredentialBinding::new(
        ExtensionCredentialHandle::new("api").unwrap(),
        SecretHandle::new("secret").unwrap(),
    );
    let make_installation = |installation_id: &str, checked_at: &str, updated_at: &str, status| {
        let checked_at = chrono::DateTime::parse_from_rfc3339(checked_at)
            .unwrap()
            .with_timezone(&Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(updated_at)
            .unwrap()
            .with_timezone(&Utc);
        ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id: ExtensionInstallationId::new(installation_id).unwrap(),
            extension_id: extension_id.clone(),
            activation_state: ExtensionActivationState::Enabled,
            manifest_ref: ExtensionManifestRef::new(extension_id.clone(), None),
            credential_bindings: vec![binding.clone()],
            health: ExtensionHealthSnapshot::new(status, None, checked_at),
            updated_at,
            owner: InstallationOwner::Tenant,
        })
        .unwrap()
    };

    let canonical = canonicalize_installation_rows(vec![
        make_installation(
            "legacy-a",
            "2026-01-02T00:00:00Z",
            "2026-01-05T00:00:00Z",
            ExtensionHealthStatus::Healthy,
        ),
        make_installation(
            "legacy-b",
            "2026-01-03T00:00:00Z",
            "2026-01-04T00:00:00Z",
            ExtensionHealthStatus::Degraded,
        ),
    ])
    .unwrap();
    let installation = &canonical[0];
    assert_eq!(
        installation.health().checked_at().to_rfc3339(),
        "2026-01-03T00:00:00+00:00"
    );
    assert_eq!(
        installation.health().status(),
        ExtensionHealthStatus::Degraded
    );
    assert_eq!(
        installation.updated_at().to_rfc3339(),
        "2026-01-05T00:00:00+00:00"
    );
    assert_eq!(installation.credential_bindings(), &[binding]);
}

fn test_state_path() -> VirtualPath {
    VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json").unwrap()
}

async fn seed_state(
    filesystem: &Arc<dyn RootFilesystem>,
    state_path: &VirtualPath,
    installations: Vec<ExtensionInstallation>,
) -> Vec<u8> {
    let state = WireState {
        manifests: vec![WireManifestRecord::from(test_manifest_record())],
        installations,
    };
    let bytes = serde_json::to_vec_pretty(&state).unwrap();
    filesystem.write_file(state_path, &bytes).await.unwrap();
    bytes
}

fn test_manifest_record() -> ExtensionManifestRecord {
    let manifest = format!(
        "schema_version = \"{}\"\nid = \"canonical-tools\"\nname = \"Canonical Tools\"\nversion = \"0.1.0\"\ndescription = \"test\"\ntrust = \"third_party\"\n\n[runtime]\nkind = \"wasm\"\nmodule = \"wasm/canonical-tools.wasm\"\n\n[[host_api]]\nid = \"ironclaw.capability_provider/v1\"\nsection = \"capability_provider.tools\"\n\n[capability_provider.tools]\n\n[[capability_provider.tools.capabilities]]\nid = \"canonical-tools.echo\"\ndescription = \"Echo\"\ndefault_permission = \"allow\"\nvisibility = \"model\"\ninput_schema_ref = \"schemas/echo.input.json\"\noutput_schema_ref = \"schemas/echo.output.json\"\n",
        MANIFEST_SCHEMA_VERSION
    );
    let contracts = product_extension_host_api_contract_registry().expect("host api contracts");
    ExtensionManifestRecord::from_toml(
        manifest,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        None,
        &contracts,
    )
    .unwrap()
}

fn stored_installation(
    installation_id: &str,
    activation_state: ExtensionActivationState,
    owner: InstallationOwner,
    manifest_hash: Option<&str>,
) -> ExtensionInstallation {
    stored_installation_with_bindings(
        installation_id,
        activation_state,
        owner,
        manifest_hash,
        Vec::new(),
    )
}

fn stored_installation_with_credential(
    installation_id: &str,
    activation_state: ExtensionActivationState,
    owner: InstallationOwner,
    secret: &str,
) -> ExtensionInstallation {
    stored_installation_with_bindings(
        installation_id,
        activation_state,
        owner,
        None,
        vec![ExtensionCredentialBinding::new(
            ExtensionCredentialHandle::new("api").unwrap(),
            SecretHandle::new(secret).unwrap(),
        )],
    )
}

fn stored_installation_with_bindings(
    installation_id: &str,
    activation_state: ExtensionActivationState,
    owner: InstallationOwner,
    manifest_hash: Option<&str>,
    credential_bindings: Vec<ExtensionCredentialBinding>,
) -> ExtensionInstallation {
    let extension_id = ExtensionId::new("canonical-tools").unwrap();
    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
        installation_id: ExtensionInstallationId::new(installation_id).unwrap(),
        extension_id: extension_id.clone(),
        activation_state,
        manifest_ref: ExtensionManifestRef::new(
            extension_id,
            manifest_hash.map(|value| ManifestHash::new(value).unwrap()),
        ),
        credential_bindings,
        health: ExtensionHealthSnapshot::new(
            ExtensionHealthStatus::Healthy,
            Some(ExtensionHealthMessage::new(installation_id)),
            timestamp,
        ),
        updated_at: timestamp,
        owner,
    })
    .unwrap()
}

fn legacy_manifest_toml(extension_id: &str) -> String {
    format!(
        r#"id = '{extension_id}' # deliberately first and single-quoted
schema_version = '{schema}'
name = 'Legacy Tools'
version = '0.1.0'
description = 'persisted legacy v2 test manifest'
trust = 'third_party'
# id = "slack" is only a decoy comment

[runtime]
kind = 'wasm'
module = 'wasm/{extension_id}.wasm'

[[capabilities]]
id = '{extension_id}.echo'
description = 'Echo'
default_permission = 'allow'
visibility = 'model'
input_schema_ref = 'schemas/echo.input.json'
output_schema_ref = 'schemas/echo.output.json'

[[capabilities]]
id = '{extension_id}.inspect'
description = 'Inspect'
default_permission = 'allow'
visibility = 'model'
input_schema_ref = 'schemas/inspect.input.json'
output_schema_ref = 'schemas/inspect.output.json'
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    )
}

fn capability_provider_section(extension_id: &str) -> String {
    format!(
        r#"[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{extension_id}.echo"
description = "Echo"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo.input.json"
output_schema_ref = "schemas/echo.output.json"
"#
    )
}

fn current_manifest_toml(extension_id: &str) -> String {
    format!(
        r#"schema_version = "{schema}"
id = "{extension_id}"
name = "Current Tools"
version = "0.1.0"
description = "current v2 test manifest"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{extension_id}.wasm"

{capability_provider}
"#,
        schema = MANIFEST_SCHEMA_VERSION,
        capability_provider = capability_provider_section(extension_id),
    )
}

fn structural_manifest_toml(extension_id: &str) -> String {
    current_manifest_toml(extension_id)
        .replace(
            &format!(
                "schema_version = \"{MANIFEST_SCHEMA_VERSION}\"\nid = \"{extension_id}\""
            ),
            &format!(
                "id = '{extension_id}' # id first and single-quoted\nschema_version = '{MANIFEST_SCHEMA_VERSION}'\n# id = \"slack\" decoy"
            ),
        )
}

fn test_cleanup_requirement() -> ExtensionRemovalCleanupRequirement {
    ExtensionRemovalCleanupRequirement::channel_connection(
        ExtensionRemovalCleanupAdapterId::new("slack.connection").unwrap(),
        ExtensionRemovalChannelId::new("slack").unwrap(),
    )
}

fn test_binding(handle: &str, secret: &str) -> ExtensionCredentialBinding {
    ExtensionCredentialBinding::new(
        ExtensionCredentialHandle::new(handle).unwrap(),
        SecretHandle::new(secret).unwrap(),
    )
}

#[allow(clippy::too_many_arguments)]
fn named_installation(
    installation_id: &str,
    extension_id: &str,
    activation_state: ExtensionActivationState,
    owner: InstallationOwner,
    manifest_hash: Option<&str>,
    credential_bindings: Vec<ExtensionCredentialBinding>,
    checked_at: &str,
    updated_at: &str,
    health_status: ExtensionHealthStatus,
) -> ExtensionInstallation {
    let extension_id = ExtensionId::new(extension_id).unwrap();
    let checked_at = chrono::DateTime::parse_from_rfc3339(checked_at)
        .unwrap()
        .with_timezone(&Utc);
    let updated_at = chrono::DateTime::parse_from_rfc3339(updated_at)
        .unwrap()
        .with_timezone(&Utc);
    ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
        installation_id: ExtensionInstallationId::new(installation_id).unwrap(),
        extension_id: extension_id.clone(),
        activation_state,
        manifest_ref: ExtensionManifestRef::new(
            extension_id,
            manifest_hash.map(|value| ManifestHash::new(value).unwrap()),
        ),
        credential_bindings,
        health: ExtensionHealthSnapshot::new(
            health_status,
            Some(ExtensionHealthMessage::new(installation_id)),
            checked_at,
        ),
        updated_at,
        owner,
    })
    .unwrap()
}

fn slack_wire_state(installations: Vec<ExtensionInstallation>) -> WireState {
    WireState {
        manifests: vec![
            WireManifestRecord {
                raw_toml: structural_manifest_toml("slack_bot"),
                source: WireManifestSource::HostBundled,
                manifest_hash: Some(ManifestHash::new("sha256:retired-slack").unwrap()),
                removal_cleanup_requirements: Vec::new(),
            },
            WireManifestRecord {
                raw_toml: current_manifest_toml("slack"),
                source: WireManifestSource::HostBundled,
                manifest_hash: Some(ManifestHash::new("sha256:unified-slack").unwrap()),
                removal_cleanup_requirements: Vec::new(),
            },
        ],
        installations,
    }
}

async fn seed_wire_state(
    backend: &Arc<InMemoryBackend>,
    state_path: &VirtualPath,
    state: &WireState,
) -> VersionedEntry {
    let bytes = serde_json::to_vec_pretty(state).unwrap();
    backend
        .put(state_path, Entry::bytes(bytes), CasExpectation::Absent)
        .await
        .unwrap();
    backend.get(state_path).await.unwrap().unwrap()
}

fn manifest_id(raw_toml: &str) -> Option<String> {
    toml::from_str::<toml::Value>(raw_toml)
        .ok()?
        .as_table()?
        .get("id")?
        .as_str()
        .map(ToOwned::to_owned)
}

struct ConflictOnceFilesystem {
    inner: Arc<InMemoryBackend>,
    injected: AtomicBool,
}

impl ConflictOnceFilesystem {
    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            injected: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl RootFilesystem for ConflictOnceFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if !self.injected.swap(true, Ordering::SeqCst) {
            let current = self.inner.get(path).await?.expect("seeded state");
            let mut concurrent_state: WireState =
                serde_json::from_slice(&current.entry.body).expect("valid seeded state");
            concurrent_state.manifests.push(WireManifestRecord {
                raw_toml: current_manifest_toml("concurrent-tools"),
                source: WireManifestSource::HostBundled,
                manifest_hash: None,
                removal_cleanup_requirements: Vec::new(),
            });
            self.inner
                .put(
                    path,
                    Entry::bytes(
                        serde_json::to_vec_pretty(&concurrent_state)
                            .expect("serialize concurrent state"),
                    ),
                    CasExpectation::Version(current.version),
                )
                .await?;
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

struct NonCasFilesystem {
    inner: Arc<InMemoryBackend>,
    puts: AtomicUsize,
}

impl NonCasFilesystem {
    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            puts: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl RootFilesystem for NonCasFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::bytes_only()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

struct WriteFailureFilesystem {
    inner: Arc<InMemoryBackend>,
}

#[async_trait]
impl RootFilesystem for WriteFailureFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
            reason: "injected extension installation write failure".to_string(),
        })
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

struct ReadFailureFilesystem {
    inner: InMemoryBackend,
}

impl ReadFailureFilesystem {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
        }
    }
}

#[async_trait]
impl RootFilesystem for ReadFailureFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReadFile,
            reason: "raw backend detail".to_string(),
        })
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}
