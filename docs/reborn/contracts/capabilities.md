# Reborn Capability Host Contract

`ironclaw_capabilities` is the caller-facing capability invocation service. It coordinates extension descriptor lookup, trust-aware authorization, approval resume, auth-gate resume, run-state transitions, optional obligation handling, dispatch, and process spawning without depending on concrete runtime crates.

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

## Auth-gate resume (`auth_resume_json`)

`CapabilityHost::auth_resume_json` re-dispatches a capability whose run record is in `BlockedAuth` status (i.e., the runtime previously returned an auth gate rather than a result).

**Preconditions:**

- The run record must exist and have status `BlockedAuth`.  Any other status returns `ResumeNotBlocked`.
- The `capability_id` on the request must match the one stored in the run record; a mismatch fails the run with `ResumeContextMismatch`.
- The execution context must pass `validate()`; failure returns `AuthorizationDenied { InternalInvariantViolation }`.

**Prior approval lease handling:**

When `approval_request_id` is `Some`, the invocation previously passed an approval gate.  The host locates the matching fingerprinted lease by first searching for an `Active` lease (first arrival), then for a `Claimed` lease (re-bounce after a prior auth attempt already claimed it).  The located lease is transitioned to `Dispatching` via `begin_dispatch_claimed` before authorization runs — this is a single-winner CAS: a second concurrent call sees a non-`Claimed` status and gets `InactiveLease`, mirroring the loser path of a concurrent `Active` `claim()`.

After authorization returns `Allow` and dispatch succeeds, the `Dispatching` lease is consumed via `consume`.  If dispatch fails but the error is a non-terminal `BlockedAuth` transition (the runtime hit another auth gate), the host calls `abort_dispatch_claimed` to revert the lease from `Dispatching` back to `Claimed`, so the next `auth_resume_json` call can reuse it without requiring a second human approval.  Any other failure revokes the lease.

When `approval_request_id` is `None`, no prior approval is in play and no lease step is performed; the path falls through to normal authorization and dispatch.

The obligations phase uses `CapabilityObligationPhase::Resume` (same as `resume_json`).

**Host-runtime integration:**

`HostRuntime::auth_resume_capability` is the composition-facing entry point.  The default trait implementation returns an explicit `Failed` outcome so stubs that do not override it fail loudly.  `DefaultHostRuntime` provides the production override: it evaluates runtime policy and trust, then routes to `CapabilityHost::auth_resume_json` via `CapabilityAuthResumeRequest`.

The agent-loop host constructs a `RuntimeCapabilityAuthResumeRequest` (carrying the original `invocation_id` encoded as a resume token and, when present, the prior `approval_request_id`) and calls `dispatch_runtime_capability_auth_resume`, which always uses `auth_resume_capability` — there is no spawn variant because sandbox spawns do not pass through auth gates.

## Failure taxonomy

- Unsupported obligations surface as `CapabilityInvocationError::UnsupportedObligations`.
- Handler failures surface as `CapabilityInvocationError::ObligationFailed` with a stable `CapabilityObligationFailureKind`.
- Run state records use stable error kinds such as `UnsupportedObligations` and `ObligationFailed`.

Capability host errors must not expose raw secrets, raw output, raw DB/provider errors, or raw host paths.
