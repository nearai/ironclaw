use std::{
    collections::HashMap,
    marker::PhantomData,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use futures_util::FutureExt;

use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{
    CapabilityDescriptor, InvocationOrigin, MountView, ResourceEstimate, ResourceReservation,
    UserId, runtime_policy::EffectiveRuntimePolicy,
};
use serde_json::Value;

use super::wasm_blocking::run_wasm_prepare_blocking;
use super::wasm_execution::{ReservationGuard, execute_prepared_wasm};
use super::{
    CapabilityId, DenyWasmHostHttp, DispatchError, ExtensionRuntime, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, InvocationServicesResolutionRequest, InvocationServicesResolver,
    McpError, McpExecutionRequest, McpExecutor, McpInvocation, NetworkObligationPolicyStore,
    PlannerError, PreparedWitTool, ResourceGovernor, ResourceReservationId, ResourceScope,
    RootFilesystem, RuntimeAdapterResult, RuntimeDispatchErrorKind, RuntimeKind, RuntimeLane,
    ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation, SharedRuntimeHttpEgress,
    WasmError, WasmRuntimeCredentialProvider, WasmRuntimeHttpAdapter, WasmRuntimePolicyDiscarder,
    WitToolHost, WitToolRuntime, WitToolRuntimeConfig, plan_capability, runtime_http_egress,
};
use crate::{
    FirstPartyCapabilityError,
    latency::{
        RuntimeLatencyFields, RuntimeLatencyMetrics, started_at as latency_started_at,
        trace_runtime_error, trace_runtime_ok,
    },
};

/// Per-invocation execution request handed to a runtime lane.
///
/// Host-internal seam behind the prebound
/// [`ironclaw_dispatcher::BoundCapabilityAdapter`] bindings: the registry-lane
/// resolver captures the static fields (package, descriptor, runtime policy,
/// filesystem, governor) when it constructs a binding and materializes one of
/// these per call. If `resource_reservation` is present, the lane must
/// reconcile or release that prepared reservation instead of creating a
/// second reservation.
pub(crate) struct RuntimeLaneRequest<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    pub package: &'a ExtensionPackage,
    pub descriptor: &'a CapabilityDescriptor,
    pub filesystem: &'a F,
    pub governor: &'a G,
    pub runtime_policy: &'a EffectiveRuntimePolicy,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    /// The authenticated human actor who initiated this invocation, distinct
    /// from the resource subject carried in `scope`. Threaded end-to-end
    /// (`CapabilityDispatchRequest` → here → `FirstPartyCapabilityRequest`) so a
    /// first-party handler can attribute the action to the acting user.
    pub authenticated_actor_user_id: Option<UserId>,
    /// Loop turn-run identity forwarded from the dispatch request. `None`
    /// for non-loop callers.
    pub run_id: Option<ironclaw_host_api::RunId>,
    /// Host-sealed origin used by capability-boundary policy.
    pub origin: Option<InvocationOrigin>,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
}

/// One runtime execution lane (Script/MCP/first-party/WASM).
///
/// Implementations must not perform caller-facing authorization or approval
/// resolution. They may reserve/reconcile resources through the provided
/// governor and must surface only redacted [`DispatchError`] categories.
#[async_trait]
pub(crate) trait RuntimeAdapter<F, G>: Send + Sync
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeLaneRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}

type FirstPartyLatencyFields = RuntimeLatencyFields;

fn first_party_latency_fields<F, G>(
    request: &RuntimeLaneRequest<'_, F, G>,
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
        request: RuntimeLaneRequest<'_, F, G>,
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
                dispatch_error_for_runtime(
                    request.descriptor.runtime,
                    planner_error_kind(&error),
                    Some(error.to_string()),
                )
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
                dispatch_error_for_runtime(
                    request.descriptor.runtime,
                    error.kind(),
                    Some(error.to_string()),
                )
            })?;

        self.inner.dispatch_json(request).await
    }
}

/// Closed host-runtime lane set. Capability bindings retain this one concrete
/// executor and a [`RuntimeLane`] value instead of retaining a trait object
/// selected from a `HashMap<RuntimeKind, dyn RuntimeAdapter>`.
pub(super) struct RuntimeLaneExecutor<F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    first_party: Option<FirstPartyRuntimeAdapter>,
    wasm: Option<ServiceResolvedRuntimeAdapter<WasmRuntimeAdapter>>,
    mcp: Option<ServiceResolvedRuntimeAdapter<McpRuntimeAdapter>>,
    process: Option<ServiceResolvedRuntimeAdapter<ScriptRuntimeAdapter>>,
    #[cfg(test)]
    test_adapters: HashMap<RuntimeLane, Arc<dyn RuntimeAdapter<F, G>>>,
    marker: PhantomData<fn() -> (F, G)>,
}

impl<F, G> RuntimeLaneExecutor<F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
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
            #[cfg(test)]
            test_adapters: HashMap::new(),
            marker: PhantomData,
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_adapter(
        mut self,
        lane: RuntimeLane,
        adapter: Arc<dyn RuntimeAdapter<F, G>>,
    ) -> Self {
        self.test_adapters.insert(lane, adapter);
        self
    }

    pub(super) fn supports_lane(&self, lane: RuntimeLane) -> bool {
        #[cfg(test)]
        if self.test_adapters.contains_key(&lane) {
            return true;
        }
        match lane {
            RuntimeLane::FirstParty => self.first_party.is_some(),
            RuntimeLane::Wasm => self.wasm.is_some(),
            RuntimeLane::Mcp => self.mcp.is_some(),
            RuntimeLane::Process => self.process.is_some(),
        }
    }

    pub(super) async fn dispatch_json(
        &self,
        lane: RuntimeLane,
        request: RuntimeLaneRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        #[cfg(test)]
        if let Some(adapter) = self.test_adapters.get(&lane) {
            return adapter.dispatch_json(request).await;
        }
        match lane {
            RuntimeLane::FirstParty => match self.first_party.as_ref() {
                Some(adapter) => adapter.dispatch_json(request).await,
                None => fail_unconfigured_lane(request),
            },
            RuntimeLane::Wasm => match self.wasm.as_ref() {
                Some(adapter) => adapter.dispatch_json(request).await,
                None => fail_unconfigured_lane(request),
            },
            RuntimeLane::Mcp => match self.mcp.as_ref() {
                Some(adapter) => adapter.dispatch_json(request).await,
                None => fail_unconfigured_lane(request),
            },
            RuntimeLane::Process => match self.process.as_ref() {
                Some(adapter) => adapter.dispatch_json(request).await,
                None => fail_unconfigured_lane(request),
            },
        }
    }
}

fn fail_unconfigured_lane<F, G>(
    request: RuntimeLaneRequest<'_, F, G>,
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
        request: RuntimeLaneRequest<'_, F, G>,
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
                model_visible_cause: Some(error.to_string()),
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
        request: RuntimeLaneRequest<'_, F, G>,
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
                    model_visible_cause: Some(error.to_string()),
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
        request: RuntimeLaneRequest<'_, F, G>,
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
                    safe_summary: Some(error.to_string()),
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
                    safe_summary: Some(error.to_string()),
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
            // The authenticated human actor (distinct from the resource subject
            // in `scope`), threaded through so a first-party handler can
            // attribute the action to the acting user.
            authenticated_actor_user_id: request.authenticated_actor_user_id.clone(),
            run_id: request.run_id,
            origin: request.origin,
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
        self.prepared.lock().map_err(|error| DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Executor,
            model_visible_cause: Some(error.to_string()),
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
        request: RuntimeLaneRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let module_path = match &request.package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => module
                .resolve_under(&request.package.root)
                .map_err(|error| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Manifest,
                    model_visible_cause: Some(error.to_string()),
                })?,
            other => {
                return Err(DispatchError::Wasm {
                    kind: if other.kind() == RuntimeKind::Wasm {
                        RuntimeDispatchErrorKind::Manifest
                    } else {
                        RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
                    },
                    model_visible_cause: None,
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
            .map_err(|error| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::FilesystemDenied,
                model_visible_cause: Some(error.to_string()),
            })?;
        let prepared = Arc::new(
            run_wasm_prepare_blocking(
                self.runtime.clone(),
                request.package.id.as_str().to_string(),
                wasm_bytes,
            )
            .await
            .map_err(|error| DispatchError::Wasm {
                kind: error.kind(),
                model_visible_cause: Some(error.source().to_string()),
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
    cause: Option<String>,
) -> DispatchError {
    match runtime {
        RuntimeKind::Mcp => DispatchError::Mcp {
            kind,
            model_visible_cause: cause,
        },
        RuntimeKind::Script => DispatchError::Script {
            kind,
            model_visible_cause: cause,
        },
        RuntimeKind::Wasm => DispatchError::Wasm {
            kind,
            model_visible_cause: cause,
        },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty {
            kind,
            safe_summary: cause,
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
        // A script timeout does not encode whether the request, runtime, or
        // backend caused the deadline to elapse. Keep it in the executor lane
        // until the runtime can provide typed timeout provenance; otherwise a
        // transient host slowdown would be mislabeled as a deterministic
        // model-visible operation failure and lose its retry path.
        ScriptError::Timeout { .. } => RuntimeDispatchErrorKind::Executor,
        ScriptError::InvalidOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
    }
}

fn mcp_error_kind(error: &McpError) -> RuntimeDispatchErrorKind {
    match error {
        McpError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        McpError::Client { .. } => RuntimeDispatchErrorKind::Client,
        McpError::InvalidToolCatalog { .. } => RuntimeDispatchErrorKind::OutputDecode,
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
