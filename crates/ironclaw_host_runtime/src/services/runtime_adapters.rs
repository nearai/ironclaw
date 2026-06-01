use std::{
    collections::HashMap,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use futures_util::FutureExt;

use super::{
    CapabilityId, DenyWasmHostHttp, DispatchError, ExtensionRuntime, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, InvocationServicesResolutionRequest, InvocationServicesResolver,
    McpError, McpExecutionRequest, McpExecutor, McpInvocation, NetworkObligationPolicyStore,
    PlannerError, PreparedWitTool, ResourceGovernor, ResourceReservationId, ResourceScope,
    ResourceUsage, RootFilesystem, RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult,
    RuntimeDispatchErrorKind, RuntimeKind, ScriptError, ScriptExecutionRequest, ScriptExecutor,
    ScriptInvocation, SharedRuntimeHttpEgress, WasmError, WasmRuntimeCredentialProvider,
    WasmRuntimeHttpAdapter, WasmRuntimePolicyDiscarder, WitToolHost, WitToolRequest,
    WitToolRuntime, WitToolRuntimeConfig, plan_capability, runtime_http_egress,
};
use crate::FirstPartyCapabilityError;

pub(super) struct ServiceResolvedRuntimeAdapter<T> {
    inner: Arc<T>,
    invocation_services: Arc<dyn InvocationServicesResolver>,
}

// arch-exempt: large_file, runtime adapter composition is still centralized
// in HostRuntimeServices until the Reborn architecture decomposition tracked
// by nearai/ironclaw#3231 splits runtime wiring into focused modules.
impl<T> ServiceResolvedRuntimeAdapter<T> {
    pub(super) fn new(
        inner: Arc<T>,
        invocation_services: Arc<dyn InvocationServicesResolver>,
    ) -> Self {
        Self {
            inner,
            invocation_services,
        }
    }
}

#[async_trait]
impl<F, G, T> RuntimeAdapter<F, G> for ServiceResolvedRuntimeAdapter<T>
where
    F: RootFilesystem,
    G: ResourceGovernor,
    T: RuntimeAdapter<F, G>,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let plan =
            plan_capability(request.descriptor, request.runtime_policy).map_err(|error| {
                release_adapter_reservation(
                    request.governor,
                    request
                        .resource_reservation
                        .as_ref()
                        .map(|reservation| reservation.id),
                );
                dispatch_error_for_runtime(request.descriptor.runtime, planner_error_kind(&error))
            })?;
        self.invocation_services
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &request.scope,
                mounts: request.mounts.as_ref(),
            })
            .map_err(|error| {
                release_adapter_reservation(
                    request.governor,
                    request
                        .resource_reservation
                        .as_ref()
                        .map(|reservation| reservation.id),
                );
                dispatch_error_for_runtime(request.descriptor.runtime, error.kind())
            })?;

        self.inner.dispatch_json(request).await
    }
}

#[derive(Clone)]
pub(super) struct ScriptRuntimeAdapter {
    executor: Arc<dyn ScriptExecutor>,
}

impl ScriptRuntimeAdapter {
    pub(super) fn from_executor(executor: Arc<dyn ScriptExecutor>) -> Self {
        Self { executor }
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
        let execution = self
            .executor
            .execute_extension_json(
                request.governor,
                ScriptExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    mounts: request.mounts,
                    resource_reservation: request.resource_reservation,
                    invocation: ScriptInvocation {
                        input: request.input,
                    },
                },
            )
            .map_err(|error| DispatchError::Script {
                kind: script_error_kind(&error),
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

#[derive(Clone)]
pub(super) struct McpRuntimeAdapter {
    executor: Arc<dyn McpExecutor>,
}

impl McpRuntimeAdapter {
    pub(super) fn from_executor(executor: Arc<dyn McpExecutor>) -> Self {
        Self { executor }
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
        let execution = self
            .executor
            .execute_extension_json(
                request.governor,
                McpExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    resource_reservation: request.resource_reservation,
                    invocation: McpInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(|error| DispatchError::Mcp {
                kind: mcp_error_kind(&error),
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

#[derive(Clone)]
pub(super) struct FirstPartyRuntimeAdapter {
    registry: Arc<FirstPartyCapabilityRegistry>,
    invocation_services: Arc<dyn InvocationServicesResolver>,
}

impl FirstPartyRuntimeAdapter {
    pub(crate) fn from_registry(
        registry: Arc<FirstPartyCapabilityRegistry>,
        invocation_services: Arc<dyn InvocationServicesResolver>,
    ) -> Self {
        Self {
            registry,
            invocation_services,
        }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for FirstPartyRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    #[tracing::instrument(
        level = "debug",
        skip(self, request),
        fields(
            capability_id = %request.capability_id,
            scope = ?request.scope,
        )
    )]
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        tracing::debug!("first-party runtime adapter dispatch started");
        let Some(handler) = self.registry.get(request.capability_id) else {
            if let Some(reservation) = request.resource_reservation
                && let Err(error) = request.governor.release(reservation.id)
            {
                tracing::warn!(
                    reservation_id = %reservation.id,
                    error = %error,
                    "failed to release prepared resource reservation after missing first-party handler"
                );
            }
            tracing::debug!("first-party runtime adapter missing handler");
            return Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::UndeclaredCapability,
            });
        };

        let plan =
            plan_capability(request.descriptor, request.runtime_policy).map_err(|error| {
                tracing::debug!(
                    error_kind = %planner_error_kind(&error),
                    "first-party runtime adapter policy planning failed"
                );
                if let Some(reservation) = &request.resource_reservation {
                    release_first_party_reservation(request.governor, reservation.id);
                }
                DispatchError::FirstParty {
                    kind: planner_error_kind(&error),
                }
            })?;
        tracing::debug!(
            filesystem_backend = ?plan.filesystem_backend,
            process_backend = ?plan.process_backend,
            network_mode = ?plan.network_mode,
            secret_mode = ?plan.secret_mode,
            "first-party runtime adapter policy plan resolved"
        );
        let services = self
            .invocation_services
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &request.scope,
                mounts: request.mounts.as_ref(),
            })
            .map_err(|error| {
                tracing::debug!(
                    error_kind = %error.kind(),
                    "first-party runtime adapter service resolution failed"
                );
                if let Some(reservation) = &request.resource_reservation {
                    release_first_party_reservation(request.governor, reservation.id);
                }
                DispatchError::FirstParty { kind: error.kind() }
            })?;
        tracing::debug!("first-party runtime adapter services resolved");

        let used_prepared_reservation = request.resource_reservation.is_some();
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => request
                .governor
                .reserve(request.scope.clone(), request.estimate.clone())
                .map_err(|_| {
                    tracing::debug!("first-party runtime adapter resource reservation failed");
                    DispatchError::FirstParty {
                        kind: RuntimeDispatchErrorKind::Resource,
                    }
                })?,
        };
        tracing::debug!(
            reservation_id = %reservation.id,
            used_prepared_reservation,
            "first-party runtime adapter resource reservation ready"
        );

        tracing::debug!(
            reservation_id = %reservation.id,
            "first-party runtime adapter invoking handler"
        );
        let result = match AssertUnwindSafe(handler.dispatch(FirstPartyCapabilityRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            estimate: request.estimate,
            mounts: request.mounts,
            services,
            input: request.input,
        }))
        .catch_unwind()
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                tracing::debug!(
                    reservation_id = %reservation.id,
                    is_auth_required = error.is_auth_required(),
                    "first-party runtime adapter handler failed"
                );
                if let Err(acct_err) = account_or_release_failed_first_party_execution(
                    request.governor,
                    reservation.id,
                    error.usage(),
                ) {
                    tracing::warn!(
                        reservation_id = %reservation.id,
                        error = ?acct_err,
                        "first-party resource accounting failed on handler error; \
                         returning original handler error"
                    );
                }
                return match error {
                    FirstPartyCapabilityError::AuthRequired {
                        required_secrets, ..
                    } => Err(DispatchError::AuthRequired {
                        capability: request.capability_id.clone(),
                        required_secrets,
                    }),
                    FirstPartyCapabilityError::Dispatch { kind, .. } => {
                        Err(DispatchError::FirstParty { kind })
                    }
                };
            }
            Err(_) => {
                tracing::debug!(
                    reservation_id = %reservation.id,
                    "first-party runtime adapter handler panicked"
                );
                release_first_party_reservation(request.governor, reservation.id);
                return Err(DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Backend,
                });
            }
        };

        let output_bytes = serde_json::to_vec(&result.output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|_| {
                tracing::debug!(
                    reservation_id = %reservation.id,
                    "first-party runtime adapter output serialization failed"
                );
                release_first_party_reservation(request.governor, reservation.id);
                DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::OutputDecode,
                }
            })?;
        let mut usage = result.usage;
        usage.output_bytes = usage.output_bytes.max(output_bytes);
        let receipt = match request.governor.reconcile(reservation.id, usage.clone()) {
            Ok(receipt) => receipt,
            Err(_) => {
                tracing::debug!(
                    reservation_id = %reservation.id,
                    "first-party runtime adapter resource reconcile failed"
                );
                if let Err(release_error) = request.governor.release(reservation.id) {
                    tracing::warn!(
                        reservation_id = %reservation.id,
                        error = %release_error,
                        "failed to release first-party resource reservation after reconcile failure"
                    );
                }
                return Err(DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Resource,
                });
            }
        };
        tracing::debug!(
            reservation_id = %reservation.id,
            output_bytes,
            "first-party runtime adapter dispatch completed"
        );

        Ok(RuntimeAdapterResult {
            output: result.output,
            usage,
            receipt,
            output_bytes,
        })
    }
}

pub(super) struct WasmRuntimeAdapter {
    runtime: WitToolRuntime,
    host: WitToolHost,
    network_policy_store: Arc<NetworkObligationPolicyStore>,
    runtime_http_egress: SharedRuntimeHttpEgress,
    credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    prepared: Mutex<HashMap<String, Arc<PreparedWitTool>>>,
}

impl WasmRuntimeAdapter {
    pub(crate) fn new(
        runtime: WitToolRuntime,
        host: WitToolHost,
        network_policy_store: Arc<NetworkObligationPolicyStore>,
        runtime_http_egress: SharedRuntimeHttpEgress,
        credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    ) -> Self {
        Self {
            runtime,
            host,
            network_policy_store,
            runtime_http_egress,
            credential_provider,
            prepared: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn try_new(
        config: WitToolRuntimeConfig,
        host: WitToolHost,
        network_policy_store: Arc<NetworkObligationPolicyStore>,
        runtime_http_egress: SharedRuntimeHttpEgress,
        credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    ) -> Result<Self, WasmError> {
        Ok(Self::new(
            WitToolRuntime::new(config)?,
            host,
            network_policy_store,
            runtime_http_egress,
            credential_provider,
        ))
    }

    fn prepared_guard(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, Arc<PreparedWitTool>>>, DispatchError> {
        self.prepared.lock().map_err(|_| DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Executor,
        })
    }

    fn host_for_scope(&self, scope: &ResourceScope, capability_id: &CapabilityId) -> WitToolHost {
        let egress = runtime_http_egress(&self.runtime_http_egress);
        let Some(policy) = self.network_policy_store.get(scope, capability_id) else {
            return if egress.is_some() {
                self.host.clone().with_http(Arc::new(DenyWasmHostHttp))
            } else {
                self.host.clone()
            };
        };
        let Some(egress) = egress else {
            return self.host.clone().with_http(Arc::new(DenyWasmHostHttp));
        };
        let mut adapter =
            WasmRuntimeHttpAdapter::new(egress, scope.clone(), capability_id.clone(), policy)
                .with_policy_discarder(Arc::new(NetworkPolicyDiscarder {
                    store: Arc::clone(&self.network_policy_store),
                }));
        if let Some(provider) = &self.credential_provider {
            adapter = adapter.with_credential_provider(Arc::clone(provider));
        }
        self.host.clone().with_http(Arc::new(adapter))
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
        let module_path = match &request.package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => module
                .resolve_under(&request.package.root)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Manifest,
                })?,
            other => {
                return Err(DispatchError::Wasm {
                    kind: if other.kind() == RuntimeKind::Wasm {
                        RuntimeDispatchErrorKind::Manifest
                    } else {
                        RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
                    },
                });
            }
        };
        let cache_key = module_path.as_str().to_string();
        let prepared = self.prepared_guard()?.get(&cache_key).cloned();
        if let Some(prepared) = prepared {
            let host = self.host_for_scope(&request.scope, request.capability_id);
            return execute_prepared_wasm(&self.runtime, &prepared, host, request);
        }

        let wasm_bytes = request
            .filesystem
            .read_file(&module_path)
            .await
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::FilesystemDenied,
            })?;
        let prepared = Arc::new(
            self.runtime
                .prepare(request.package.id.as_str(), &wasm_bytes)
                .map_err(|error| DispatchError::Wasm {
                    kind: wasm_error_kind(&error),
                })?,
        );
        let prepared = {
            let mut prepared_cache = self.prepared_guard()?;
            if let Some(existing) = prepared_cache.get(&cache_key).cloned() {
                existing
            } else {
                prepared_cache.insert(cache_key, Arc::clone(&prepared));
                prepared
            }
        };
        let host = self.host_for_scope(&request.scope, request.capability_id);
        execute_prepared_wasm(&self.runtime, &prepared, host, request)
    }
}

#[derive(Debug, Clone)]
struct NetworkPolicyDiscarder {
    store: Arc<NetworkObligationPolicyStore>,
}

impl WasmRuntimePolicyDiscarder for NetworkPolicyDiscarder {
    fn discard(&self, scope: &ResourceScope, capability_id: &CapabilityId) {
        self.store.discard_for_capability(scope, capability_id);
    }
}

fn execute_prepared_wasm<G>(
    runtime: &WitToolRuntime,
    prepared: &PreparedWitTool,
    host: WitToolHost,
    request: RuntimeAdapterRequest<'_, impl RootFilesystem, G>,
) -> Result<RuntimeAdapterResult, DispatchError>
where
    G: ResourceGovernor,
{
    let reservation = match request.resource_reservation {
        Some(reservation) => reservation,
        None => request
            .governor
            .reserve(request.scope.clone(), request.estimate.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?,
    };
    let input_json = match serde_json::to_string(&request.input) {
        Ok(json) => json,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::InputEncode,
            });
        }
    };
    let context_json = wasm_invocation_context(request.capability_id);
    let execution = match runtime.execute(
        prepared,
        host,
        WitToolRequest::new(input_json).with_context(context_json),
    ) {
        Ok(execution) => execution,
        Err(error) => {
            if let Some(usage) = preserved_wasm_error_usage(&error) {
                account_or_release_failed_wasm_execution(request.governor, reservation.id, &usage)?;
            } else {
                release_wasm_reservation(request.governor, reservation.id);
            }
            return Err(DispatchError::Wasm {
                kind: wasm_error_kind(&error),
            });
        }
    };
    if execution.error.is_some() {
        account_or_release_failed_wasm_execution(
            request.governor,
            reservation.id,
            &execution.usage,
        )?;
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::OperationFailed,
        });
    }
    let Some(output_json) = execution.output_json else {
        account_or_release_failed_wasm_execution(
            request.governor,
            reservation.id,
            &execution.usage,
        )?;
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::InvalidResult,
        });
    };
    let output = match serde_json::from_str(&output_json) {
        Ok(output) => output,
        Err(_) => {
            account_or_release_failed_wasm_execution(
                request.governor,
                reservation.id,
                &execution.usage,
            )?;
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::OutputDecode,
            });
        }
    };
    let receipt = match request
        .governor
        .reconcile(reservation.id, execution.usage.clone())
    {
        Ok(receipt) => receipt,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            });
        }
    };
    Ok(RuntimeAdapterResult {
        output,
        output_bytes: execution.usage.output_bytes,
        usage: execution.usage,
        receipt,
    })
}

fn wasm_invocation_context(capability_id: &CapabilityId) -> String {
    serde_json::json!({
        "capability_id": capability_id.as_str(),
    })
    .to_string()
}

fn account_or_release_failed_first_party_execution<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    usage: Option<&ResourceUsage>,
) -> Result<(), DispatchError>
where
    G: ResourceGovernor + ?Sized,
{
    let Some(usage) = usage else {
        release_first_party_reservation(governor, reservation_id);
        return Ok(());
    };
    if !has_accountable_effects(usage) {
        release_first_party_reservation(governor, reservation_id);
        return Ok(());
    }

    if governor.reconcile(reservation_id, usage.clone()).is_err() {
        release_first_party_reservation(governor, reservation_id);
        return Err(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::Resource,
        });
    }

    Ok(())
}

fn release_first_party_reservation<G>(governor: &G, reservation_id: ResourceReservationId)
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
}

fn account_or_release_failed_wasm_execution<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    usage: &ResourceUsage,
) -> Result<(), DispatchError>
where
    G: ResourceGovernor + ?Sized,
{
    if !has_accountable_effects(usage) {
        release_wasm_reservation(governor, reservation_id);
        return Ok(());
    }

    if governor.reconcile(reservation_id, usage.clone()).is_err() {
        release_wasm_reservation(governor, reservation_id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Resource,
        });
    }

    Ok(())
}

fn release_wasm_reservation<G>(governor: &G, reservation_id: ResourceReservationId)
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
}

fn release_adapter_reservation<G>(governor: &G, reservation_id: Option<ResourceReservationId>)
where
    G: ResourceGovernor + ?Sized,
{
    let Some(reservation_id) = reservation_id else {
        return;
    };
    if let Err(error) = governor.release(reservation_id) {
        tracing::warn!(
            reservation_id = %reservation_id,
            error = %error,
            "failed to release prepared resource reservation after runtime policy rejection"
        );
    }
}

fn preserved_wasm_error_usage(error: &WasmError) -> Option<ResourceUsage> {
    if let WasmError::ExecutionFailed { usage, .. } = error
        && has_accountable_effects(usage)
    {
        Some(usage.clone())
    } else {
        None
    }
}

fn has_accountable_effects(usage: &ResourceUsage) -> bool {
    usage.usd != Default::default()
        || usage.input_tokens > 0
        || usage.output_tokens > 0
        || usage.wall_clock_ms > 0
        || usage.output_bytes > 0
        || usage.network_egress_bytes > 0
        || usage.process_count > 0
}

fn dispatch_error_for_runtime(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
) -> DispatchError {
    match runtime {
        RuntimeKind::Mcp => DispatchError::Mcp { kind },
        RuntimeKind::Script => DispatchError::Script { kind },
        RuntimeKind::Wasm => DispatchError::Wasm { kind },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty { kind },
    }
}

fn planner_error_kind(error: &PlannerError) -> RuntimeDispatchErrorKind {
    match error {
        PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. } => {
            RuntimeDispatchErrorKind::UnsupportedRunner
        }
        PlannerError::NetworkRequiredButNetworkModeIsDeny { .. } => {
            RuntimeDispatchErrorKind::NetworkDenied
        }
        PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. } => {
            RuntimeDispatchErrorKind::SecretDenied
        }
    }
}

fn script_error_kind(error: &ScriptError) -> RuntimeDispatchErrorKind {
    match error {
        ScriptError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        ScriptError::Backend { .. } => RuntimeDispatchErrorKind::Backend,
        ScriptError::UnsupportedRunner { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        ScriptError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        ScriptError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        ScriptError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::Manifest,
        ScriptError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        ScriptError::ExitFailure { .. } => RuntimeDispatchErrorKind::ExitFailure,
        ScriptError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        ScriptError::Timeout { .. } => RuntimeDispatchErrorKind::Executor,
        ScriptError::InvalidOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
    }
}

fn mcp_error_kind(error: &McpError) -> RuntimeDispatchErrorKind {
    match error {
        McpError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        McpError::Client { .. } => RuntimeDispatchErrorKind::Client,
        McpError::UnsupportedTransport { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::HostHttpEgressRequired { .. } => RuntimeDispatchErrorKind::NetworkDenied,
        McpError::ExternalStdioTransportUnsupported => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        McpError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        McpError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::Manifest,
        McpError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        McpError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
    }
}

fn wasm_error_kind(error: &WasmError) -> RuntimeDispatchErrorKind {
    match error {
        WasmError::EngineCreationFailed(_) => RuntimeDispatchErrorKind::Executor,
        WasmError::CompilationFailed(_) => RuntimeDispatchErrorKind::Manifest,
        WasmError::StoreConfiguration(_) => RuntimeDispatchErrorKind::Executor,
        WasmError::LinkerConfiguration(_) => RuntimeDispatchErrorKind::Executor,
        WasmError::InstantiationFailed(_) => RuntimeDispatchErrorKind::MethodMissing,
        WasmError::ExecutionFailed { .. } => RuntimeDispatchErrorKind::Guest,
        WasmError::InvalidSchema(_) => RuntimeDispatchErrorKind::Manifest,
    }
}
