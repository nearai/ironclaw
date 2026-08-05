use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    capability::{EffectKind, OriginGateMatrix, OriginGatePolicy, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    ids::CapabilityId,
    resource::{ResourceEstimate, ResourceProfile, ResourceUsage},
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_skills::ScopedSkillManagementPort;
use serde::Deserialize;

use ironclaw_extension_host::ExtensionLifecycleManager;

use super::link_service::IronhubLinkStateStore;
use super::model::{IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions};
use super::service::{IronhubManifestUrl, execute_reborn_ironhub_service_command};

pub const IRONHUB_SEARCH_CAPABILITY_ID: &str = "builtin.ironhub_search";
pub const IRONHUB_INFO_CAPABILITY_ID: &str = "builtin.ironhub_info";
pub const IRONHUB_INSTALL_CAPABILITY_ID: &str = "builtin.ironhub_install";

const IRONHUB_CAPABILITY_IDS: [&str; 3] = [
    IRONHUB_SEARCH_CAPABILITY_ID,
    IRONHUB_INFO_CAPABILITY_ID,
    IRONHUB_INSTALL_CAPABILITY_ID,
];

pub fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.extend(manifests()?);
    let root = package
        .materialized_root()
        .map_err(|error| ExtensionError::InvalidManifest {
            reason: format!("IronHub built-in package must have a materialized root: {error}"),
        })?
        .clone();
    ExtensionPackage::from_manifest(package.manifest, root)
}

pub fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    link_state: Arc<IronhubLinkStateStore>,
    manifest_url: IronhubManifestUrl,
) -> Result<(), HostApiError> {
    let handler = Arc::new(IronHubCapabilityHandler {
        skill_management,
        extension_management,
        link_state,
        manifest_url,
    });
    for capability_id in IRONHUB_CAPABILITY_IDS {
        registry.insert_handler(CapabilityId::new(capability_id)?, handler.clone());
    }
    Ok(())
}

fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        capability_manifest(
            IRONHUB_SEARCH_CAPABILITY_ID,
            "Browse or search the signed IronHub catalog of installable tools and skills. Call it with no query to list the whole catalog; pass a query only to filter.",
            vec![EffectKind::Network],
            PermissionMode::Allow,
        )?,
        capability_manifest(
            IRONHUB_INFO_CAPABILITY_ID,
            "Inspect one signed IronHub catalog entry, including provenance and its signed artifact digest.",
            vec![EffectKind::Network],
            PermissionMode::Allow,
        )?,
        capability_manifest(
            IRONHUB_INSTALL_CAPABILITY_ID,
            "Install a verified IronHub tool or skill through the Reborn extension or skill manager. Unverified community entries require explicit operator acknowledgement and cannot be installed by this model-facing capability.",
            vec![
                EffectKind::Network,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            PermissionMode::Ask,
        )?,
    ])
}

fn capability_manifest(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
) -> Result<CapabilityManifest, ExtensionError> {
    let schema_name = id.strip_prefix("builtin.").unwrap_or(id).replace('.', "-");
    let mut origin_gate_matrix = OriginGateMatrix::builtin_loop_run_seed(id);
    if id == IRONHUB_INSTALL_CAPABILITY_ID {
        origin_gate_matrix.product = OriginGatePolicy::ConsentSufficient;
    }
    Ok(CapabilityManifest {
        id: CapabilityId::new(id)?,
        description: description.to_string(),
        effects,
        default_permission,
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.input.v1.json"
        ))?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.output.v1.json"
        ))?),
        prompt_doc_ref: None,
        required_host_ports: vec![],
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(30_000)
                .set_output_bytes(32 * 1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(origin_gate_matrix),
    })
}

struct IronHubCapabilityHandler {
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    link_state: Arc<IronhubLinkStateStore>,
    manifest_url: IronhubManifestUrl,
}

#[derive(Debug, Deserialize)]
struct SearchInput {
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize)]
struct InfoInput {
    name: String,
    #[serde(default)]
    kind: Option<IronHubEntryKind>,
}

#[derive(Debug, Deserialize)]
struct InstallInput {
    name: String,
    #[serde(default)]
    kind: Option<IronHubEntryKind>,
    #[serde(default)]
    force: bool,
    expected_version: String,
    expected_artifact_digest: String,
}

#[async_trait]
impl FirstPartyCapabilityHandler for IronHubCapabilityHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let runtime_http_egress = request
            .services
            .runtime_http_egress
            .clone()
            .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Executor))?;
        let command = match request.capability_id.as_str() {
            IRONHUB_SEARCH_CAPABILITY_ID => {
                let input: SearchInput = parse_input(request.input)?;
                IronHubCommand::Search { query: input.query }
            }
            IRONHUB_INFO_CAPABILITY_ID => {
                let input: InfoInput = parse_input(request.input)?;
                IronHubCommand::Info {
                    name: input.name,
                    kind: input.kind,
                }
            }
            IRONHUB_INSTALL_CAPABILITY_ID => {
                let input: InstallInput = parse_input(request.input)?;
                IronHubCommand::Install {
                    name: input.name,
                    options: IronHubInstallOptions {
                        kind: input.kind,
                        force: input.force,
                        acknowledge_unverified: false,
                        expected_version: Some(input.expected_version),
                        expected_artifact_digest: Some(input.expected_artifact_digest),
                        private_manifest_url: None,
                    },
                }
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let response = execute_reborn_ironhub_service_command(
            Arc::clone(&self.skill_management),
            Arc::clone(&self.extension_management),
            runtime_http_egress,
            Arc::clone(&self.link_state),
            self.manifest_url.clone(),
            request.scope,
            command,
        )
        .await
        .map_err(capability_error)?;
        let output = serde_json::to_value(response).map_err(|error| {
            tracing::debug!(%error, "failed to serialize IronHub capability response");
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode)
        })?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            ResourceUsage {
                wall_clock_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                ..ResourceUsage::default()
            },
        ))
    }
}

fn parse_input<T>(input: serde_json::Value) -> Result<T, FirstPartyCapabilityError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(input).map_err(|error| {
        tracing::debug!(%error, "failed to deserialize IronHub capability input");
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
    })
}

fn capability_error(error: IronHubCommandError) -> FirstPartyCapabilityError {
    let kind = match error {
        IronHubCommandError::InvalidInput { .. } => RuntimeDispatchErrorKind::InputEncode,
        IronHubCommandError::RuntimeHttpEgressUnavailable => RuntimeDispatchErrorKind::Executor,
        IronHubCommandError::Catalog { .. }
        | IronHubCommandError::Install { .. }
        | IronHubCommandError::Product(_) => RuntimeDispatchErrorKind::OperationFailed,
    };
    tracing::debug!(?kind, "IronHub capability dispatch failed");
    FirstPartyCapabilityError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::http::{
        RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse,
    };

    use super::*;

    struct CountingEgress {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl RuntimeHttpEgress for CountingEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(RuntimeHttpEgressError::Request {
                reason: "unexpected test egress".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            })
        }
    }

    #[test]
    fn command_errors_map_to_redacted_dispatch_categories() {
        let cases = [
            (
                IronHubCommandError::InvalidInput {
                    reason: "private".to_string(),
                },
                RuntimeDispatchErrorKind::InputEncode,
            ),
            (
                IronHubCommandError::RuntimeHttpEgressUnavailable,
                RuntimeDispatchErrorKind::Executor,
            ),
            (
                IronHubCommandError::Catalog {
                    reason: "private".to_string(),
                },
                RuntimeDispatchErrorKind::OperationFailed,
            ),
            (
                IronHubCommandError::Install {
                    reason: "private".to_string(),
                },
                RuntimeDispatchErrorKind::OperationFailed,
            ),
            (
                IronHubCommandError::Product(
                    ironclaw_product_contracts::error::ProductOperationFailure::InvalidBindingRequest {
                        reason: "private".to_string(),
                    },
                ),
                RuntimeDispatchErrorKind::OperationFailed,
            ),
        ];

        for (error, expected) in cases {
            let FirstPartyCapabilityError::Dispatch { kind, .. } = capability_error(error) else {
                panic!("IronHub failures must remain redacted dispatch failures");
            };
            assert_eq!(kind, expected);
        }
    }

    #[tokio::test]
    async fn model_install_dispatch_requires_pins_before_egress() {
        let services = crate::lifecycle_test_support::build_lifecycle_test_services(
            "ironhub-capability-pins",
            None,
            false,
        )
        .await;
        let scope = crate::lifecycle_test_support::webui_gate_resource_scope_for_owner(
            "ironhub-capability-pins",
        );
        let egress = Arc::new(CountingEgress {
            calls: AtomicUsize::new(0),
        });
        let runtime_egress: Arc<dyn RuntimeHttpEgress> = egress.clone();
        let handler = IronHubCapabilityHandler {
            skill_management: services.skill_management,
            extension_management: services.extension_management,
            link_state: Arc::new(IronhubLinkStateStore::new(Arc::new(InMemoryBackend::new()))),
            manifest_url: IronhubManifestUrl::default(),
        };

        let error = handler
            .dispatch(FirstPartyCapabilityRequest::request_for_test(
                CapabilityId::new(IRONHUB_INSTALL_CAPABILITY_ID).expect("capability id"),
                scope,
                serde_json::json!({"name": "example-tool"}),
                Some(runtime_egress),
            ))
            .await
            .expect_err("missing approval pins fail at the capability boundary");
        assert!(matches!(
            error,
            FirstPartyCapabilityError::Dispatch {
                kind: RuntimeDispatchErrorKind::InputEncode,
                ..
            }
        ));
        assert_eq!(
            egress.calls.load(Ordering::SeqCst),
            0,
            "invalid model input must not fetch the signed catalog"
        );

        let input = serde_json::from_value::<InstallInput>(serde_json::json!({
            "name": "example-tool",
            "expected_version": "1.2.3",
            "expected_artifact_digest": "sha256:abc"
        }))
        .expect("complete approval pins deserialize");
        assert_eq!(input.expected_version, "1.2.3");
        assert_eq!(input.expected_artifact_digest, "sha256:abc");
    }
}
