use std::{collections::BTreeSet, sync::Arc};

use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionHealthMessage, ExtensionHealthSnapshot, ExtensionHealthStatus, ExtensionInstallation,
    ExtensionInstallationId, ExtensionInstallationPersistedParts, ExtensionManifestRecord,
    ExtensionManifestRef, InstallationOwner, MANIFEST_SCHEMA_VERSION,
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
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let state_path =
        VirtualPath::new("/tenants/acme/system/extensions/.installations/missing-state.json")
            .expect("valid state path");

    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .expect("missing state file loads as empty");

    assert!(
        store
            .list_installations()
            .await
            .expect("list installations")
            .is_empty()
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

[[capabilities]]
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
    assert_eq!(filesystem.read_file(&state_path).await.unwrap(), original);
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
    assert_eq!(filesystem.read_file(&state_path).await.unwrap(), original);
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
    assert_eq!(backend.read_file(&state_path).await.unwrap(), original);
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
        "schema_version = \"{}\"\nid = \"canonical-tools\"\nname = \"Canonical Tools\"\nversion = \"0.1.0\"\ndescription = \"test\"\ntrust = \"third_party\"\n\n[runtime]\nkind = \"wasm\"\nmodule = \"wasm/canonical-tools.wasm\"\n\n[[capabilities]]\nid = \"canonical-tools.echo\"\ndescription = \"Echo\"\ndefault_permission = \"allow\"\nvisibility = \"model\"\ninput_schema_ref = \"schemas/echo.input.json\"\noutput_schema_ref = \"schemas/echo.output.json\"\n",
        MANIFEST_SCHEMA_VERSION
    );
    ExtensionManifestRecord::from_toml(
        manifest,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        None,
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
