use std::sync::Arc;

use serde::Deserialize;
use static_assertions::const_assert;
use tokio::sync::Semaphore;

use super::runtime_adapters::wasm_error_kind;
use super::wasm_diagnostics::{log_wasm_guest_error, log_wasm_runtime_error};
use super::{
    CapabilityId, DispatchError, PreparedWitTool, ResourceGovernor, ResourceReservationId,
    ResourceUsage, RootFilesystem, RuntimeAdapterRequest, RuntimeAdapterResult,
    RuntimeDispatchErrorKind, WasmError, WitToolExecution, WitToolHost, WitToolRequest,
    WitToolRuntime,
};
use ironclaw_host_api::ResourceReceipt;

/// Upper bound on WASM tool executions running concurrently inside
/// `spawn_blocking`.
///
/// Each WASM tool call runs a synchronous wasmtime guest call. We offload it to
/// the blocking thread pool (see [`execute_prepared_wasm`]) so it never parks a
/// tokio *worker* thread, but the blocking pool is itself finite. Without a
/// bound, a burst of concurrent turns (the incident: ~40 turns fanning out at
/// once) could occupy every blocking thread, starving every other
/// `spawn_blocking` user (DB, filesystem, etc.) and re-creating the runtime
/// wedge one layer down. This semaphore caps in-flight native WASM executions
/// well below the default blocking-pool ceiling (512) while staying far above
/// steady-state demand, so normal load never waits and a storm degrades to
/// queuing instead of pool exhaustion.
const MAX_CONCURRENT_WASM_EXEC: usize = 64;

// Enforce the bound invariant at compile time: it must be positive and stay
// below tokio's default blocking-pool ceiling (512) so native WASM execution
// can never monopolize the pool and starve other `spawn_blocking` users.
// `const_assert!` is evaluated at compile time and cannot panic at runtime.
const_assert!(MAX_CONCURRENT_WASM_EXEC > 0 && MAX_CONCURRENT_WASM_EXEC < 512);

/// Upper bound on WASM component compilations running concurrently inside
/// `spawn_blocking`.
///
/// Preparation (wasmtime `Component::new`) is capped at one quarter of the
/// execution bound so that a cold-compile storm cannot starve already-prepared
/// hot executions waiting on [`WASM_EXEC_SEMAPHORE`]. The two gates are
/// independent, so total blocking-pool usage is bounded by their sum
/// (16 + 64 = 80) — still far below tokio's default blocking-pool ceiling (512).
const MAX_CONCURRENT_WASM_PREPARE: usize = 16;

// Enforce the prepare bound at compile time: positive, smaller than the exec
// bound (so compiles cannot crowd out hot executions), and far below the pool
// ceiling. `const_assert!` is evaluated at compile time and cannot panic at runtime.
const_assert!(
    MAX_CONCURRENT_WASM_PREPARE > 0 && MAX_CONCURRENT_WASM_PREPARE < MAX_CONCURRENT_WASM_EXEC
);

/// Process-wide gate over concurrent native WASM execution. Shared across all
/// `WasmRuntimeAdapter` instances because they all draw from the same blocking
/// thread pool.
static WASM_EXEC_SEMAPHORE: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_WASM_EXEC)));

/// Process-wide gate over concurrent WASM component compilations. Independent
/// of [`WASM_EXEC_SEMAPHORE`] so a cold-compile storm cannot consume all
/// execution permits and starve already-prepared hot executions.
static WASM_PREPARE_SEMAPHORE: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_WASM_PREPARE)));

/// RAII guard over an in-flight `ResourceGovernor` reservation.
///
/// A reservation is added to the governor's per-scope `reserved_by_account`
/// tally by `reserve(...)` and is only subtracted by `reconcile(...)` or
/// `release(...)`. The governor has no TTL/sweep: a reservation that is never
/// reconciled or released leaks permanently, eventually exhausting the scope's
/// budget. Dispatch paths reserve *before* an `.await` (the `spawn_blocking`
/// join for WASM, the handler `catch_unwind` for first-party), so if the
/// surrounding future is dropped mid-`.await` — the turn scheduler cancels it on
/// user cancel, lease expiry, or a heartbeat-store timeout — any code after the
/// await never runs and the reservation leaks.
///
/// Holding this guard across the `.await` makes it part of the future's state.
/// Dropping the future runs [`Drop`], which releases the still-armed
/// reservation. On the normal paths the guard is settled explicitly via
/// [`reconcile`](Self::reconcile), [`account_failed`](Self::account_failed), or
/// [`disarm`](Self::disarm), which clears `armed` so `Drop` is a no-op and never
/// double-releases.
///
/// `governor.reconcile`/`governor.release` are synchronous (see
/// `ResourceGovernor` in `ironclaw_resources`), so calling `release` from `Drop`
/// is sound.
pub(super) struct ReservationGuard<'g, G: ResourceGovernor + ?Sized> {
    governor: &'g G,
    id: ResourceReservationId,
    armed: bool,
}

impl<'g, G: ResourceGovernor + ?Sized> ReservationGuard<'g, G> {
    pub(super) fn new(governor: &'g G, id: ResourceReservationId) -> Self {
        Self {
            governor,
            id,
            armed: true,
        }
    }

    /// Disarm the `Drop` safety net without settling, handing reservation
    /// ownership back to the caller. Used by the first-party happy path, which
    /// reconciles inline so it can preserve the warn-log on a
    /// release-after-reconcile-failure.
    pub(super) fn disarm(mut self) -> ResourceReservationId {
        self.armed = false;
        self.id
    }

    /// Happy path: reconcile actual usage, consuming the guard. On reconcile
    /// error, release the reservation and return the caller-supplied dispatch
    /// error. Settlement failures (reconcile or the fallback release) come from
    /// durable storage; we cannot surface their cause through the sanitized
    /// dispatch error, so they are logged as warnings keyed by `reservation_id`
    /// instead of being silently discarded.
    pub(super) fn reconcile(
        mut self,
        usage: ResourceUsage,
        on_reconcile_error: impl FnOnce() -> DispatchError,
    ) -> Result<ResourceReceipt, DispatchError> {
        self.armed = false;
        match self.governor.reconcile(self.id, usage) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                tracing::warn!(
                    reservation_id = %self.id,
                    error = %error,
                    "failed to reconcile resource reservation; releasing instead"
                );
                self.release_on_settlement_failure();
                Err(on_reconcile_error())
            }
        }
    }

    /// Failed execution: account partial usage if it has accountable effects,
    /// otherwise release. Mirrors `account_or_release_failed_*`: no usage or
    /// non-accountable usage releases (returning `Ok`); accountable usage
    /// reconciles, and a reconcile failure releases and returns the
    /// caller-supplied dispatch error. Settlement failures are logged (keyed by
    /// `reservation_id`) rather than silently dropped.
    pub(super) fn account_failed(
        mut self,
        usage: Option<&ResourceUsage>,
        on_reconcile_error: impl FnOnce() -> DispatchError,
    ) -> Result<(), DispatchError> {
        self.armed = false;
        match usage {
            Some(usage) if has_accountable_effects(usage) => {
                if let Err(error) = self.governor.reconcile(self.id, usage.clone()) {
                    tracing::warn!(
                        reservation_id = %self.id,
                        error = %error,
                        "failed to reconcile resource reservation for failed execution; releasing instead"
                    );
                    self.release_on_settlement_failure();
                    return Err(on_reconcile_error());
                }
                Ok(())
            }
            _ => {
                self.release_on_settlement_failure();
                Ok(())
            }
        }
    }

    /// Release the reservation, logging (rather than discarding) a durable-storage
    /// release failure keyed by `reservation_id`. Used by the explicit settlement
    /// paths; the `Drop` safety net logs separately so its message names the
    /// unsettled-on-drop case.
    fn release_on_settlement_failure(&self) {
        if let Err(error) = self.governor.release(self.id) {
            tracing::warn!(
                reservation_id = %self.id,
                error = %error,
                "failed to release resource reservation after settlement failure"
            );
        }
    }
}

impl<'g, G: ResourceGovernor + ?Sized> Drop for ReservationGuard<'g, G> {
    fn drop(&mut self) {
        if self.armed {
            // Cancellation / unexpected-return safety net: the reservation was
            // never settled, so release it to avoid a permanent budget leak.
            // `Drop` cannot return an error, so a durable-storage release failure
            // is surfaced as a sanitized warning keyed by `reservation_id` rather
            // than being silently discarded.
            if let Err(error) = self.governor.release(self.id) {
                tracing::warn!(
                    reservation_id = %self.id,
                    error = %error,
                    "failed to release unsettled resource reservation on drop"
                );
            }
        }
    }
}

pub(super) async fn execute_prepared_wasm<G>(
    runtime: WitToolRuntime,
    prepared: Arc<PreparedWitTool>,
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
    // Hold the reservation in an RAII guard from here on. The guard is carried
    // across the `run_wasm_execution_blocking().await` below, so if the turn
    // scheduler drops this future on cancel/lease-expiry/timeout, `Drop`
    // releases the reservation instead of leaking it permanently. Every early
    // `return` below drops the still-armed guard, which releases.
    let guard = ReservationGuard::new(request.governor, reservation.id);
    let wasm_resource_error = || DispatchError::Wasm {
        kind: RuntimeDispatchErrorKind::Resource,
    };
    let input_json = match serde_json::to_string(&request.input) {
        Ok(json) => json,
        Err(_) => {
            // Dropping `guard` releases the reservation.
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::InputEncode,
            });
        }
    };
    let context_json = wasm_invocation_context(request.capability_id);
    // The wasmtime guest call is synchronous and CPU/IO-bound. Running it
    // inline on the async worker would park that worker for the full duration
    // of WASM execution; a burst of concurrent turns (>= worker_count) could
    // then starve the scheduler, poller, and heartbeats — the runtime wedge.
    // Offload it to the blocking pool (mirroring legacy
    // `src/tools/wasm/wrapper.rs`), gated by a semaphore so a storm queues
    // instead of exhausting the blocking pool.
    let execution = match run_wasm_execution_blocking(
        runtime,
        prepared,
        host,
        input_json,
        context_json,
    )
    .await
    {
        Ok(execution) => execution,
        Err(error) => {
            log_wasm_runtime_error(request.capability_id, &error);
            // `preserved_wasm_error_usage` returns `Some` only for accountable
            // `ExecutionFailed` usage; `account_failed` then reconciles, and
            // otherwise releases — matching the prior reserve/account split.
            guard.account_failed(
                preserved_wasm_error_usage(&error).as_ref(),
                wasm_resource_error,
            )?;
            return Err(DispatchError::Wasm {
                kind: wasm_error_kind(&error),
            });
        }
    };
    if let Some(error) = execution.error {
        log_wasm_guest_error(request.capability_id, &execution.logs, &error);
        guard.account_failed(Some(&execution.usage), wasm_resource_error)?;
        return Err(wasm_guest_dispatch_error(&error, request.capability_id));
    }
    let Some(output_json) = execution.output_json else {
        guard.account_failed(Some(&execution.usage), wasm_resource_error)?;
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::InvalidResult,
        });
    };
    let output = match serde_json::from_str(&output_json) {
        Ok(output) => output,
        Err(_) => {
            guard.account_failed(Some(&execution.usage), wasm_resource_error)?;
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::OutputDecode,
            });
        }
    };
    let receipt = guard.reconcile(execution.usage.clone(), wasm_resource_error)?;
    Ok(RuntimeAdapterResult {
        output,
        display_preview: None,
        output_bytes: execution.usage.output_bytes,
        usage: execution.usage,
        receipt,
    })
}

/// Run the synchronous wasmtime guest call on the blocking thread pool.
///
/// The owned `runtime`/`prepared`/`host` are all cheap-to-move (`WitToolRuntime`
/// is a cheap `Clone` that shares its `Engine` by reference count; `prepared` is
/// an `Arc`; `WitToolHost` is `Clone`), so the closure is `Send + 'static`. A
/// semaphore permit is acquired here and then moved into the `spawn_blocking`
/// closure so its lifetime is tied to the blocking thread, not the outer async
/// future — cancellation of the caller does not release the slot early. A
/// `JoinError` (panic or cancellation of the blocking task) is surfaced as an
/// execution failure.
pub(super) async fn run_wasm_execution_blocking(
    runtime: WitToolRuntime,
    prepared: Arc<PreparedWitTool>,
    host: WitToolHost,
    input_json: String,
    context_json: String,
) -> Result<WitToolExecution, WasmError> {
    let permit = WASM_EXEC_SEMAPHORE
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| WasmError::execution_failed("wasm execution gate closed".to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.execute(
            &prepared,
            host,
            WitToolRequest::new(input_json).with_context(context_json),
        )
    })
    .await
    .map_err(|_| WasmError::execution_failed("wasm execution task panicked".to_string()))?
}

/// Run the synchronous wasmtime component compilation on the blocking thread pool.
///
/// `WitToolRuntime::prepare` performs `Component::new` (wasmtime compilation) plus
/// metadata extraction — CPU-heavy and blocking, exactly like guest execution.
/// Running it inline on the async worker would park that worker for the full
/// compile; a burst of cold-cache misses (cold start or cache eviction) could then
/// pin every async worker and re-create the runtime wedge that offloading
/// execution fixes. So `prepare` is offloaded to the blocking pool behind the
/// dedicated, smaller [`WASM_PREPARE_SEMAPHORE`] — separate from
/// [`WASM_EXEC_SEMAPHORE`] — so a cold-compile storm cannot starve
/// already-prepared hot executions that are waiting on their own gate. The owned
/// `runtime` is a cheap `Clone` (shared `Engine`) and `wasm_bytes` is moved in,
/// so the closure is `Send + 'static`. The semaphore permit is moved into the
/// `spawn_blocking` closure so its lifetime is tied to the blocking thread, not
/// the outer async future. A `JoinError` (panic or cancellation of the blocking
/// task) is surfaced as an execution failure.
pub(super) async fn run_wasm_prepare_blocking(
    runtime: WitToolRuntime,
    package_id: String,
    wasm_bytes: Vec<u8>,
) -> Result<PreparedWitTool, WasmError> {
    let permit = WASM_PREPARE_SEMAPHORE
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| WasmError::execution_failed("wasm preparation gate closed".to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.prepare(&package_id, &wasm_bytes)
    })
    .await
    .map_err(|_| WasmError::execution_failed("wasm preparation task panicked".to_string()))?
}

fn wasm_invocation_context(capability_id: &CapabilityId) -> String {
    serde_json::json!({
        "capability_id": capability_id.as_str(),
    })
    .to_string()
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

fn wasm_guest_dispatch_error(error: &str, capability: &CapabilityId) -> DispatchError {
    match wasm_guest_error_kind(error) {
        WasmGuestErrorKind::AuthRequired => DispatchError::AuthRequired {
            capability: capability.clone(),
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        },
        WasmGuestErrorKind::Runtime(kind) => DispatchError::Wasm { kind },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WasmGuestErrorKind {
    AuthRequired,
    Runtime(RuntimeDispatchErrorKind),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredWasmGuestErrorKind {
    AuthRequired,
    Input,
    OutputTooLarge,
    Executor,
    NetworkDenied,
    Client,
    OperationFailed,
}

#[derive(Debug, Deserialize)]
struct StructuredWasmGuestError {
    #[allow(dead_code)]
    code: String,
    kind: StructuredWasmGuestErrorKind,
}

fn wasm_guest_error_kind(error: &str) -> WasmGuestErrorKind {
    if let Ok(payload) = serde_json::from_str::<StructuredWasmGuestError>(error) {
        return match payload.kind {
            StructuredWasmGuestErrorKind::AuthRequired => WasmGuestErrorKind::AuthRequired,
            StructuredWasmGuestErrorKind::Input => {
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::InputEncode)
            }
            StructuredWasmGuestErrorKind::OutputTooLarge => {
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OutputTooLarge)
            }
            StructuredWasmGuestErrorKind::Executor => {
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Executor)
            }
            StructuredWasmGuestErrorKind::NetworkDenied => {
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied)
            }
            StructuredWasmGuestErrorKind::Client => {
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Client)
            }
            StructuredWasmGuestErrorKind::OperationFailed => {
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OperationFailed)
            }
        };
    }

    match error {
        "AuthRequired" => WasmGuestErrorKind::AuthRequired,
        "missing_invocation_context"
        | "invalid_invocation_context"
        | "unsupported_capability"
        | "invalid_parameters"
        | "invalid_repository"
        | "invalid_query_empty"
        | "invalid_query_too_large"
        | "invalid_author"
        | "invalid_assignee"
        | "invalid_involves"
        | "invalid_state"
        | "invalid_type"
        | "invalid_sort"
        | "invalid_order"
        | "invalid_page"
        | "invalid_limit"
        | "invalid_issue_number"
        | "invalid_body_empty"
        | "invalid_body_too_large" => {
            WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::InputEncode)
        }
        "host_http_body_limit" => {
            WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OutputTooLarge)
        }
        "host_http_timeout" => WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Executor),
        "host_http_network_denied" => {
            WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied)
        }
        "host_http_forbidden" | "host_http_rate_limited" => {
            WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Client)
        }
        _ => WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ironclaw_wasm::{WasmHostError, WasmHostHttp, WasmHttpRequest, WasmHttpResponse};
    use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
    use wit_parser::Resolve;

    use super::super::{ResourceScope, WitToolRuntimeConfig};
    use super::*;

    // ---------------------------------------------------------------------------
    // ReservationGuard unit tests
    //
    // These verify the RAII contract that fixes the permanent resource leak:
    //   • An armed guard dropped without settling releases exactly once.
    //   • An explicitly settled guard (reconcile / account_failed) does NOT
    //     release again on Drop.
    // ---------------------------------------------------------------------------

    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_host_api::{ReservationStatus, ResourceEstimate};
    use ironclaw_resources::{
        AccountSnapshot, ReservationOutcome, ResourceAccount, ResourceError, ResourceLimits,
    };

    /// Recording governor: counts `reconcile`/`release` calls and the last id
    /// each saw. `reserve*` are unused by the guard unit tests (the guard is
    /// constructed directly with a known id), so they return a stub error.
    #[derive(Debug, Default)]
    struct RecordingGovernor {
        reconcile_calls: AtomicUsize,
        release_calls: AtomicUsize,
    }

    impl RecordingGovernor {
        fn reconcile_calls(&self) -> usize {
            self.reconcile_calls.load(Ordering::SeqCst)
        }

        fn release_calls(&self) -> usize {
            self.release_calls.load(Ordering::SeqCst)
        }

        fn receipt(id: ResourceReservationId, status: ReservationStatus) -> ResourceReceipt {
            ResourceReceipt {
                id,
                scope: ResourceScope::system(),
                status,
                estimate: ResourceEstimate::default(),
                actual: None,
            }
        }
    }

    impl ResourceGovernor for RecordingGovernor {
        fn set_limit(
            &self,
            _account: ResourceAccount,
            _limits: ResourceLimits,
        ) -> Result<(), ResourceError> {
            Ok(())
        }

        fn reserve_with_outcome(
            &self,
            _scope: ResourceScope,
            _estimate: ResourceEstimate,
        ) -> Result<ReservationOutcome, ResourceError> {
            Err(ResourceError::Storage {
                reason: "reserve unused in guard unit tests".to_string(),
            })
        }

        fn reserve_with_id_and_outcome(
            &self,
            _scope: ResourceScope,
            _estimate: ResourceEstimate,
            _reservation_id: ResourceReservationId,
        ) -> Result<ReservationOutcome, ResourceError> {
            Err(ResourceError::Storage {
                reason: "reserve unused in guard unit tests".to_string(),
            })
        }

        fn reconcile(
            &self,
            reservation_id: ResourceReservationId,
            _actual: ResourceUsage,
        ) -> Result<ResourceReceipt, ResourceError> {
            self.reconcile_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Self::receipt(reservation_id, ReservationStatus::Reconciled))
        }

        fn release(
            &self,
            reservation_id: ResourceReservationId,
        ) -> Result<ResourceReceipt, ResourceError> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Self::receipt(reservation_id, ReservationStatus::Released))
        }

        fn account_snapshot(
            &self,
            _account: &ResourceAccount,
        ) -> Result<Option<AccountSnapshot>, ResourceError> {
            Ok(None)
        }
    }

    fn accountable_usage() -> ResourceUsage {
        ResourceUsage::default().set_output_bytes(32)
    }

    #[test]
    fn reservation_guard_drop_without_settling_releases_exactly_once() {
        let governor = RecordingGovernor::default();
        let id = ResourceReservationId::new();
        {
            let _guard = ReservationGuard::new(&governor, id);
            // Drop the still-armed guard at end of scope.
        }
        assert_eq!(
            governor.release_calls(),
            1,
            "an armed guard dropped without settling must release exactly once"
        );
        assert_eq!(
            governor.reconcile_calls(),
            0,
            "a dropped, unsettled guard must not reconcile"
        );
    }

    #[test]
    fn reservation_guard_reconcile_settles_and_drop_does_not_double_release() {
        let governor = RecordingGovernor::default();
        let id = ResourceReservationId::new();
        let guard = ReservationGuard::new(&governor, id);
        let receipt = guard
            .reconcile(accountable_usage(), || DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })
            .expect("reconcile must succeed");
        assert_eq!(receipt.status, ReservationStatus::Reconciled);
        // The guard was consumed by `reconcile`; no Drop release fires.
        assert_eq!(governor.reconcile_calls(), 1);
        assert_eq!(
            governor.release_calls(),
            0,
            "a reconciled guard must not release on Drop"
        );
    }

    #[test]
    fn reservation_guard_account_failed_accountable_reconciles_without_release() {
        let governor = RecordingGovernor::default();
        let id = ResourceReservationId::new();
        let guard = ReservationGuard::new(&governor, id);
        guard
            .account_failed(Some(&accountable_usage()), || DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })
            .expect("account_failed with accountable usage must reconcile");
        assert_eq!(governor.reconcile_calls(), 1);
        assert_eq!(
            governor.release_calls(),
            0,
            "accountable failed usage reconciles; it must not also release or double-release on Drop"
        );
    }

    #[test]
    fn reservation_guard_account_failed_non_accountable_releases_once() {
        let governor = RecordingGovernor::default();
        let id = ResourceReservationId::new();
        let guard = ReservationGuard::new(&governor, id);
        // No usage → release path; guard consumed, so Drop does not fire again.
        guard
            .account_failed(None, || DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })
            .expect("account_failed with no usage releases and returns Ok");
        assert_eq!(
            governor.release_calls(),
            1,
            "non-accountable failed usage must release exactly once"
        );
        assert_eq!(governor.reconcile_calls(), 0);
    }

    #[test]
    fn reservation_guard_disarm_suppresses_drop_release() {
        let governor = RecordingGovernor::default();
        let id = ResourceReservationId::new();
        let guard = ReservationGuard::new(&governor, id);
        let returned = guard.disarm();
        assert_eq!(returned, id, "disarm returns the reservation id");
        assert_eq!(
            governor.release_calls(),
            0,
            "a disarmed guard must not release on Drop — the caller owns settlement"
        );
        assert_eq!(governor.reconcile_calls(), 0);
    }

    // ---------------------------------------------------------------------------
    // WAT fixtures shared by caller-path WASM execution tests.
    // These are intentionally minimal — they only need to build a valid component
    // so the synchronous wasmtime guest call runs inside `spawn_blocking`.
    // ---------------------------------------------------------------------------

    /// Minimal WASM tool that returns "1" immediately, used to exercise the
    /// happy-path through `run_wasm_execution_blocking`.
    const SIMPLE_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 1024) "{\"type\":\"object\"}")
  (data (i32.const 2048) "test fixture")
  (data (i32.const 3072) "1")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 12
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    i32.const 3072
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

    /// Minimal WASM tool that makes one HTTP host call before returning "1".
    /// Used to inject a panicking HTTP host implementation so the panic occurs
    /// inside the `spawn_blocking` closure, exercising the JoinError→WasmError path.
    const HTTP_CALL_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 128) "POST")
  (data (i32.const 160) "https://example.test/api")
  (data (i32.const 224) "{}")
  (data (i32.const 256) "x")
  (data (i32.const 1024) "{\"type\":\"object\"}")
  (data (i32.const 2048) "test fixture")
  (data (i32.const 3072) "1")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 12
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    i32.const 128
    i32.const 4
    i32.const 160
    i32.const 24
    i32.const 224
    i32.const 2
    i32.const 1
    i32.const 256
    i32.const 1
    i32.const 0
    i32.const 0
    i32.const 512
    call $http_request

    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    i32.const 3072
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

    fn tool_component(wat_src: &str) -> Vec<u8> {
        let mut module = wat::parse_str(wat_src).expect("fixture WAT must parse");
        let mut resolve = Resolve::default();
        let package = resolve
            .push_str("tool.wit", include_str!("../../../../wit/tool.wit"))
            .expect("tool WIT must parse");
        let world = resolve
            .select_world(&[package], Some("sandboxed-tool"))
            .expect("sandboxed-tool world must exist");
        embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
            .expect("component metadata must embed");
        ComponentEncoder::default()
            .module(&module)
            .expect("fixture module must decode")
            .validate(true)
            .encode()
            .expect("component must encode")
    }

    // ---------------------------------------------------------------------------
    // Caller-path tests for `run_wasm_execution_blocking`
    // ---------------------------------------------------------------------------

    /// A WASM host HTTP implementation that panics unconditionally.
    ///
    /// Injected into the HTTP-calling fixture to make the blocking task panic,
    /// exercising the `JoinError → WasmError::ExecutionFailed` mapping introduced
    /// by this branch.
    #[derive(Debug)]
    struct PanickingHttp;

    impl WasmHostHttp for PanickingHttp {
        fn request(&self, _request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
            panic!("deliberate panic inside blocking task for test");
        }
    }

    // Serializes the semaphore-draining tests so they cannot interfere with
    // each other. Tests that drain `WASM_EXEC_SEMAPHORE` or `WASM_PREPARE_SEMAPHORE`
    // must hold this lock for the duration; concurrent execution would deadlock them.
    static SEMAPHORE_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    // 3a — Panicking blocking task maps to WasmError and releases the semaphore permit.
    //
    // Drives the real `run_wasm_execution_blocking` with a host HTTP impl that
    // panics, which causes the blocking task to panic. Asserts:
    //   • The call returns `Err(WasmError::ExecutionFailed { .. })` with the static
    //     message (i.e. the JoinError/panic payload is NOT leaked).
    //   • `WASM_EXEC_SEMAPHORE` returns to the same permit count after the call,
    //     confirming the permit is released even on the panic path.
    //     We pre-drain all-but-one permits to make the leak observable: if the
    //     panic leaked the permit, the second (successful) call would hang forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_wasm_execution_blocking_panic_maps_to_execution_failed_and_releases_permit() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;

        let runtime = Arc::new(WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap());
        let http_prepared = Arc::new(
            runtime
                .prepare("panic-http", &tool_component(HTTP_CALL_TOOL_WAT))
                .unwrap(),
        );
        let simple_prepared = Arc::new(
            runtime
                .prepare("simple", &tool_component(SIMPLE_TOOL_WAT))
                .unwrap(),
        );

        // Drain all-but-one permit so a leaked permit from the panicking call
        // would make the follow-up successful call hang on permit acquire.
        let mut held_permits = Vec::with_capacity(MAX_CONCURRENT_WASM_EXEC - 1);
        for _ in 0..MAX_CONCURRENT_WASM_EXEC - 1 {
            held_permits.push(
                WASM_EXEC_SEMAPHORE
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore must not be closed"),
            );
        }
        assert_eq!(WASM_EXEC_SEMAPHORE.available_permits(), 1);

        // --- panic call ---
        let panicking_host = WitToolHost::deny_all().with_http(Arc::new(PanickingHttp));
        let result = run_wasm_execution_blocking(
            (*runtime).clone(),
            Arc::clone(&http_prepared),
            panicking_host,
            "{}".to_string(),
            "{}".to_string(),
        )
        .await;

        // The JoinError from the panic must map to WasmError::ExecutionFailed
        // with the static message — NOT the panic payload.
        assert!(
            matches!(result, Err(WasmError::ExecutionFailed { .. })),
            "expected Err(WasmError::ExecutionFailed), got: {result:?}"
        );
        if let Err(WasmError::ExecutionFailed { message, .. }) = &result {
            assert_eq!(
                message, "wasm execution task panicked",
                "panic message must be the static string, not the JoinError payload"
            );
        }

        // Permit must be back: semaphore must still be at 1 (not 0).
        // If the panic leaked the permit, available_permits() would be 0 here,
        // and the successful call below would hang.
        assert_eq!(
            WASM_EXEC_SEMAPHORE.available_permits(),
            1,
            "semaphore permit must be released even when the blocking task panics"
        );

        // --- successful call with the returned permit ---
        let ok_result = tokio::time::timeout(
            Duration::from_secs(10),
            run_wasm_execution_blocking(
                (*runtime).clone(),
                Arc::clone(&simple_prepared),
                WitToolHost::deny_all(),
                "{}".to_string(),
                "{}".to_string(),
            ),
        )
        .await
        .expect("second call must not hang — permit was properly returned after the panic");

        assert!(
            ok_result.is_ok(),
            "second call must succeed after permit was returned: {ok_result:?}"
        );

        drop(held_permits);
    }

    // 3b — The real `run_wasm_execution_blocking` path is gated by the shared semaphore.
    //
    // Drains `WASM_EXEC_SEMAPHORE` to 0, confirms the call cannot proceed
    // within a short deadline (demonstrating it is blocked on the semaphore),
    // then releases all permits and verifies the call completes successfully.
    // Uses a deterministic barrier (a `tokio::spawn` task + `is_finished` poll
    // within a `time::timeout`) so there are no arbitrary sleeps.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_wasm_execution_blocking_is_gated_by_shared_semaphore() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;

        let runtime = Arc::new(WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap());
        let prepared = Arc::new(
            runtime
                .prepare("semaphore-gate", &tool_component(SIMPLE_TOOL_WAT))
                .unwrap(),
        );

        // Drain all permits. Collect them so they stay alive until we choose to
        // drop them.
        let mut held_permits = Vec::with_capacity(MAX_CONCURRENT_WASM_EXEC);
        for _ in 0..MAX_CONCURRENT_WASM_EXEC {
            let permit = WASM_EXEC_SEMAPHORE
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore must not be closed");
            held_permits.push(permit);
        }
        assert_eq!(
            WASM_EXEC_SEMAPHORE.available_permits(),
            0,
            "all permits must be drained before the backpressure assertion"
        );

        // Kick off the call — it should block inside `run_wasm_execution_blocking`
        // waiting to acquire the semaphore permit.
        let runtime_clone = (*runtime).clone();
        let prepared_clone = Arc::clone(&prepared);
        let call = tokio::spawn(async move {
            run_wasm_execution_blocking(
                runtime_clone,
                prepared_clone,
                WitToolHost::deny_all(),
                "{}".to_string(),
                "{}".to_string(),
            )
            .await
        });

        // Give the spawned task a moment to start and reach the semaphore acquire.
        // A short yield loop is sufficient — we confirm it has NOT finished yet.
        tokio::time::timeout(Duration::from_millis(80), async {
            loop {
                tokio::task::yield_now().await;
                if call.is_finished() {
                    break;
                }
            }
        })
        .await
        .expect_err("the blocking call must not complete while all semaphore permits are held");

        assert!(
            !call.is_finished(),
            "the call must still be queued while the semaphore is exhausted"
        );

        // Release all permits — the call should now be able to proceed.
        drop(held_permits);

        let result = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("call must complete after permits are released")
            .expect("task must not panic");

        assert!(
            result.is_ok(),
            "execution must succeed after the semaphore is released: {result:?}"
        );
    }

    // 3c — The real `run_wasm_prepare_blocking` path is gated by its own prepare semaphore.
    //
    // Mirrors `run_wasm_execution_blocking_is_gated_by_shared_semaphore` for the
    // compile/prepare offload: drains `WASM_PREPARE_SEMAPHORE` to 0, confirms the
    // prepare call cannot proceed within a short deadline (it is blocked on the
    // semaphore acquire, which happens before any compilation), then releases all
    // permits and verifies the call completes successfully. Uses a deterministic
    // barrier (a `tokio::spawn` task + `is_finished` poll within a `time::timeout`)
    // so there are no arbitrary sleeps.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_wasm_prepare_blocking_is_gated_by_prepare_semaphore() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;

        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
        let wasm_bytes = tool_component(SIMPLE_TOOL_WAT);

        // Drain all prepare permits. Collect them so they stay alive until we
        // choose to drop them.
        let mut held_permits = Vec::with_capacity(MAX_CONCURRENT_WASM_PREPARE);
        for _ in 0..MAX_CONCURRENT_WASM_PREPARE {
            let permit = WASM_PREPARE_SEMAPHORE
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore must not be closed");
            held_permits.push(permit);
        }
        assert_eq!(
            WASM_PREPARE_SEMAPHORE.available_permits(),
            0,
            "all prepare permits must be drained before the backpressure assertion"
        );

        // Kick off the prepare — it should block inside `run_wasm_prepare_blocking`
        // waiting to acquire the prepare semaphore permit (before any compilation).
        let runtime_clone = runtime.clone();
        let bytes_clone = wasm_bytes.clone();
        let call = tokio::spawn(async move {
            run_wasm_prepare_blocking(runtime_clone, "semaphore-gate".to_string(), bytes_clone)
                .await
        });

        // Give the spawned task a moment to start and reach the semaphore acquire.
        // A short yield loop is sufficient — we confirm it has NOT finished yet.
        tokio::time::timeout(Duration::from_millis(80), async {
            loop {
                tokio::task::yield_now().await;
                if call.is_finished() {
                    break;
                }
            }
        })
        .await
        .expect_err(
            "the prepare call must not complete while all prepare semaphore permits are held",
        );

        assert!(
            !call.is_finished(),
            "the prepare call must still be queued while the prepare semaphore is exhausted"
        );

        // Release all permits — the call should now be able to proceed.
        drop(held_permits);

        let result = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("prepare call must complete after permits are released")
            .expect("task must not panic");

        assert!(
            result.is_ok(),
            "prepare must succeed after the prepare semaphore is released: {result:?}"
        );
    }

    // 3c2 — Decoupling guarantee: a full `WASM_EXEC_SEMAPHORE` does NOT block prepare.
    //
    // Drains all `WASM_EXEC_SEMAPHORE` permits, then asserts that
    // `run_wasm_prepare_blocking` still completes promptly because it uses the
    // independent `WASM_PREPARE_SEMAPHORE`. This is the key invariant that prevents
    // a cold-compile storm from starving already-prepared hot executions: execution
    // and preparation compete on separate gates.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wasm_prepare_is_not_blocked_by_full_execution_semaphore() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;

        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
        let wasm_bytes = tool_component(SIMPLE_TOOL_WAT);

        // Drain ALL execution permits — simulates a burst that has saturated the
        // execution gate while a new cold-cache compile arrives.
        let mut held_permits = Vec::with_capacity(MAX_CONCURRENT_WASM_EXEC);
        for _ in 0..MAX_CONCURRENT_WASM_EXEC {
            let permit = WASM_EXEC_SEMAPHORE
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore must not be closed");
            held_permits.push(permit);
        }
        assert_eq!(
            WASM_EXEC_SEMAPHORE.available_permits(),
            0,
            "all execution permits must be drained before testing prepare independence"
        );

        // Prepare must complete promptly — it uses WASM_PREPARE_SEMAPHORE, not
        // WASM_EXEC_SEMAPHORE, so the exhausted exec gate must not block it.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_wasm_prepare_blocking(runtime, "decoupled-gate".to_string(), wasm_bytes),
        )
        .await
        .expect(
            "prepare must not block when the execution semaphore is full — gates are independent",
        );

        assert!(
            result.is_ok(),
            "prepare must succeed even with execution semaphore fully drained: {result:?}"
        );

        drop(held_permits);
    }

    // 3c3 — Decoupling guarantee (inverse): a full `WASM_PREPARE_SEMAPHORE` does NOT
    // block hot execution.
    //
    // Drains all `WASM_PREPARE_SEMAPHORE` permits, then asserts that
    // `run_wasm_execution_blocking` still completes promptly because it uses the
    // independent `WASM_EXEC_SEMAPHORE`. This is the symmetric complement of 3c2:
    // a cold-compile storm that has saturated the prepare gate must not starve
    // already-prepared hot executions waiting on their own gate.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wasm_execution_is_not_blocked_by_full_prepare_semaphore() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;

        let runtime = Arc::new(WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap());
        let prepared = Arc::new(
            runtime
                .prepare("decoupled-gate-exec", &tool_component(SIMPLE_TOOL_WAT))
                .unwrap(),
        );

        // Drain ALL prepare permits — simulates a cold-compile storm that has
        // saturated the prepare gate while a hot-execution call arrives.
        let mut held_permits = Vec::with_capacity(MAX_CONCURRENT_WASM_PREPARE);
        for _ in 0..MAX_CONCURRENT_WASM_PREPARE {
            let permit = WASM_PREPARE_SEMAPHORE
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore must not be closed");
            held_permits.push(permit);
        }
        assert_eq!(
            WASM_PREPARE_SEMAPHORE.available_permits(),
            0,
            "all prepare permits must be drained before testing execution independence"
        );

        // Execution must complete promptly — it uses WASM_EXEC_SEMAPHORE, not
        // WASM_PREPARE_SEMAPHORE, so the exhausted prepare gate must not block it.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_wasm_execution_blocking(
                (*runtime).clone(),
                Arc::clone(&prepared),
                WitToolHost::deny_all(),
                "{}".to_string(),
                "{}".to_string(),
            ),
        )
        .await
        .expect(
            "execution must not block when the prepare semaphore is full — gates are independent",
        );

        assert!(
            result.is_ok(),
            "execution must succeed even with prepare semaphore fully drained: {result:?}"
        );

        drop(held_permits);
    }

    // 3d — Invalid wasm bytes surface as a `WasmError` through the offloaded helper.
    //
    // Exercises the prepare error path through `run_wasm_prepare_blocking`: passing
    // bytes that are not a valid component makes `Component::new` fail, which the
    // runtime maps to `WasmError::CompilationFailed`. The offloaded helper must
    // propagate that error (not a JoinError-mapped `ExecutionFailed`), preserving
    // the `wasm_error_kind` mapping the dispatch path relies on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_wasm_prepare_blocking_invalid_bytes_maps_to_wasm_error() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;

        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
        let invalid_bytes = b"not a valid wasm component".to_vec();

        let result = run_wasm_prepare_blocking(runtime, "invalid".to_string(), invalid_bytes).await;

        assert!(
            matches!(result, Err(WasmError::CompilationFailed(_))),
            "invalid wasm bytes must surface as WasmError::CompilationFailed, got: {result:?}"
        );
        // Confirm the dispatch-path mapping is preserved end to end.
        if let Err(error) = &result {
            assert_eq!(
                wasm_error_kind(error),
                RuntimeDispatchErrorKind::Manifest,
                "compilation failure must map to the Manifest dispatch kind"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wasm_exec_semaphore_starts_open_at_configured_bound() {
        let _lock = SEMAPHORE_TEST_LOCK.lock().await;
        // The fix offloads sync WASM execution to the blocking pool under
        // `WASM_EXEC_SEMAPHORE`. The shared gate must start fully open at the
        // configured bound. (The bound's positivity / sub-ceiling invariant is
        // enforced at compile time next to the constant.)
        assert_eq!(
            WASM_EXEC_SEMAPHORE.available_permits(),
            MAX_CONCURRENT_WASM_EXEC,
            "the shared semaphore must start fully open at the configured bound"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wasm_exec_gate_bounds_concurrent_permits() {
        // A standalone gate sized like the production one must serialize
        // acquisitions beyond its bound rather than admitting all at once — the
        // property that prevents a turn burst from exhausting the pool.
        let gate = Arc::new(Semaphore::new(2));
        let a = gate.clone().acquire_owned().await.unwrap();
        let b = gate.clone().acquire_owned().await.unwrap();
        assert_eq!(gate.available_permits(), 0);
        assert!(
            gate.clone().try_acquire_owned().is_err(),
            "a third concurrent acquire must queue, not exceed the bound"
        );
        drop(a);
        assert!(gate.clone().try_acquire_owned().is_ok());
        drop(b);
    }

    #[test]
    fn wasm_guest_error_kind_maps_structured_payloads() {
        let cases = [
            (
                r#"{"code":"AuthRequired","kind":"auth_required"}"#,
                WasmGuestErrorKind::AuthRequired,
            ),
            (
                r#"{"code":"invalid_repository","kind":"input"}"#,
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            ),
            (
                r#"{"code":"host_http_body_limit","kind":"output_too_large"}"#,
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OutputTooLarge),
            ),
            (
                r#"{"code":"host_http_timeout","kind":"executor"}"#,
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Executor),
            ),
            (
                r#"{"code":"host_http_network_denied","kind":"network_denied"}"#,
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied),
            ),
            (
                r#"{"code":"host_http_forbidden","kind":"client"}"#,
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Client),
            ),
            (
                r#"{"code":"host_http_request_failed","kind":"operation_failed"}"#,
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(wasm_guest_error_kind(error), expected);
        }
    }

    #[test]
    fn wasm_guest_error_kind_preserves_legacy_error_mapping_without_prefix_catch_all() {
        let cases = [
            ("AuthRequired", WasmGuestErrorKind::AuthRequired),
            (
                "invalid_repository",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            ),
            (
                "missing_invocation_context",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            ),
            (
                "unsupported_capability",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            ),
            (
                "host_http_body_limit",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OutputTooLarge),
            ),
            (
                "host_http_timeout",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Executor),
            ),
            (
                "host_http_network_denied",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied),
            ),
            (
                "host_http_forbidden",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Client),
            ),
            (
                "host_http_rate_limited",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::Client),
            ),
            (
                "invalid_internal_state",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            ),
            (
                "unknown_error",
                WasmGuestErrorKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(wasm_guest_error_kind(error), expected);
        }
    }
}
