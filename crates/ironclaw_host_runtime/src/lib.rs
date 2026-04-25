//! Host runtime composition helpers for IronClaw Reborn.
//!
//! This crate is intentionally composition-only. It wires existing host services
//! together without moving authorization, dispatch, process lifecycle, approval,
//! or run-state responsibilities out of their owning crates.

use std::sync::Arc;

use ironclaw_authorization::{CapabilityDispatchAuthorizer, CapabilityLeaseStore};
use ironclaw_capabilities::{CapabilityDispatcher, CapabilityHost, DispatchProcessExecutor};
use ironclaw_dispatcher::RuntimeDispatcher;
use ironclaw_events::EventSink;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_mcp::McpExecutor;
use ironclaw_processes::{
    ProcessExecutor, ProcessHost, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_scripts::ScriptExecutor;
use ironclaw_wasm::WasmRuntime;

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

    pub fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_arcs(
            Arc::clone(&self.registry),
            Arc::clone(&self.filesystem),
            Arc::clone(&self.governor),
        );

        if let Some(runtime) = &self.wasm_runtime {
            dispatcher = dispatcher.with_wasm_runtime_arc(Arc::clone(runtime));
        }
        if let Some(runtime) = &self.script_runtime {
            dispatcher = dispatcher.with_script_runtime_arc(Arc::clone(runtime));
        }
        if let Some(runtime) = &self.mcp_runtime {
            dispatcher = dispatcher.with_mcp_runtime_arc(Arc::clone(runtime));
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
        host
    }
}
