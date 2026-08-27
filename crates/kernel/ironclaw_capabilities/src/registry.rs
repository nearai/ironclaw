//! Central capability dispatch registry.
//!
//! This is the registration surface extension hosts and built-in providers
//! converge on: descriptors are registered with a prebound handler, and
//! dispatch resolution becomes a map lookup by `CapabilityId`.

use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::extension::Extension;
use ironclaw_extension_contracts::tool_adapter::{
    ToolAdapter, ToolCall, ToolCallResources, ToolError, ToolPorts,
};
use ironclaw_host_api::{
    capability::CapabilityDescriptor,
    dispatch::{CapabilityDispatchRequest, DispatchError, DispatchFailureKind},
    ids::{CapabilityId, ExtensionId},
    resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
    runtime::RuntimeKind,
};

use crate::dispatch::{
    BoundCapabilityAdapter, ResolvedCapability, RuntimeAdapterResult, ToolResolver,
};

/// In-memory capability registration table.
#[derive(Default)]
pub struct CapabilityDispatchRegistry {
    entries: BTreeMap<CapabilityId, RegisteredCapability>,
}

#[derive(Clone)]
struct RegisteredCapability {
    descriptor: Arc<CapabilityDescriptor>,
    handler: Arc<dyn BoundCapabilityAdapter>,
}

impl CapabilityDispatchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a prebound capability handler.
    ///
    /// Duplicate capability ids are rejected before the new handler is stored.
    pub fn register(
        &mut self,
        descriptor: CapabilityDescriptor,
        handler: Arc<dyn BoundCapabilityAdapter>,
    ) -> Result<(), CapabilityRegistrationError> {
        let capability_id = descriptor.id.clone();
        match self.entries.entry(capability_id.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(RegisteredCapability {
                    descriptor: Arc::new(descriptor),
                    handler,
                });
                Ok(())
            }
            Entry::Occupied(existing) => Err(CapabilityRegistrationError::DuplicateCapability {
                capability_id,
                existing_provider: existing.get().descriptor.provider.clone(),
            }),
        }
    }

    /// Register every capability declared by one live extension.
    pub fn register_extension(
        &mut self,
        extension: Arc<dyn Extension>,
    ) -> Result<(), CapabilityRegistrationError> {
        let contract = extension.contract();
        if contract.capabilities.is_empty() {
            return Ok(());
        }
        let Some(adapter) = extension.capability_adapter() else {
            return Err(CapabilityRegistrationError::MissingCapabilityAdapter {
                extension_id: contract.identity.extension_id.clone(),
            });
        };
        for descriptor in &contract.capabilities {
            self.register(
                descriptor.clone(),
                Arc::new(ToolAdapterCapabilityHandler {
                    adapter: Arc::clone(&adapter),
                    runtime: descriptor.runtime,
                }),
            )?;
        }
        Ok(())
    }

    pub fn descriptor(&self, capability_id: &CapabilityId) -> Option<Arc<CapabilityDescriptor>> {
        self.entries
            .get(capability_id)
            .map(|entry| Arc::clone(&entry.descriptor))
    }

    pub fn descriptors(&self) -> impl Iterator<Item = Arc<CapabilityDescriptor>> + '_ {
        self.entries
            .values()
            .map(|entry| Arc::clone(&entry.descriptor))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

struct ToolAdapterCapabilityHandler {
    adapter: Arc<dyn ToolAdapter>,
    runtime: RuntimeKind,
}

#[async_trait]
impl BoundCapabilityAdapter for ToolAdapterCapabilityHandler {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let capability_id = request.capability_id.clone();
        let scope = request.scope.clone();
        let estimate = request.estimate.clone();
        let reservation_id = request
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id);
        let call = ToolCall {
            capability_id: request.capability_id,
            scope: request.scope,
            input: request.input,
            deadline: None,
            resources: ToolCallResources {
                estimate: request.estimate,
                mounts: request.mounts,
                reservation: request.resource_reservation,
            },
        };
        let ports = ToolPorts { egress: None };
        let result =
            self.adapter.invoke(call, &ports).await.map_err(|error| {
                tool_error_to_dispatch_error(&capability_id, self.runtime, error)
            })?;
        let output_bytes = serde_json::to_vec(&result.output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(result.output_bytes);
        let usage = ResourceUsage {
            output_bytes,
            ..ResourceUsage::default()
        };
        Ok(RuntimeAdapterResult {
            output: result.output,
            display_preview: result.display_preview,
            output_bytes,
            usage: usage.clone(),
            receipt: ResourceReceipt {
                id: reservation_id.unwrap_or_default(),
                scope,
                status: ReservationStatus::Reconciled,
                estimate,
                actual: Some(usage),
            },
        })
    }
}

fn tool_error_to_dispatch_error(
    capability_id: &CapabilityId,
    runtime: RuntimeKind,
    error: ToolError,
) -> DispatchError {
    match error {
        ToolError::AuthRequired { requirement } => DispatchError::AuthRequired {
            capability: capability_id.clone(),
            requirement,
        },
        ToolError::Rejected {
            kind,
            diagnostic,
            detail,
            ..
        } => DispatchError::Rejected {
            // Runtime provenance belongs to the trusted registry binding, not
            // to the extension-supplied error payload.
            runtime: Some(runtime),
            kind: DispatchFailureKind::Runtime(kind),
            diagnostic,
            detail,
        },
    }
}

impl ToolResolver for CapabilityDispatchRegistry {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        let entry = self.entries.get(capability_id)?;
        Some(ResolvedCapability {
            provider: entry.descriptor.provider.clone(),
            runtime: entry.descriptor.runtime,
            adapter: Arc::clone(&entry.handler),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityRegistrationError {
    #[error("capability `{capability_id}` is already registered by provider `{existing_provider}`")]
    DuplicateCapability {
        capability_id: CapabilityId,
        existing_provider: ExtensionId,
    },
    #[error("extension `{extension_id}` declares capabilities but has no capability adapter")]
    MissingCapabilityAdapter { extension_id: ExtensionId },
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_extension_contracts::extension::{
        Extension, ExtensionContract, ExtensionInstanceId, ExtensionRuntimeIdentity,
    };
    use ironclaw_extension_contracts::tool_adapter::{
        ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
    };
    use ironclaw_host_api::{
        capability::{CapabilityDescriptor, EffectKind, PermissionMode},
        dispatch::{
            CapabilityDispatchRequest, DispatchError, DispatchFailureDetail, DispatchFailureKind,
            ProviderDiagnostic, ProviderErrorCode, RuntimeDispatchErrorKind,
            UntrustedProviderMessage,
        },
        ids::{ExtensionId, InvocationId, ProductKind, TenantId, UserId},
        invocation::InvocationOrigin,
        resource::{ResourceEstimate, ResourceProfile, ResourceScope},
        runtime::{RuntimeKind, TrustClass},
    };
    use serde_json::json;

    use super::*;
    use crate::RuntimeAdapterResult;

    struct NoopHandler;

    #[async_trait]
    impl BoundCapabilityAdapter for NoopHandler {
        async fn dispatch_json(
            &self,
            _request: CapabilityDispatchRequest,
        ) -> Result<RuntimeAdapterResult, DispatchError> {
            unreachable!("registry tests only resolve handlers")
        }
    }

    struct NoopToolAdapter;

    #[async_trait]
    impl ToolAdapter for NoopToolAdapter {
        async fn invoke(
            &self,
            _call: ToolCall,
            _ports: &ToolPorts<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult {
                output: json!({"ok": true}),
                display_preview: None,
                output_bytes: 11,
            })
        }
    }

    struct TestExtension {
        contract: ExtensionContract,
        adapter: Option<Arc<dyn ToolAdapter>>,
    }

    impl Extension for TestExtension {
        fn contract(&self) -> &ExtensionContract {
            &self.contract
        }

        fn capability_adapter(&self) -> Option<Arc<dyn ToolAdapter>> {
            self.adapter.clone()
        }
    }

    fn descriptor(id: &str, provider: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::new(id).expect("capability id"),
            provider: ExtensionId::new(provider).expect("provider"),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::FirstParty,
            description: "test capability".to_string(),
            parameters_schema: json!({"type": "object"}),
            effects: vec![EffectKind::ReadFilesystem],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            max_egress_bytes: None,
            resource_profile: Some(ResourceProfile {
                default_estimate: ResourceEstimate::default(),
                hard_ceiling: None,
            }),
            origin_gate_matrix: None,
            standard_op: None,
        }
    }

    fn extension(provider: &str, adapter: Option<Arc<dyn ToolAdapter>>) -> Arc<dyn Extension> {
        let descriptor = descriptor(&format!("{provider}.echo"), provider);
        Arc::new(TestExtension {
            contract: ExtensionContract {
                identity: ExtensionRuntimeIdentity {
                    extension_id: ExtensionId::new(provider).expect("extension id"),
                    instance_id: ExtensionInstanceId::new(format!("{provider}:install"))
                        .expect("instance id"),
                },
                display_name: provider.to_string(),
                capabilities: vec![descriptor],
                channel: None,
            },
            adapter,
        })
    }

    #[test]
    fn duplicate_capability_registration_is_rejected() {
        let mut registry = CapabilityDispatchRegistry::new();
        registry
            .register(descriptor("test.echo", "provider-a"), Arc::new(NoopHandler))
            .expect("first registration");

        let error = registry
            .register(descriptor("test.echo", "provider-b"), Arc::new(NoopHandler))
            .expect_err("duplicate rejected");

        assert!(matches!(
            error,
            CapabilityRegistrationError::DuplicateCapability { .. }
        ));
    }

    #[test]
    fn registry_resolves_prebound_capability_handler() {
        let mut registry = CapabilityDispatchRegistry::new();
        let capability_id = CapabilityId::new("test.echo").expect("capability id");
        registry
            .register(descriptor("test.echo", "provider-a"), Arc::new(NoopHandler))
            .expect("registration");

        let resolved = registry.resolve(&capability_id).expect("resolved");
        assert_eq!(resolved.provider.as_str(), "provider-a");
        assert_eq!(resolved.runtime, RuntimeKind::FirstParty);
    }

    #[test]
    fn extension_registration_registers_declared_capabilities() {
        let mut registry = CapabilityDispatchRegistry::new();
        registry
            .register_extension(extension("provider-a", Some(Arc::new(NoopToolAdapter))))
            .expect("extension registration");

        let capability_id = CapabilityId::new("provider-a.echo").expect("capability id");
        assert!(registry.resolve(&capability_id).is_some());
        assert!(registry.descriptor(&capability_id).is_some());
    }

    #[test]
    fn extension_registration_requires_adapter_for_declared_capabilities() {
        let mut registry = CapabilityDispatchRegistry::new();
        let error = registry
            .register_extension(extension("provider-a", None))
            .expect_err("missing adapter rejected");

        assert!(matches!(
            error,
            CapabilityRegistrationError::MissingCapabilityAdapter { .. }
        ));
    }

    struct RejectingToolAdapter;

    #[async_trait]
    impl ToolAdapter for RejectingToolAdapter {
        async fn invoke(
            &self,
            _call: ToolCall,
            _ports: &ToolPorts<'_>,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError::Rejected {
                kind: RuntimeDispatchErrorKind::PolicyDenied,
                diagnostic: Some(Box::new(ProviderDiagnostic {
                    code: Some(ProviderErrorCode::new("channel_not_found")),
                    message: Some(UntrustedProviderMessage::new("no such channel")),
                    retry_after: None,
                })),
                detail: None,
            })
        }
    }

    fn sample_dispatch_request(capability_id: &str) -> CapabilityDispatchRequest {
        CapabilityDispatchRequest {
            authorized_descriptor: None,
            capability_id: CapabilityId::new(capability_id).expect("capability id"),
            scope: ResourceScope {
                tenant_id: TenantId::new("tenant-a").expect("tenant id"),
                user_id: UserId::new("user-a").expect("user id"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            authenticated_actor_user_id: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").expect("product kind")),
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        }
    }

    #[tokio::test]
    async fn tool_adapter_rejection_maps_to_dispatch_error_rejected_preserving_kind_and_diagnostic()
    {
        let mut registry = CapabilityDispatchRegistry::new();
        registry
            .register_extension(extension(
                "provider-a",
                Some(Arc::new(RejectingToolAdapter)),
            ))
            .expect("extension registration");

        let resolved = registry
            .resolve(&CapabilityId::new("provider-a.echo").expect("capability id"))
            .expect("resolved");

        let error = resolved
            .adapter
            .dispatch_json(sample_dispatch_request("provider-a.echo"))
            .await
            .expect_err("rejection propagated");

        match error {
            DispatchError::Rejected {
                runtime,
                kind,
                diagnostic,
                ..
            } => {
                assert_eq!(runtime, Some(RuntimeKind::FirstParty));
                assert_eq!(
                    kind,
                    DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::PolicyDenied)
                );
                let diagnostic = diagnostic.expect("diagnostic preserved");
                assert_eq!(
                    diagnostic.code.as_ref().map(ProviderErrorCode::as_str),
                    Some("channel_not_found")
                );
                assert_eq!(
                    diagnostic
                        .message
                        .as_ref()
                        .map(UntrustedProviderMessage::as_str),
                    Some("no such channel")
                );
            }
            other => panic!("expected DispatchError::Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_adapter_rejection_keeps_host_summary_separate_from_vendor_cause() {
        struct RejectedToolAdapter;

        #[async_trait]
        impl ToolAdapter for RejectedToolAdapter {
            async fn invoke(
                &self,
                _call: ToolCall,
                _ports: &ToolPorts<'_>,
            ) -> Result<ToolResult, ToolError> {
                Err(ToolError::Rejected {
                    kind: RuntimeDispatchErrorKind::Backend,
                    diagnostic: Some(Box::new(ProviderDiagnostic {
                        code: None,
                        message: Some(UntrustedProviderMessage::new("vendor backend returned 503")),
                        retry_after: None,
                    })),
                    detail: Some(DispatchFailureDetail::HostSummary {
                        summary: ironclaw_host_api::safe_summary::SafeSummary::new(
                            "the tool's backend failed",
                        )
                        .unwrap(),
                        detail: None,
                    }),
                })
            }
        }

        let mut registry = CapabilityDispatchRegistry::new();
        registry
            .register_extension(extension("provider-a", Some(Arc::new(RejectedToolAdapter))))
            .expect("extension registration");
        let resolved = registry
            .resolve(&CapabilityId::new("provider-a.echo").expect("capability id"))
            .expect("resolved");

        let error = resolved
            .adapter
            .dispatch_json(sample_dispatch_request("provider-a.echo"))
            .await
            .expect_err("failure propagated");
        let DispatchError::Rejected {
            diagnostic, detail, ..
        } = error
        else {
            panic!("expected Rejected dispatch error");
        };
        assert_eq!(
            diagnostic
                .as_ref()
                .and_then(|diagnostic| diagnostic.message.as_ref())
                .map(|message| message.as_str()),
            Some("vendor backend returned 503")
        );
        assert!(matches!(
            detail,
            Some(DispatchFailureDetail::HostSummary { summary, detail })
                if summary.as_str() == "the tool's backend failed" && detail.is_none()
        ));
    }

    #[tokio::test]
    async fn tool_adapter_rejection_without_host_summary_carries_only_vendor_cause() {
        struct RejectedToolAdapter;

        #[async_trait]
        impl ToolAdapter for RejectedToolAdapter {
            async fn invoke(
                &self,
                _call: ToolCall,
                _ports: &ToolPorts<'_>,
            ) -> Result<ToolResult, ToolError> {
                Err(ToolError::Rejected {
                    kind: RuntimeDispatchErrorKind::Guest,
                    diagnostic: Some(Box::new(ProviderDiagnostic {
                        code: None,
                        message: Some(UntrustedProviderMessage::new("guest trapped")),
                        retry_after: None,
                    })),
                    detail: None,
                })
            }
        }

        let mut registry = CapabilityDispatchRegistry::new();
        registry
            .register_extension(extension("provider-a", Some(Arc::new(RejectedToolAdapter))))
            .expect("extension registration");
        let resolved = registry
            .resolve(&CapabilityId::new("provider-a.echo").expect("capability id"))
            .expect("resolved");

        let error = resolved
            .adapter
            .dispatch_json(sample_dispatch_request("provider-a.echo"))
            .await
            .expect_err("failure propagated");
        let DispatchError::Rejected {
            diagnostic, detail, ..
        } = error
        else {
            panic!("expected Rejected dispatch error");
        };
        assert_eq!(
            diagnostic
                .as_ref()
                .and_then(|diagnostic| diagnostic.message.as_ref())
                .map(|message| message.as_str()),
            Some("guest trapped")
        );
        assert_eq!(detail, None);
    }
}
