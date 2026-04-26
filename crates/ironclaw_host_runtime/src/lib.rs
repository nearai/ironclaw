//! Host runtime composition helpers for IronClaw Reborn.
//!
//! This crate is intentionally composition-only. It wires existing host services
//! together without moving authorization, dispatch, process lifecycle, approval,
//! or run-state responsibilities out of their owning crates.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::ApprovalResolver;
use ironclaw_authorization::{CapabilityDispatchAuthorizer, CapabilityLeaseStore};
use ironclaw_capabilities::{CapabilityHost, CapabilityObligationHandler, DispatchProcessExecutor};
use ironclaw_dispatcher::{
    DispatchError, RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{AuditSink, EventSink};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDispatchError, CapabilityDispatchFailureKind, CapabilityDispatcher, CapabilityId,
    ExtensionId, RuntimeKind,
};
use ironclaw_mcp::{McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_processes::{
    ProcessExecutor, ProcessHost, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_scripts::{ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_wasm::{CapabilityInvocation, WasmExecutionRequest, WasmRuntime};

/// Dispatcher adapter for the concrete WASM runtime crate.
pub struct WasmRuntimeAdapter {
    runtime: Arc<WasmRuntime>,
}

impl WasmRuntimeAdapter {
    pub fn new(runtime: Arc<WasmRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for WasmRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let capability_id = request.capability_id.clone();
        let provider = request.descriptor.provider.clone();
        let runtime = request.descriptor.runtime;
        let execution = self
            .runtime
            .execute_extension_json(
                request.filesystem,
                request.governor,
                WasmExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: CapabilityInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(|_| {
                runtime_dispatch_error(
                    &capability_id,
                    &provider,
                    runtime,
                    CapabilityDispatchFailureKind::Wasm,
                )
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Dispatcher adapter for the concrete script executor port.
pub struct ScriptRuntimeAdapter {
    runtime: Arc<dyn ScriptExecutor>,
}

impl ScriptRuntimeAdapter {
    pub fn new<T>(runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        let runtime: Arc<dyn ScriptExecutor> = runtime;
        Self { runtime }
    }

    pub fn from_dyn(runtime: Arc<dyn ScriptExecutor>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for ScriptRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let capability_id = request.capability_id.clone();
        let provider = request.descriptor.provider.clone();
        let runtime = request.descriptor.runtime;
        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                ScriptExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: ScriptInvocation {
                        input: request.input,
                    },
                },
            )
            .map_err(|_| {
                runtime_dispatch_error(
                    &capability_id,
                    &provider,
                    runtime,
                    CapabilityDispatchFailureKind::Script,
                )
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Dispatcher adapter for the concrete MCP executor port.
pub struct McpRuntimeAdapter {
    runtime: Arc<dyn McpExecutor>,
}

impl McpRuntimeAdapter {
    pub fn new<T>(runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        let runtime: Arc<dyn McpExecutor> = runtime;
        Self { runtime }
    }

    pub fn from_dyn(runtime: Arc<dyn McpExecutor>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for McpRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let capability_id = request.capability_id.clone();
        let provider = request.descriptor.provider.clone();
        let runtime = request.descriptor.runtime;
        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                McpExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: McpInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(|_| {
                runtime_dispatch_error(
                    &capability_id,
                    &provider,
                    runtime,
                    CapabilityDispatchFailureKind::Mcp,
                )
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Composition root for the Reborn host/runtime vertical slice.
///
/// `HostRuntimeServices` owns shared service handles and can build the narrow
/// service facades used by callers:
///
/// - `RuntimeDispatcher` for already-authorized runtime dispatch
/// - `CapabilityHost` for caller-facing invocation/spawn workflows
/// - `ProcessHost` for lifecycle/result/output/cancellation operations
///
/// It is deliberately not an authority engine, dispatcher, process manager, or
/// lifecycle store. Those responsibilities remain in their owning crates.
pub struct HostRuntimeServices<F, G, S, R, A>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
    A: CapabilityDispatchAuthorizer + 'static,
{
    registry: Arc<ExtensionRegistry>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    authorizer: Arc<A>,
    process_services: ProcessServices<S, R>,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    wasm_runtime: Option<Arc<WasmRuntime>>,
    script_runtime: Option<Arc<dyn ScriptExecutor>>,
    mcp_runtime: Option<Arc<dyn McpExecutor>>,
    event_sink: Option<Arc<dyn EventSink>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    obligation_handler: Option<Arc<dyn CapabilityObligationHandler>>,
}

impl<F, G, S, R, A> HostRuntimeServices<F, G, S, R, A>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
    A: CapabilityDispatchAuthorizer + 'static,
{
    pub fn new(
        registry: Arc<ExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        authorizer: Arc<A>,
        process_services: ProcessServices<S, R>,
    ) -> Self {
        Self {
            registry,
            filesystem,
            governor,
            authorizer,
            process_services,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            wasm_runtime: None,
            script_runtime: None,
            mcp_runtime: None,
            event_sink: None,
            audit_sink: None,
            obligation_handler: None,
        }
    }

    pub fn registry(&self) -> Arc<ExtensionRegistry> {
        Arc::clone(&self.registry)
    }

    pub fn filesystem(&self) -> Arc<F> {
        Arc::clone(&self.filesystem)
    }

    pub fn governor(&self) -> Arc<G> {
        Arc::clone(&self.governor)
    }

    pub fn authorizer(&self) -> Arc<A> {
        Arc::clone(&self.authorizer)
    }

    pub fn process_services(&self) -> &ProcessServices<S, R> {
        &self.process_services
    }

    pub fn process_host(&self) -> ProcessHost<'_> {
        self.process_services.host()
    }

    pub fn with_run_state<T>(mut self, run_state: Arc<T>) -> Self
    where
        T: RunStateStore + 'static,
    {
        self.run_state = Some(run_state);
        self
    }

    pub fn with_approval_requests<T>(mut self, approval_requests: Arc<T>) -> Self
    where
        T: ApprovalRequestStore + 'static,
    {
        self.approval_requests = Some(approval_requests);
        self
    }

    pub fn with_capability_leases<T>(mut self, capability_leases: Arc<T>) -> Self
    where
        T: CapabilityLeaseStore + 'static,
    {
        self.capability_leases = Some(capability_leases);
        self
    }

    pub fn with_wasm_runtime(mut self, runtime: Arc<WasmRuntime>) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    pub fn with_script_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        let runtime: Arc<dyn ScriptExecutor> = runtime;
        self.script_runtime = Some(runtime);
        self
    }

    pub fn with_mcp_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        let runtime: Arc<dyn McpExecutor> = runtime;
        self.mcp_runtime = Some(runtime);
        self
    }

    pub fn with_event_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: EventSink + 'static,
    {
        let sink: Arc<dyn EventSink> = sink;
        self.event_sink = Some(sink);
        self
    }

    pub fn with_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        let sink: Arc<dyn AuditSink> = sink;
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_obligation_handler<T>(mut self, handler: Arc<T>) -> Self
    where
        T: CapabilityObligationHandler + 'static,
    {
        let handler: Arc<dyn CapabilityObligationHandler> = handler;
        self.obligation_handler = Some(handler);
        self
    }

    pub fn approval_resolver(
        &self,
    ) -> Option<ApprovalResolver<'_, dyn ApprovalRequestStore, dyn CapabilityLeaseStore>> {
        let approval_requests = self.approval_requests.as_deref()?;
        let capability_leases = self.capability_leases.as_deref()?;
        let mut resolver = ApprovalResolver::new(approval_requests, capability_leases);
        if let Some(audit_sink) = &self.audit_sink {
            resolver = resolver.with_audit_sink(audit_sink.as_ref());
        }
        Some(resolver)
    }

    pub fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_arcs(
            Arc::clone(&self.registry),
            Arc::clone(&self.filesystem),
            Arc::clone(&self.governor),
        );

        if let Some(runtime) = &self.wasm_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Wasm,
                Arc::new(WasmRuntimeAdapter::new(Arc::clone(runtime))),
            );
        }
        if let Some(runtime) = &self.script_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Script,
                Arc::new(ScriptRuntimeAdapter::from_dyn(Arc::clone(runtime))),
            );
        }
        if let Some(runtime) = &self.mcp_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Mcp,
                Arc::new(McpRuntimeAdapter::from_dyn(Arc::clone(runtime))),
            );
        }
        if let Some(sink) = &self.event_sink {
            dispatcher = dispatcher.with_event_sink_arc(Arc::clone(sink));
        }

        dispatcher
    }

    pub fn runtime_dispatcher_arc(&self) -> Arc<RuntimeDispatcher<'static, F, G>> {
        Arc::new(self.runtime_dispatcher())
    }

    pub fn capability_host<'a, D, E>(
        &'a self,
        dispatcher: &'a D,
        executor: Arc<E>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
        E: ProcessExecutor + 'static,
    {
        self.configure_capability_host(
            CapabilityHost::new(self.registry.as_ref(), dispatcher, self.authorizer.as_ref())
                .with_process_services(&self.process_services, executor),
        )
    }

    pub fn capability_host_for_runtime_dispatcher<'a>(
        &'a self,
        dispatcher: &'a Arc<RuntimeDispatcher<'static, F, G>>,
    ) -> CapabilityHost<'a, RuntimeDispatcher<'static, F, G>> {
        let executor = Arc::new(DispatchProcessExecutor::new(Arc::clone(dispatcher)));
        self.capability_host(dispatcher.as_ref(), executor)
    }

    fn configure_capability_host<'a, D>(
        &'a self,
        host: CapabilityHost<'a, D>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
    {
        let mut host = host;
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        if let Some(approval_requests) = &self.approval_requests {
            host = host.with_approval_requests(approval_requests.as_ref());
        }
        if let Some(capability_leases) = &self.capability_leases {
            host = host.with_capability_leases(capability_leases.as_ref());
        }
        if let Some(obligation_handler) = &self.obligation_handler {
            host = host.with_obligation_handler(obligation_handler.as_ref());
        }
        host
    }
}

fn runtime_dispatch_error(
    capability_id: &CapabilityId,
    provider: &ExtensionId,
    runtime: RuntimeKind,
    kind: CapabilityDispatchFailureKind,
) -> DispatchError {
    CapabilityDispatchError::new(
        kind,
        capability_id.clone(),
        Some(provider.clone()),
        Some(runtime),
    )
}
