//! Central capability dispatch registry.
//!
//! This is the registration surface extension hosts and built-in providers
//! converge on: descriptors are registered with a prebound handler, and
//! dispatch resolution becomes a map lookup by `CapabilityId`.

use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityDispatchRequest, CapabilityId, DispatchError,
    DispatchFailureDetail, Extension, ExtensionId, ReservationStatus, ResourceReceipt,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeKind, ToolAdapter, ToolCall, ToolCallResources,
    ToolError, ToolPorts,
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
        ToolError::AuthRequired {
            required_secrets,
            credential_requirements,
        } => DispatchError::AuthRequired {
            capability: capability_id.clone(),
            required_secrets,
            credential_requirements,
        },
        ToolError::Failed {
            kind,
            safe_summary,
            model_visible_cause,
        } => runtime_dispatch_error(runtime, kind, safe_summary, model_visible_cause),
    }
}

fn runtime_dispatch_error(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
    safe_summary: Option<String>,
    model_visible_cause: Option<String>,
) -> DispatchError {
    match runtime {
        RuntimeKind::Mcp => DispatchError::Mcp {
            kind,
            model_visible_cause,
        },
        RuntimeKind::Wasm => DispatchError::Wasm {
            kind,
            model_visible_cause,
        },
        RuntimeKind::Script => DispatchError::Script {
            kind,
            model_visible_cause,
        },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty {
            kind,
            safe_summary,
            detail: model_visible_cause.map(|text| DispatchFailureDetail::Diagnostic { text }),
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
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityDispatchRequest, DispatchError, EffectKind, Extension,
        ExtensionContract, ExtensionId, ExtensionInstanceId, ExtensionRuntimeIdentity,
        PermissionMode, ResourceEstimate, ResourceProfile, RuntimeKind, ToolAdapter, ToolCall,
        ToolError, ToolPorts, ToolResult, TrustClass,
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
}
