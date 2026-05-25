use std::collections::BTreeMap;

use ironclaw_extensions::{ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_host_api::{
    CapabilityId, NetworkTargetPattern, RuntimeCredentialInjection, RuntimeCredentialSource,
    RuntimeCredentialTarget, RuntimeKind, SecretHandle,
};
use ironclaw_network::{network_target_for_url, target_matches_pattern};
use ironclaw_wasm::{WasmHostError, WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest};

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
    pub(crate) handle: SecretHandle,
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

#[derive(Debug, Clone)]
pub(crate) struct SharedHostWasmRuntimeCredentials {
    registry: SharedExtensionRegistry,
}

impl SharedHostWasmRuntimeCredentials {
    pub(crate) fn new(registry: SharedExtensionRegistry) -> Self {
        Self { registry }
    }
}

impl WasmRuntimeCredentialProvider for SharedHostWasmRuntimeCredentials {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        wasm_runtime_credentials_from_registry(&self.registry.snapshot())
            .credential_injections(request)
    }
}

impl WasmRuntimeCredentialProvider for HostWasmRuntimeCredentials {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        let Ok(target) = network_target_for_url(&request.url) else {
            return Ok(Vec::new());
        };
        Ok(self
            .by_capability
            .get(&request.capability_id)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .map(|idx| &self.credentials[*idx])
            .filter(|credential| target_matches_pattern(&target, &credential.audience))
            .map(|credential| RuntimeCredentialInjection {
                handle: credential.handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: request.capability_id.clone(),
                },
                target: credential.target.clone(),
                required: credential.required,
            })
            .collect())
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
                        handle: credential.handle.clone(),
                        audience: credential.audience.clone(),
                        target: credential.target.clone(),
                        required: credential.required,
                    })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ManifestSource};
    use ironclaw_host_api::{
        HostPortCatalog, InvocationId, NetworkMethod, NetworkScheme, NetworkTargetPattern,
        ProjectId, ResourceScope, RuntimeCredentialTarget, TenantId, UserId, VirtualPath,
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
            handle: SecretHandle::new("api_token").unwrap(),
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
            handle: SecretHandle::new("api_token").unwrap(),
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
    fn manifest_wasm_credentials_ignore_malformed_and_non_http_urls() {
        let credentials = HostWasmRuntimeCredentials::new(vec![HostWasmRuntimeCredential {
            capability_id: CapabilityId::new("test-wasm.fetch").unwrap(),
            handle: SecretHandle::new("api_token").unwrap(),
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
