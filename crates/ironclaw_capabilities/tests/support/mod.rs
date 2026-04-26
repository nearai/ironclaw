#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::*;
use ironclaw_resources::{InMemoryResourceGovernor, ResourceGovernor};

pub struct RuntimeDispatcher<'a> {
    governor: GovernorBinding<'a>,
    runtime: Option<RuntimeKind>,
}

enum GovernorBinding<'a> {
    Borrowed(&'a InMemoryResourceGovernor),
    Owned(Arc<InMemoryResourceGovernor>),
}

impl GovernorBinding<'_> {
    fn governor(&self) -> &InMemoryResourceGovernor {
        match self {
            Self::Borrowed(governor) => governor,
            Self::Owned(governor) => governor.as_ref(),
        }
    }
}

impl<'a> RuntimeDispatcher<'a> {
    pub fn new<F>(
        _registry: &ExtensionRegistry,
        _filesystem: &'a F,
        governor: &'a InMemoryResourceGovernor,
    ) -> Self {
        Self {
            governor: GovernorBinding::Borrowed(governor),
            runtime: None,
        }
    }

    pub fn with_wasm_runtime<T>(mut self, _runtime: &T) -> Self {
        self.runtime = Some(RuntimeKind::Wasm);
        self
    }

    pub fn with_mcp_runtime<T>(mut self, _runtime: &T) -> Self {
        self.runtime = Some(RuntimeKind::Mcp);
        self
    }
}

impl RuntimeDispatcher<'static> {
    pub fn from_arcs<F>(
        _registry: Arc<ExtensionRegistry>,
        _filesystem: Arc<F>,
        governor: Arc<InMemoryResourceGovernor>,
    ) -> Self {
        Self {
            governor: GovernorBinding::Owned(governor),
            runtime: None,
        }
    }

    pub fn with_wasm_runtime_arc<T>(mut self, _runtime: Arc<T>) -> Self {
        self.runtime = Some(RuntimeKind::Wasm);
        self
    }
}

#[async_trait]
impl CapabilityDispatcher for RuntimeDispatcher<'_> {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let runtime = self.runtime.ok_or(DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Wasm,
        })?;
        let reservation = self
            .governor
            .governor()
            .reserve(request.scope.clone(), request.estimate.clone())
            .map_err(|_| DispatchError::UnsupportedRuntime {
                capability: request.capability_id.clone(),
                runtime,
            })?;
        let usage = ResourceUsage::default();
        let receipt = self
            .governor
            .governor()
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::UnsupportedRuntime {
                capability: request.capability_id.clone(),
                runtime,
            })?;

        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id.clone(),
            provider: provider_for(&request.capability_id),
            runtime,
            output: request.input,
            usage,
            receipt,
        })
    }
}

fn provider_for(capability_id: &CapabilityId) -> ExtensionId {
    let provider = capability_id
        .as_str()
        .split_once('.')
        .map(|(provider, _)| provider)
        .unwrap_or("echo");
    ExtensionId::new(provider).unwrap_or_else(|_| ExtensionId::new("echo").unwrap())
}
