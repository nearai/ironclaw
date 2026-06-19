use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    sync::{Arc, LazyLock, Mutex, mpsc},
};

use ironclaw_extensions::{ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_host_api::{
    CapabilityId, CredentialStageError, ExtensionId, NetworkTargetPattern,
    RuntimeCredentialInjection, RuntimeCredentialRequirementSource, RuntimeCredentialSource,
    RuntimeCredentialTarget, RuntimeKind, SecretHandle,
};
use ironclaw_network::{network_target_for_url, target_matches_pattern};
use ironclaw_secrets::SecretStore;
use ironclaw_wasm::{WasmHostError, WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest};
use tokio::runtime::{Handle, RuntimeFlavor};

use crate::{
    RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver,
    obligations::RuntimeSecretInjectionStore,
};

/// Host-derived WASM credential provider built from validated extension manifests.
///
/// This provider emits only host-approved staged-obligation injection plans. It
/// never exposes raw secret material to WASM and never grants new authority; the
/// matching staged secret material must already exist in the host egress handoff
/// store for the scoped capability invocation.
#[derive(Debug, Clone, Default)]
pub(crate) struct HostWasmRuntimeCredentials {
    credentials: Vec<HostWasmRuntimeCredential>,
    by_capability: BTreeMap<CapabilityId, Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostWasmRuntimeCredential {
    pub(crate) capability_id: CapabilityId,
    pub(crate) requester_extension: ExtensionId,
    pub(crate) handle: SecretHandle,
    pub(crate) source: RuntimeCredentialRequirementSource,
    pub(crate) provider_scopes: Vec<String>,
    pub(crate) audience: NetworkTargetPattern,
    pub(crate) target: RuntimeCredentialTarget,
    pub(crate) required: bool,
}

impl HostWasmRuntimeCredentials {
    pub(crate) fn new(credentials: Vec<HostWasmRuntimeCredential>) -> Self {
        let mut by_capability = BTreeMap::new();
        for (idx, credential) in credentials.iter().enumerate() {
            by_capability
                .entry(credential.capability_id.clone())
                .or_insert_with(Vec::new)
                .push(idx);
        }
        Self {
            credentials,
            by_capability,
        }
    }

    #[cfg(test)]
    pub(crate) fn credentials(&self) -> &[HostWasmRuntimeCredential] {
        &self.credentials
    }
}

#[derive(Clone, Default)]
pub(crate) struct SharedHostWasmRuntimeCredentials {
    registry: SharedExtensionRegistry,
    cache: Arc<Mutex<SharedCredentialCache>>,
    restager: Option<RuntimeCredentialRestager>,
}

impl SharedHostWasmRuntimeCredentials {
    pub(crate) fn new(registry: SharedExtensionRegistry) -> Self {
        Self {
            registry,
            cache: Arc::new(Mutex::new(SharedCredentialCache::default())),
            restager: None,
        }
    }

    pub(crate) fn with_product_auth_restaging(
        mut self,
        secret_store: Arc<dyn SecretStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        account_resolver: Arc<dyn RuntimeCredentialAccountResolver>,
    ) -> Self {
        self.restager = Some(RuntimeCredentialRestager {
            secret_store,
            secret_injections,
            account_resolver,
        });
        self
    }
}

impl fmt::Debug for SharedHostWasmRuntimeCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedHostWasmRuntimeCredentials")
            .field("registry", &self.registry)
            .field("cache", &self.cache)
            .field("restager", &self.restager.as_ref().map(|_| "<configured>"))
            .finish()
    }
}

#[derive(Debug, Default)]
struct SharedCredentialCache {
    registry_version: Option<u64>,
    credentials: HostWasmRuntimeCredentials,
}

#[derive(Clone)]
struct RuntimeCredentialRestager {
    secret_store: Arc<dyn SecretStore>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    account_resolver: Arc<dyn RuntimeCredentialAccountResolver>,
}

impl fmt::Debug for RuntimeCredentialRestager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeCredentialRestager")
            .field("secret_store", &"[REDACTED]")
            .field("secret_injections", &self.secret_injections)
            .field("account_resolver", &"<resolver>")
            .finish()
    }
}

impl RuntimeCredentialRestager {
    fn stage_for_request(
        &self,
        request: WasmRuntimeCredentialRequest,
        credential: HostWasmRuntimeCredential,
    ) -> Result<(), CredentialStageError> {
        let restager = self.clone();
        block_on_credential_restage(async move {
            restager
                .stage_for_request_async(&request, &credential)
                .await
        })
    }

    async fn stage_for_request_async(
        &self,
        request: &WasmRuntimeCredentialRequest,
        credential: &HostWasmRuntimeCredential,
    ) -> Result<(), CredentialStageError> {
        let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } =
            &credential.source
        else {
            return Ok(());
        };
        let access_secret = self
            .account_resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &request.scope,
                provider,
                setup,
                provider_scopes: &credential.provider_scopes,
                requester_extension: &credential.requester_extension,
            })
            .await?;
        let lease = self
            .secret_store
            .lease_once(&access_secret.scope, &access_secret.handle)
            .await
            .map_err(crate::services::stage_secret_error)?;
        let material = self
            .secret_store
            .consume(&access_secret.scope, lease.id)
            .await
            .map_err(crate::services::stage_secret_error)?;
        self.secret_injections
            .insert(
                &request.scope,
                &request.capability_id,
                &credential.handle,
                material,
            )
            .map_err(|_| CredentialStageError::Backend)
    }
}

impl WasmRuntimeCredentialProvider for SharedHostWasmRuntimeCredentials {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        let registry_version = self.registry.version();
        let credentials = {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if cache.registry_version != Some(registry_version) {
                cache.credentials =
                    wasm_runtime_credentials_from_registry(&self.registry.snapshot());
                cache.registry_version = Some(registry_version);
            }
            cache.credentials.clone()
        };
        credentials.credential_injections_with_restager(request, self.restager.as_ref())
    }
}

impl WasmRuntimeCredentialProvider for HostWasmRuntimeCredentials {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        self.credential_injections_with_restager(request, None)
    }
}

impl HostWasmRuntimeCredentials {
    fn credential_injections_with_restager(
        &self,
        request: &WasmRuntimeCredentialRequest,
        restager: Option<&RuntimeCredentialRestager>,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        let Ok(target) = network_target_for_url(&request.url) else {
            return Ok(Vec::new());
        };
        let matching_credentials = self
            .by_capability
            .get(&request.capability_id)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .map(|idx| &self.credentials[*idx])
            .filter(|credential| target_matches_pattern(&target, &credential.audience));
        let mut injections = Vec::new();
        for credential in matching_credentials {
            if let Some(restager) = restager {
                restager
                    .stage_for_request(request.clone(), credential.clone())
                    .map_err(|_| {
                        WasmHostError::Unavailable("credential_unavailable".to_string())
                    })?;
            }
            injections.push(RuntimeCredentialInjection {
                handle: credential.handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: request.capability_id.clone(),
                },
                target: credential.target.clone(),
                required: credential.required,
            });
        }
        Ok(injections)
    }
}

pub(crate) fn wasm_runtime_credentials_from_registry(
    registry: &ExtensionRegistry,
) -> HostWasmRuntimeCredentials {
    HostWasmRuntimeCredentials::new(
        registry
            .capabilities()
            .filter(|descriptor| descriptor.runtime == RuntimeKind::Wasm)
            .flat_map(|descriptor| {
                descriptor
                    .runtime_credentials
                    .iter()
                    .map(move |credential| HostWasmRuntimeCredential {
                        capability_id: descriptor.id.clone(),
                        requester_extension: descriptor.provider.clone(),
                        handle: credential.handle.clone(),
                        source: credential.source.clone(),
                        provider_scopes: credential.provider_scopes.clone(),
                        audience: credential.audience.clone(),
                        target: credential.target.clone(),
                        required: credential.required,
                    })
            })
            .collect(),
    )
}

fn block_on_credential_restage<F, T>(future: F) -> Result<T, CredentialStageError>
where
    F: Future<Output = Result<T, CredentialStageError>> + Send + 'static,
    T: Send + 'static,
{
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Ok(_) => run_credential_restage_on_worker(future),
        Err(_) => run_credential_restage_future(future),
    }
}

fn run_credential_restage_on_worker<F, T>(future: F) -> Result<T, CredentialStageError>
where
    F: Future<Output = Result<T, CredentialStageError>> + Send + 'static,
    T: Send + 'static,
{
    let runtime = WASM_CREDENTIAL_RESTAGE_RUNTIME
        .as_ref()
        .map_err(|error| *error)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver.recv().map_err(|_| CredentialStageError::Backend)?
}

fn run_credential_restage_future<F, T>(future: F) -> Result<T, CredentialStageError>
where
    F: Future<Output = Result<T, CredentialStageError>>,
{
    let runtime = WASM_CREDENTIAL_RESTAGE_RUNTIME
        .as_ref()
        .map_err(|error| *error)?;
    runtime.block_on(future)
}

static WASM_CREDENTIAL_RESTAGE_RUNTIME: LazyLock<
    Result<tokio::runtime::Runtime, CredentialStageError>,
> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("wasm-credential-restage")
        .enable_all()
        .build()
        .map_err(|_| CredentialStageError::Backend)
});

#[cfg(test)]
mod tests {
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ManifestSource};
    use ironclaw_host_api::{
        ExtensionId, HostPortCatalog, InvocationId, NetworkMethod, NetworkScheme,
        NetworkTargetPattern, ProjectId, ResourceScope, RuntimeCredentialTarget, TenantId, UserId,
        VirtualPath,
    };

    use super::*;

    #[test]
    fn wasm_credentials_are_derived_from_manifest_descriptors() {
        let package = test_package(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "test-wasm"
name = "Test WASM"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/test.wasm"

[[capabilities]]
id = "test-wasm.fetch"
description = "fetch"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "host_internal"
input_schema_ref = "schemas/test/fetch.input.v1.json"
output_schema_ref = "schemas/test/fetch.output.v1.json"
runtime_credentials = [
  { handle = "api_token", audience = { scheme = "https", host_pattern = "api.example.test" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]
"#,
            "test-wasm",
        );
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).unwrap();

        let credentials = wasm_runtime_credentials_from_registry(&registry);

        assert_eq!(credentials.credentials().len(), 1);
        assert_eq!(
            credentials.credentials()[0].capability_id,
            CapabilityId::new("test-wasm.fetch").unwrap()
        );
        assert_eq!(
            credentials.credentials()[0].handle,
            SecretHandle::new("api_token").unwrap()
        );
        assert_eq!(
            credentials.credentials()[0].audience,
            NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: None,
            }
        );
        assert_eq!(
            credentials.credentials()[0].target,
            RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            }
        );
        assert!(credentials.credentials()[0].required);
    }

    #[test]
    fn manifest_wasm_credentials_match_default_url_ports() {
        let credentials = HostWasmRuntimeCredentials::new(vec![HostWasmRuntimeCredential {
            capability_id: CapabilityId::new("test-wasm.fetch").unwrap(),
            requester_extension: ExtensionId::new("test-wasm").unwrap(),
            handle: SecretHandle::new("api_token").unwrap(),
            source: RuntimeCredentialRequirementSource::SecretHandle,
            provider_scopes: Vec::new(),
            audience: NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: Some(443),
            },
            target: runtime_credential_header_target(),
            required: true,
        }]);

        let injections = credentials
            .credential_injections(&wasm_credential_request(
                "test-wasm.fetch",
                "https://api.example.test/repos",
            ))
            .unwrap();

        assert_eq!(injections.len(), 1);
        assert_eq!(
            injections[0].handle,
            SecretHandle::new("api_token").unwrap()
        );
    }

    #[test]
    fn manifest_wasm_credentials_are_scoped_to_capability_and_audience() {
        let credentials = HostWasmRuntimeCredentials::new(vec![HostWasmRuntimeCredential {
            capability_id: CapabilityId::new("test-wasm.fetch").unwrap(),
            requester_extension: ExtensionId::new("test-wasm").unwrap(),
            handle: SecretHandle::new("api_token").unwrap(),
            source: RuntimeCredentialRequirementSource::SecretHandle,
            provider_scopes: Vec::new(),
            audience: NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: None,
            },
            target: runtime_credential_header_target(),
            required: true,
        }]);

        assert!(
            credentials
                .credential_injections(&wasm_credential_request(
                    "test-wasm.other",
                    "https://api.example.test/repos",
                ))
                .unwrap()
                .is_empty()
        );
        assert!(
            credentials
                .credential_injections(&wasm_credential_request(
                    "test-wasm.fetch",
                    "https://uploads.example.test/repos",
                ))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn shared_credentials_derive_from_active_registry_updates() {
        let registry = SharedExtensionRegistry::default();
        let provider = SharedHostWasmRuntimeCredentials::new(registry.clone());

        assert!(
            provider
                .credential_injections(&wasm_credential_request(
                    "test-wasm.fetch",
                    "https://api.example.test/repos",
                ))
                .unwrap()
                .is_empty()
        );

        registry
            .insert(test_package(
                r#"
schema_version = "reborn.extension_manifest.v2"
id = "test-wasm"
name = "Test WASM"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/test.wasm"

[[capabilities]]
id = "test-wasm.fetch"
description = "fetch"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "host_internal"
input_schema_ref = "schemas/test/fetch.input.v1.json"
output_schema_ref = "schemas/test/fetch.output.v1.json"
runtime_credentials = [
  { handle = "api_token", audience = { scheme = "https", host_pattern = "api.example.test" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]
"#,
                "test-wasm",
            ))
            .expect("insert package");

        let dynamic_injections = provider
            .credential_injections(&wasm_credential_request(
                "test-wasm.fetch",
                "https://api.example.test/repos",
            ))
            .unwrap();
        let static_credentials = wasm_runtime_credentials_from_registry(&registry.snapshot());
        let static_injections = static_credentials
            .credential_injections(&wasm_credential_request(
                "test-wasm.fetch",
                "https://api.example.test/repos",
            ))
            .unwrap();

        assert_eq!(dynamic_injections, static_injections);
        assert_eq!(dynamic_injections.len(), 1);

        registry.remove(&ExtensionId::new("test-wasm").unwrap());
        assert!(
            provider
                .credential_injections(&wasm_credential_request(
                    "test-wasm.fetch",
                    "https://api.example.test/repos",
                ))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn manifest_wasm_credentials_ignore_malformed_and_non_http_urls() {
        let credentials = HostWasmRuntimeCredentials::new(vec![HostWasmRuntimeCredential {
            capability_id: CapabilityId::new("test-wasm.fetch").unwrap(),
            requester_extension: ExtensionId::new("test-wasm").unwrap(),
            handle: SecretHandle::new("api_token").unwrap(),
            source: RuntimeCredentialRequirementSource::SecretHandle,
            provider_scopes: Vec::new(),
            audience: NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: None,
            },
            target: runtime_credential_header_target(),
            required: true,
        }]);

        for url in ["not a url", "ftp://api.example.test/repos"] {
            assert!(
                credentials
                    .credential_injections(&wasm_credential_request("test-wasm.fetch", url))
                    .unwrap()
                    .is_empty(),
                "{url:?} should not receive staged credentials"
            );
        }
    }

    fn test_package(manifest: &str, extension_id: &str) -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            manifest,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("test manifest should parse");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{extension_id}")).unwrap(),
        )
        .expect("test package should build")
    }

    fn runtime_credential_header_target() -> RuntimeCredentialTarget {
        RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        }
    }

    fn wasm_credential_request(capability_id: &str, url: &str) -> WasmRuntimeCredentialRequest {
        WasmRuntimeCredentialRequest {
            scope: sample_scope(),
            capability_id: CapabilityId::new(capability_id).unwrap(),
            method: NetworkMethod::Get,
            url: url.to_string(),
            headers: Vec::new(),
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}
