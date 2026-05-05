# Reborn Capability Host Contract

`ironclaw_capabilities` is the caller-facing capability invocation service. It coordinates extension descriptor lookup, trust-aware authorization, approval resume, run-state transitions, optional obligation handling, dispatch, and process spawning without depending on concrete runtime crates.

## Obligation handling

Authorization may return `Decision::Allow { obligations }`. The capability host must fail closed unless either:

- the obligation list is empty; or
- a configured `CapabilityObligationHandler` accepts and satisfies every obligation.

The public handler seam is:

- `CapabilityObligationHandler::prepare(...)` before downstream side effects.
- `CapabilityObligationHandler::abort(...)` after prepare succeeded but dispatch/spawn failed.
- `CapabilityObligationHandler::complete_dispatch(...)` after successful inline dispatch but before output is returned.

Supported phases are:

- `Invoke` for inline capability dispatch.
- `Resume` for approved inline dispatch resume.
- `Spawn` for background process start.

Post-output obligations (`AuditAfter`, `RedactOutput`, `EnforceOutputLimit`) are invalid for `Spawn` and must fail before process start.

Prepared effects are explicit handoffs, not ambient state:

- `CapabilityObligationOutcome.mounts` narrows the effective mount view passed to dispatch/process start.
- `CapabilityObligationOutcome.resource_reservation` hands a prepared reservation to dispatch/process start.

If downstream dispatch/spawn fails after `prepare`, the capability host calls `abort` so handlers can release reservations or discard staged side effects.

## Failure taxonomy

- Unsupported obligations surface as `CapabilityInvocationError::UnsupportedObligations`.
- Handler failures surface as `CapabilityInvocationError::ObligationFailed` with a stable `CapabilityObligationFailureKind`.
- Run state records use stable error kinds such as `UnsupportedObligations` and `ObligationFailed`.

Capability host errors must not expose raw secrets, raw output, raw DB/provider errors, or raw host paths.
