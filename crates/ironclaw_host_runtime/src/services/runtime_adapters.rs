use std::{
    collections::HashMap,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use futures_util::FutureExt;

use super::wasm_execution::{ReservationGuard, execute_prepared_wasm, run_wasm_prepare_blocking};
use super::{
    CapabilityId, DenyWasmHostHttp, DispatchError, ExtensionRuntime, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, InvocationServicesResolutionRequest, InvocationServicesResolver,
    McpError, McpExecutionRequest, McpExecutor, McpInvocation, NetworkObligationPolicyStore,
    PlannerError, PreparedWitTool, ResourceGovernor, ResourceReservationId, ResourceScope,
    RootFilesystem, RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult,
    RuntimeDispatchErrorKind, RuntimeExecutor, RuntimeKind, RuntimeLane, ScriptError,
    ScriptExecutionRequest, ScriptExecutor, ScriptInvocation, SharedRuntimeHttpEgress, WasmError,
    WasmRuntimeCredentialProvider, WasmRuntimeHttpAdapter, WasmRuntimePolicyDiscarder, WitToolHost,
    WitToolRuntime, WitToolRuntimeConfig, plan_capability, runtime_http_egress,
};
use crate::{
    FirstPartyCapabilityError,
    latency::{
        RuntimeLatencyFields, RuntimeLatencyMetrics, started_at as latency_started_at,
        trace_runtime_error, trace_runtime_ok,
    },
};

type FirstPartyLatencyFields = RuntimeLatencyFields;

fn first_party_latency_fields<F, G>(
    request: &RuntimeAdapterRequest<'_, F, G>,
) -> Option<FirstPartyLatencyFields>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    RuntimeLatencyFields::from_json_input(
        request.capability_id,
        &request.scope,
        request.descriptor.runtime.as_str(),
        &request.input,
    )
}

fn trace_first_party_latency_ok(
    operation: &'static str,
    fields: Option<&FirstPartyLatencyFields>,
    started_at: Option<std::time::Instant>,
    output_bytes: u64,
    used_prepared_reservation: bool,
) {
    trace_runtime_ok(
        "first_party_runtime_adapter",
        operation,
        fields,
        started_at,
        RuntimeLatencyMetrics {
            output_bytes,
            used_prepared_reservation,
            ..RuntimeLatencyMetrics::default()
        },
    );
}

fn trace_first_party_latency_error(
    operation: &'static str,
    fields: Option<&FirstPartyLatencyFields>,
    started_at: Option<std::time::Instant>,
    error_kind: &str,
    used_prepared_reservation: bool,
) {
    trace_runtime_error(
        "first_party_runtime_adapter",
        operation,
        fields,
        started_at,
        error_kind,
        RuntimeLatencyMetrics {
            used_prepared_reservation,
            ..RuntimeLatencyMetrics::default()
        },
    );
}

fn trace_first_party_stage_and_dispatch_error(
    stage: &'static str,
    fields: Option<&FirstPartyLatencyFields>,
    stage_started_at: Option<std::time::Instant>,
    dispatch_started_at: Option<std::time::Instant>,
    error_kind: &str,
    used_prepared_reservation: bool,
) {
    trace_first_party_latency_error(
        stage,
        fields,
        stage_started_at,
        error_kind,
        used_prepared_reservation,
    );
    trace_first_party_latency_error(
        "dispatch",
        fields,
        dispatch_started_at,
        error_kind,
        used_prepared_reservation,
    );
}

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

/// Closed runtime-lane router: the host-runtime-side [`RuntimeExecutor`] the
/// dispatcher monomorphizes over (arch-simplification §4.2, `dyn`→enum collapse
/// of the capability hot path). Each configured lane is a field; adding a
/// [`RuntimeLane`] variant is a compile error until the two exhaustive matches
/// below handle it (the §4.2 safety property).
///
/// Lanes are `Option` because composition wires them conditionally on which
/// runtimes are configured — exactly as the prior per-lane registry populated
/// only the configured `RuntimeKind`s. The dispatcher checks
/// [`RuntimeExecutor::supports_lane`] before selecting a runtime, so an
/// unconfigured lane fails closed with `MissingRuntimeBackend` and no
/// reservation.
pub(super) struct RuntimeLaneExecutor {
    first_party: Option<FirstPartyRuntimeAdapter>,
    wasm: Option<ServiceResolvedRuntimeAdapter<WasmRuntimeAdapter>>,
    mcp: Option<ServiceResolvedRuntimeAdapter<McpRuntimeAdapter>>,
    process: Option<ServiceResolvedRuntimeAdapter<ScriptRuntimeAdapter>>,
}

impl RuntimeLaneExecutor {
    pub(super) fn new(
        first_party: Option<FirstPartyRuntimeAdapter>,
        wasm: Option<ServiceResolvedRuntimeAdapter<WasmRuntimeAdapter>>,
        mcp: Option<ServiceResolvedRuntimeAdapter<McpRuntimeAdapter>>,
        process: Option<ServiceResolvedRuntimeAdapter<ScriptRuntimeAdapter>>,
    ) -> Self {
        Self {
            first_party,
            wasm,
            mcp,
            process,
        }
    }
}

#[async_trait]
impl<F, G> RuntimeExecutor<F, G> for RuntimeLaneExecutor
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    fn supports_lane(&self, lane: RuntimeLane) -> bool {
        match lane {
            RuntimeLane::FirstParty => self.first_party.is_some(),
            RuntimeLane::Wasm => self.wasm.is_some(),
            RuntimeLane::Mcp => self.mcp.is_some(),
            RuntimeLane::Process => self.process.is_some(),
        }
    }

    // Hand-desugared (no `async fn`): the router RETURNS the lane adapter's
    // already-boxed `#[async_trait]` future instead of wrapping it in a future
    // of its own. The former `dyn RuntimeAdapter` registry contributed exactly
    // one boxed future here; an `async fn` router would stack a second one on
    // top of it, and the Reborn trace suite runs close enough to the 2 MiB
    // test-thread stack limit that the extra layer overflowed it in CI
    // (`reborn_trace_skill_management_first_party_tools_parity`). Zero-depth
    // forwarding restores exact structural parity with the dyn registry while
    // keeping the closed lane set.
    #[allow(clippy::type_complexity)]
    fn dispatch_json<'life0, 'life1, 'async_trait>(
        &'life0 self,
        lane: RuntimeLane,
        request: RuntimeAdapterRequest<'life1, F, G>,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<RuntimeAdapterResult, DispatchError>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        match lane {
            RuntimeLane::FirstParty => match self.first_party.as_ref() {
                Some(adapter) => adapter.dispatch_json(request),
                None => Box::pin(fail_unconfigured_lane(request)),
            },
            RuntimeLane::Wasm => match self.wasm.as_ref() {
                Some(adapter) => adapter.dispatch_json(request),
                None => Box::pin(fail_unconfigured_lane(request)),
            },
            RuntimeLane::Mcp => match self.mcp.as_ref() {
                Some(adapter) => adapter.dispatch_json(request),
                None => Box::pin(fail_unconfigured_lane(request)),
            },
            RuntimeLane::Process => match self.process.as_ref() {
                Some(adapter) => adapter.dispatch_json(request),
                None => Box::pin(fail_unconfigured_lane(request)),
            },
        }
    }
}

/// Fail closed for an unconfigured lane. The dispatcher gates dispatch with
/// `supports_lane`, so this arm is defensive; it still releases any prepared
/// reservation rather than leaking it.
async fn fail_unconfigured_lane<F, G>(
    request: RuntimeAdapterRequest<'_, F, G>,
) -> Result<RuntimeAdapterResult, DispatchError>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    release_adapter_reservation(
        request.governor,
        request
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id),
    );
    Err(DispatchError::MissingRuntimeBackend {
        runtime: request.descriptor.runtime,
    })
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
            display_preview: None,
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
            .map_err(|error| match error {
                McpError::AuthRequired {
                    required_secrets,
                    credential_requirements,
                } => DispatchError::AuthRequired {
                    capability: request.capability_id.clone(),
                    required_secrets,
                    credential_requirements,
                },
                error => DispatchError::Mcp {
                    kind: mcp_error_kind(&error),
                },
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            display_preview: None,
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
        let latency_fields = first_party_latency_fields(&request);
        let dispatch_started_at = latency_started_at();
        let used_prepared_reservation = request.resource_reservation.is_some();
        tracing::debug!("first-party runtime adapter dispatch started");
        let lookup_started_at = latency_started_at();
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
            trace_first_party_stage_and_dispatch_error(
                "lookup_handler",
                latency_fields.as_ref(),
                lookup_started_at,
                dispatch_started_at,
                RuntimeDispatchErrorKind::UndeclaredCapability.as_str(),
                used_prepared_reservation,
            );
            return Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::UndeclaredCapability,
                safe_summary: None,
                detail: None,
            });
        };
        trace_first_party_latency_ok(
            "lookup_handler",
            latency_fields.as_ref(),
            lookup_started_at,
            0,
            used_prepared_reservation,
        );

        let plan_started_at = latency_started_at();
        let plan =
            plan_capability(request.descriptor, request.runtime_policy).map_err(|error| {
                let kind = planner_error_kind(&error);
                tracing::debug!(
                    error_kind = %kind,
                    "first-party runtime adapter policy planning failed"
                );
                if let Some(reservation) = &request.resource_reservation {
                    release_first_party_reservation(request.governor, reservation.id);
                }
                trace_first_party_stage_and_dispatch_error(
                    "plan_capability",
                    latency_fields.as_ref(),
                    plan_started_at,
                    dispatch_started_at,
                    kind.as_str(),
                    used_prepared_reservation,
                );
                DispatchError::FirstParty {
                    kind,
                    safe_summary: None,
                    detail: None,
                }
            })?;
        trace_first_party_latency_ok(
            "plan_capability",
            latency_fields.as_ref(),
            plan_started_at,
            0,
            used_prepared_reservation,
        );
        tracing::debug!(
            filesystem_backend = ?plan.filesystem_backend,
            process_backend = ?plan.process_backend,
            network_mode = ?plan.network_mode,
            secret_mode = ?plan.secret_mode,
            "first-party runtime adapter policy plan resolved"
        );
        let resolve_started_at = latency_started_at();
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
                trace_first_party_stage_and_dispatch_error(
                    "resolve_services",
                    latency_fields.as_ref(),
                    resolve_started_at,
                    dispatch_started_at,
                    error.kind().as_str(),
                    used_prepared_reservation,
                );
                DispatchError::FirstParty {
                    kind: error.kind(),
                    safe_summary: None,
                    detail: None,
                }
            })?;
        trace_first_party_latency_ok(
            "resolve_services",
            latency_fields.as_ref(),
            resolve_started_at,
            0,
            used_prepared_reservation,
        );
        tracing::debug!("first-party runtime adapter services resolved");

        let reserve_started_at = latency_started_at();
        let reservation = match request.resource_reservation {
            Some(reservation) => {
                trace_first_party_latency_ok(
                    "reserve_resources",
                    latency_fields.as_ref(),
                    reserve_started_at,
                    0,
                    used_prepared_reservation,
                );
                reservation
            }
            None => request
                .governor
                .reserve(request.scope.clone(), request.estimate.clone())
                .inspect(|_| {
                    trace_first_party_latency_ok(
                        "reserve_resources",
                        latency_fields.as_ref(),
                        reserve_started_at,
                        0,
                        used_prepared_reservation,
                    );
                })
                .map_err(|_| {
                    tracing::debug!("first-party runtime adapter resource reservation failed");
                    trace_first_party_stage_and_dispatch_error(
                        "reserve_resources",
                        latency_fields.as_ref(),
                        reserve_started_at,
                        dispatch_started_at,
                        RuntimeDispatchErrorKind::Resource.as_str(),
                        used_prepared_reservation,
                    );
                    DispatchError::FirstParty {
                        kind: RuntimeDispatchErrorKind::Resource,
                        safe_summary: None,
                        detail: None,
                    }
                })?,
        };
        tracing::debug!(
            reservation_id = %reservation.id,
            used_prepared_reservation,
            "first-party runtime adapter resource reservation ready"
        );
        // From here the reservation lives in an RAII guard carried across the
        // handler `catch_unwind().await` below. If the turn scheduler drops this
        // future mid-await (cancel/lease-expiry/timeout), `Drop` releases the
        // reservation instead of leaking it permanently. Every early `return`
        // below drops the still-armed guard, which releases.
        let reservation_id = reservation.id;
        let guard = ReservationGuard::new(request.governor, reservation_id);
        let first_party_resource_error = || DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::Resource,
            safe_summary: None,
            detail: None,
        };

        tracing::debug!(
            reservation_id = %reservation_id,
            "first-party runtime adapter invoking handler"
        );
        let handler_started_at = latency_started_at();
        let result = match AssertUnwindSafe(handler.dispatch(FirstPartyCapabilityRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            authenticated_actor_user_id: request.authenticated_actor_user_id,
            run_id: request.run_id,
            estimate: request.estimate,
            mounts: request.mounts,
            services,
            input: request.input,
        }))
        .catch_unwind()
        .await
        {
            Ok(Ok(result)) => {
                trace_first_party_latency_ok(
                    "handler_dispatch",
                    latency_fields.as_ref(),
                    handler_started_at,
                    0,
                    used_prepared_reservation,
                );
                result
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    reservation_id = %reservation_id,
                    is_auth_required = error.is_auth_required(),
                    "first-party runtime adapter handler failed"
                );
                let error_kind = error
                    .kind()
                    .map(RuntimeDispatchErrorKind::as_str)
                    .unwrap_or("auth_required");
                trace_first_party_stage_and_dispatch_error(
                    "handler_dispatch",
                    latency_fields.as_ref(),
                    handler_started_at,
                    dispatch_started_at,
                    error_kind,
                    used_prepared_reservation,
                );
                if let Err(acct_err) =
                    guard.account_failed(error.usage(), first_party_resource_error)
                {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        error = ?acct_err,
                        "first-party resource accounting failed on handler error; \
                         returning original handler error"
                    );
                }
                return match error {
                    FirstPartyCapabilityError::AuthRequired {
                        required_secrets,
                        credential_requirements,
                        ..
                    } => Err(DispatchError::AuthRequired {
                        capability: request.capability_id.clone(),
                        required_secrets,
                        credential_requirements,
                    }),
                    FirstPartyCapabilityError::Dispatch {
                        kind,
                        safe_summary,
                        detail,
                        ..
                    } => Err(DispatchError::FirstParty {
                        kind,
                        safe_summary,
                        detail: detail.map(|detail| *detail),
                    }),
                };
            }
            Err(_) => {
                tracing::debug!(
                    reservation_id = %reservation_id,
                    "first-party runtime adapter handler panicked"
                );
                trace_first_party_stage_and_dispatch_error(
                    "handler_dispatch",
                    latency_fields.as_ref(),
                    handler_started_at,
                    dispatch_started_at,
                    RuntimeDispatchErrorKind::Backend.as_str(),
                    used_prepared_reservation,
                );
                // Dropping `guard` releases the reservation.
                return Err(DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Backend,
                    safe_summary: None,
                    detail: None,
                });
            }
        };

        let serialize_started_at = latency_started_at();
        let output_bytes = serde_json::to_vec(&result.output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|_| {
                tracing::debug!(
                    reservation_id = %reservation_id,
                    "first-party runtime adapter output serialization failed"
                );
                trace_first_party_stage_and_dispatch_error(
                    "serialize_output",
                    latency_fields.as_ref(),
                    serialize_started_at,
                    dispatch_started_at,
                    RuntimeDispatchErrorKind::OutputDecode.as_str(),
                    used_prepared_reservation,
                );
                // Dropping `guard` releases the reservation.
                DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::OutputDecode,
                    safe_summary: None,
                    detail: None,
                }
            })?;
        trace_first_party_latency_ok(
            "serialize_output",
            latency_fields.as_ref(),
            serialize_started_at,
            output_bytes,
            used_prepared_reservation,
        );
        let mut usage = result.usage;
        usage.output_bytes = usage.output_bytes.max(output_bytes);
        // Happy path: reconcile inline so we preserve the existing
        // warn-on-release-error-after-reconcile-failure diagnostic. `disarm`
        // hands reservation ownership back from the guard; both reconcile
        // outcomes settle the reservation below.
        let reconcile_id = guard.disarm();
        let reconcile_started_at = latency_started_at();
        let receipt = match request.governor.reconcile(reconcile_id, usage.clone()) {
            Ok(receipt) => {
                trace_first_party_latency_ok(
                    "reconcile_resources",
                    latency_fields.as_ref(),
                    reconcile_started_at,
                    output_bytes,
                    used_prepared_reservation,
                );
                receipt
            }
            Err(_) => {
                tracing::debug!(
                    reservation_id = %reconcile_id,
                    "first-party runtime adapter resource reconcile failed"
                );
                if let Err(release_error) = request.governor.release(reconcile_id) {
                    tracing::warn!(
                        reservation_id = %reconcile_id,
                        error = %release_error,
                        "failed to release first-party resource reservation after reconcile failure"
                    );
                }
                trace_first_party_stage_and_dispatch_error(
                    "reconcile_resources",
                    latency_fields.as_ref(),
                    reconcile_started_at,
                    dispatch_started_at,
                    RuntimeDispatchErrorKind::Resource.as_str(),
                    used_prepared_reservation,
                );
                return Err(DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Resource,
                    safe_summary: None,
                    detail: None,
                });
            }
        };
        tracing::debug!(
            reservation_id = %reconcile_id,
            output_bytes,
            "first-party runtime adapter dispatch completed"
        );
        trace_first_party_latency_ok(
            "dispatch",
            latency_fields.as_ref(),
            dispatch_started_at,
            output_bytes,
            used_prepared_reservation,
        );

        Ok(RuntimeAdapterResult {
            output: result.output,
            display_preview: result.display_preview,
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
            safe_summary: None,
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
                    safe_summary: None,
                })?,
            other => {
                return Err(DispatchError::Wasm {
                    kind: if other.kind() == RuntimeKind::Wasm {
                        RuntimeDispatchErrorKind::Manifest
                    } else {
                        RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
                    },
                    safe_summary: None,
                });
            }
        };
        let cache_key = module_path.as_str().to_string();
        let prepared = self.prepared_guard()?.get(&cache_key).cloned();
        if let Some(prepared) = prepared {
            let host = self.host_for_scope(&request.scope, request.capability_id);
            return execute_prepared_wasm(self.runtime.clone(), prepared, host, request).await;
        }

        let wasm_bytes = request
            .filesystem
            .read_file(&module_path)
            .await
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::FilesystemDenied,
                safe_summary: None,
            })?;
        let prepared = Arc::new(
            run_wasm_prepare_blocking(
                self.runtime.clone(),
                request.package.id.as_str().to_string(),
                wasm_bytes,
            )
            .await
            .map_err(|error| DispatchError::Wasm {
                kind: wasm_error_kind(&error),
                safe_summary: None,
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
        execute_prepared_wasm(self.runtime.clone(), prepared, host, request).await
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

fn release_first_party_reservation<G>(governor: &G, reservation_id: ResourceReservationId)
where
    G: ResourceGovernor + ?Sized,
{
    if let Err(error) = governor.release(reservation_id) {
        tracing::warn!(
            reservation_id = %reservation_id,
            error = %error,
            "failed to release prepared first-party reservation"
        );
    }
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

fn dispatch_error_for_runtime(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
) -> DispatchError {
    match runtime {
        RuntimeKind::Mcp => DispatchError::Mcp { kind },
        RuntimeKind::Script => DispatchError::Script { kind },
        RuntimeKind::Wasm => DispatchError::Wasm {
            kind,
            safe_summary: None,
        },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty {
            kind,
            safe_summary: None,
            detail: None,
        },
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
        McpError::AuthRequired { .. } => RuntimeDispatchErrorKind::Client,
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

pub(super) fn wasm_error_kind(error: &WasmError) -> RuntimeDispatchErrorKind {
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
