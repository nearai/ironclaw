# WS-9 — `LoopCapabilityPort` Wired to Host Runtime (with Profile-Scoped Surface)

**Workstream:** WS-9 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_turns` + `ironclaw_loop_support` +
`ironclaw_reborn` (`LoopCapabilityPort` already exists, but
`VisibleCapabilityRequest` grows the strategy filter field)
**Depends on:** WS-7 (`PlannedDriver` adapter), WS-8 (skeleton green)
**Parallel with:** WS-10..WS-15
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §11–§12

---

## 1. Scope

Today the skeleton composes `EmptyLoopCapabilityPort`
(in `crates/ironclaw_loop_support`) into the `AgentLoopDriverHost`
facade, so `host.visible_capabilities(...)` returns an empty surface
and every `invoke_capability(...)` is `Denied`. This is intentional —
`TextOnlyModelReplyDriver` never calls the capability port anyway.

WS-9 replaces that stub with a real `HostRuntimeLoopCapabilityPort`
that bridges to the existing `CapabilityHost`
(in `crates/ironclaw_capabilities/src/host.rs`), and adds a
`CapabilitySurfaceProfileFilter` decorator that gates both the
visible surface and the invocation path by the run profile's allowed
capability set.

End-state when WS-9 lands:

```text
PlannedDriver
  AgentLoopDriverHost
    LoopCapabilityPort  →  CapabilitySurfaceProfileFilter
                            └─ HostRuntimeLoopCapabilityPort
                                 └─ CapabilityHost (auth + approvals + audit)
                                      └─ CapabilityDispatcher
                                           └─ ExtensionRegistry / built-in tools
```

Crate ownership (per master doc §12 follow-up rule):
- **Decorator + host-runtime adapter** — `ironclaw_loop_support` (new
  files; the existing test fixture `HostRuntimeLoopCapabilityPort` in
  `crates/ironclaw_reborn/tests/loop_driver_host.rs` is the template).
- **Composition wiring** — `ironclaw_reborn` (composes the two in
  `planned_driver.rs` host setup).
- **Contract crate** — `ironclaw_turns` owns the wire filter:
  `VisibleCapabilityRequest { filter: VisibleCapabilityFilter }`.
  `ResolvedRunProfile.capability_surface_profile_id` still names the
  profile-owned surface; the strategy filter can only narrow it.

## 2. Files

### NEW
- `crates/ironclaw_loop_support/src/capability_port.rs` —
  `HostRuntimeLoopCapabilityPort`, real impl wrapping `CapabilityHost`
  (or a host-owned facade above it). This port must not call
  `CapabilityDispatcher` directly.
- `crates/ironclaw_loop_support/src/capability_surface_filter.rs` —
  `CapabilitySurfaceProfileFilter` decorator implementing
  `LoopCapabilityPort`. Owns the `CapabilityAllowSet` snapshot.
- `crates/ironclaw_loop_support/src/capability_allow_set.rs` —
  `CapabilityAllowSet` value type + `CapabilitySurfaceProfileResolver`
  trait that materializes the allowset from
  `CapabilitySurfaceProfileId`.

### MODIFIED
- `crates/ironclaw_loop_support/src/lib.rs` — module declarations and
  re-exports.
- `crates/ironclaw_reborn/src/planned_driver.rs` (lands in WS-7) —
  host composition takes a `Arc<dyn CapabilitySurfaceProfileResolver>`
  and a runtime `Arc<dyn CapabilityHost>` (or facade); wraps the two
  ports in the order shown in §1.

### NOT TOUCHED
- `crates/ironclaw_turns/**` must not depend on concrete
  `CapabilityHost`, dispatcher, or runtime allowset types. The only
  turns-side change is the neutral `VisibleCapabilityFilter` enum and
  request field.
- `crates/ironclaw_capabilities/**` — `CapabilityHost`'s own surface
  is unchanged; we consume it.
- `crates/ironclaw_reborn/tests/loop_driver_host.rs` — the test
  fixtures stay, but the real impl in
  `ironclaw_loop_support/src/capability_port.rs` is what the prod
  driver composes; tests can switch to importing it once they exist.

## 3. Specification

### 3.0 Contract request filter

```rust
//! crates/ironclaw_turns/src/run_profile/host.rs

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VisibleCapabilityRequest {
    pub filter: VisibleCapabilityFilter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibleCapabilityFilter {
    All,
    AllowOnly(Vec<CapabilityName>),
    Deny(Vec<CapabilityName>),
}

impl Default for VisibleCapabilityFilter {
    fn default() -> Self { Self::All }
}
```

This is the wire path for `CapabilityStrategy::filter`. Without this
field a non-default loop family can compute a narrower surface but the
executor has no way to send it to the host, so the model still sees the
full profile surface. Host-side filtering order is fixed:
`CapabilityHost` auth/grant/approval filtering →
`VisibleCapabilityFilter` (family can only narrow) →
`CapabilitySurfaceProfileFilter` allowset.

### 3.1 `CapabilityAllowSet` value type

A snapshot of which capabilities are visible for a given run. Built
once at run-claim time, frozen for the run (honors master doc §5
layer-1 immutability — the same rule that pins
`LoopRunContext.resolved_run_profile`).

```rust
//! crates/ironclaw_loop_support/src/capability_allow_set.rs

use std::collections::BTreeSet;
use ironclaw_turns::run_profile::{CapabilityId, CapabilitySurfaceProfileId, LoopRunContext};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityAllowSet {
    /// Surface profile allows every registered capability. Used by
    /// `text_only` runs (which can't actually call capabilities) and
    /// by trusted operator profiles.
    All,

    /// Surface profile allows only the listed capability ids. Order
    /// is irrelevant; BTreeSet for deterministic surface fingerprint.
    Allowlist(BTreeSet<CapabilityId>),
}

impl CapabilityAllowSet {
    pub fn permits(&self, id: &CapabilityId) -> bool {
        match self {
            Self::All => true,
            Self::Allowlist(set) => set.contains(id),
        }
    }
}

/// Materializes a `CapabilityAllowSet` from the run profile's
/// `CapabilitySurfaceProfileId`. The resolver is host-owned because
/// the mapping depends on host-side state (extension install state,
/// skill trust, user roles, deployment profile).
///
/// The resolver is called exactly once per claimed run — `PlannedDriver`
/// resolves at host-build time, freezes the resulting `Arc<CapabilityAllowSet>`
/// onto the filter decorator, and never re-evaluates. This matches the
/// master-doc §5 layer-1 rule: capability surface is immutable for the
/// run; recomputing per iteration would invalidate `surface_version`
/// pinning.
#[async_trait]
pub trait CapabilitySurfaceProfileResolver: Send + Sync {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError>;
}
```

`CapabilityResolveError` is a small `thiserror` enum with `Unavailable
/ Internal` variants; mapped to `AgentLoopHostError` at the filter
boundary via the same pattern as
`HostSkillContextBuildError::into_host_error`.

**Why allowset is host-side, not on `ResolvedRunProfile`:**
`ResolvedRunProfile.capability_surface_profile_id: CapabilitySurfaceProfileId`
is already the opaque, contract-crate-safe handle. Materializing it
into a concrete `BTreeSet<CapabilityId>` requires consulting the
extension registry, which lives below `ironclaw_turns`. Per that
crate's CLAUDE.md, the contract must not depend on `CapabilityHost`
or the extension registry. So resolution happens in
`ironclaw_loop_support`, and the resolved allowset lives on the
decorator, not in the contract.

### 3.2 `HostRuntimeLoopCapabilityPort`

```rust
//! crates/ironclaw_loop_support/src/capability_port.rs

use async_trait::async_trait;
use ironclaw_capabilities::CapabilityHost;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use std::sync::Arc;

pub struct HostRuntimeLoopCapabilityPort {
    capability_host: Arc<dyn CapabilityHost>,
}

#[async_trait]
impl LoopCapabilityPort for HostRuntimeLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        // CapabilityHost returns its own descriptor type after applying
        // host-owned authorization, approval, lease, audit, and obligation
        // boundaries. The adapter maps to CapabilityDescriptorView. Surface
        // version is the host-visible registration/grant fingerprint.
        Ok(self.capability_host.visible_surface(request).await?)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(self.capability_host.invoke_json(request).await?)
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        Ok(self.capability_host.invoke_batch(request).await?)
    }
}
```

The exact `CapabilityHost` method names are owned by
`ironclaw_capabilities` — implementer should match the current names
at PR time. Adapter only translates types and surfaces; no policy
logic. Direct dispatcher invocation from this port is forbidden because
the turns contract requires every model-triggered effect to pass through
`CapabilityHost` for grants, approvals, leases, audit, and obligations.

### 3.2a `EffectKind → ConcurrencyHint` mapping

`HostRuntimeLoopCapabilityPort::visible_capabilities` derives the
per-descriptor `concurrency_hint` from `CapabilityDescriptor.effects`
(in `ironclaw_capabilities::EffectKind` per
`crates/ironclaw_host_api/src/capability.rs:19-33`). The mapping
table — conservative by default — is enumerated below so the
derivation logic is unambiguous at implementation time. The general
rule is "anything that mutates external state or has causal ordering
implications → `Exclusive`; pure read with no side effects →
`SafeForParallel`."

| `EffectKind` | `ConcurrencyHint` | Rationale |
|---|---|---|
| `ReadFilesystem` | `SafeForParallel` | Pure read; OS handles concurrent reads safely |
| `WriteFilesystem` | `Exclusive` | Multiple writes to same path race |
| `DeleteFilesystem` | `Exclusive` | Mutates filesystem state |
| `Network` | `Exclusive` | Conservative: a POST or stateful protocol carries causal order; classifying every network call as parallel-safe risks subtle ordering bugs. Future refinement could split `NetworkRead` from `NetworkWrite`. |
| `UseSecret` | `SafeForParallel` | Read-only secret access; concurrent secret reads are safe (the secret store itself handles concurrency) |
| `ExecuteCode` | `Exclusive` | Process execution mutates external state by default |
| `SpawnProcess` | `Exclusive` | Spawning races on OS process tables and resource limits |
| `DispatchCapability` | `Exclusive` | Recursive call into another capability — depth-unknown; conservative classification prevents unbounded parallel fan-out |
| `ModifyExtension` | `Exclusive` | Mutates extension registry state |
| `ModifyApproval` | `Exclusive` | Mutates approval state |
| `ModifyBudget` | `Exclusive` | Mutates budget counters |
| `ExternalWrite` | `Exclusive` | Generic external-mutation effect |
| `Financial` | `Exclusive` | Financial side-effects must be sequential per audit |

Derivation logic:

```rust
fn concurrency_hint_from_effects(effects: &[EffectKind]) -> ConcurrencyHint {
    if effects.iter().any(|e| matches!(e,
        EffectKind::ReadFilesystem | EffectKind::UseSecret
    )) && !effects.iter().any(|e| !matches!(e,
        EffectKind::ReadFilesystem | EffectKind::UseSecret
    )) {
        // Only safe-for-parallel effects present
        ConcurrencyHint::SafeForParallel
    } else if effects.is_empty() {
        // No declared effects → pure function → safe
        ConcurrencyHint::SafeForParallel
    } else {
        // Any exclusive effect present → exclusive
        ConcurrencyHint::Exclusive
    }
}
```

A capability with **no** declared effects (`effects: vec![]`) is
treated as `SafeForParallel` — a pure function with no side effects
is genuinely parallel-safe. Capability authors should declare effects
honestly; a misdeclared "pure" function that secretly writes state is
an extension bug, not a hint-derivation bug.

If a future capability needs a hint that differs from the
effect-derived default (e.g., a network call that's specifically
declared parallel-safe by the author), the right path is to add an
optional explicit override field on `CapabilityDescriptor` in a
follow-up PR — but the skeleton derives conservatively from `effects`
alone.

### 3.3 `CapabilitySurfaceProfileFilter` decorator

```rust
//! crates/ironclaw_loop_support/src/capability_surface_filter.rs

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityDenied, CapabilityDeniedReasonKind, CapabilityInvocation,
    CapabilityOutcome, LoopCapabilityPort, LoopProgressPort,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use std::sync::Arc;
use crate::capability_allow_set::CapabilityAllowSet;

pub struct CapabilitySurfaceProfileFilter {
    inner: Arc<dyn LoopCapabilityPort>,
    allow_set: Arc<CapabilityAllowSet>,
    progress: Arc<dyn LoopProgressPort>,
}

#[async_trait]
impl LoopCapabilityPort for CapabilitySurfaceProfileFilter {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let filter = request.filter.clone();
        let mut surface = self.inner.visible_capabilities(request).await?;
        apply_strategy_filter(&mut surface, &filter);
        if let CapabilityAllowSet::Allowlist(_) = self.allow_set.as_ref() {
            surface.descriptors.retain(|d| self.allow_set.permits(&d.capability_id));
            // surface.version is NOT re-derived from the filtered list —
            // it stays whatever the inner port reported. See §3.4 for why.
        }
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if !self.allow_set.permits(&request.capability_id) {
            self.progress
                .record_capability_denied(request.capability_id.clone(), surface_profile_denied_kind())
                .await?;
            return Ok(CapabilityOutcome::Denied(CapabilityDenied {
                reason_kind: surface_profile_denied_kind(),
                safe_summary: "capability not in run-profile surface".into(),
            }));
        }
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        // Partition: allowed invocations go through; denied ones
        // synthesize a `Denied` outcome slot at the original index.
        //
        // The executor (master doc §8) zips calls→outcomes pairwise;
        // it does not require the inner port to see denied calls.
        //
        // `CapabilityOutcome` (host.rs:895–914) has no `Pending`
        // variant, so we cannot pre-seed forwarded slots with a
        // placeholder outcome. We track them as `None` and truncate
        // the outer vec to whatever prefix the inner port actually
        // populated. The contract crate already permits a short
        // `CapabilityBatchOutcome.outcomes` when `stop_on_first_suspension`
        // trips inside the inner port, so the executor handles
        // truncation natively.
        let mut slots: Vec<Option<CapabilityOutcome>> =
            Vec::with_capacity(request.invocations.len());
        let mut allowed = Vec::new();
        let mut allowed_idx = Vec::new();
        for (i, inv) in request.invocations.iter().enumerate() {
            if self.allow_set.permits(&inv.capability_id) {
                allowed.push(inv.clone());
                allowed_idx.push(i);
                slots.push(None);
            } else {
                slots.push(Some(CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind: surface_profile_denied_kind(),
                    safe_summary: "capability not in run-profile surface".into(),
                })));
            }
        }

        let (inner_outcomes, stopped_on_suspension) = if allowed.is_empty() {
            (Vec::new(), false)
        } else {
            let inner_batch = self.inner.invoke_capability_batch(
                CapabilityBatchInvocation {
                    invocations: allowed,
                    stop_on_first_suspension: request.stop_on_first_suspension,
                },
            ).await?;
            (inner_batch.outcomes, inner_batch.stopped_on_suspension)
        };

        // Inner returned `n_inner` outcomes for `allowed_idx.len()`
        // forwarded invocations, with `n_inner <= allowed_idx.len()`.
        // The first `n_inner` allowed slots get populated; the rest
        // are dropped along with any denial slots that lie strictly
        // beyond the last covered original index.
        let n_inner = inner_outcomes.len();
        for (outcome, &orig_idx) in inner_outcomes.into_iter().zip(&allowed_idx[..n_inner]) {
            slots[orig_idx] = Some(outcome);
        }

        // Truncate to the highest original index we actually covered.
        // If no inner outcomes were returned, the result is purely
        // the denial prefix (or empty when there were no denials
        // before any allowed call). When `n_inner == allowed_idx.len()`,
        // every slot is populated and the outer vec matches the
        // input length.
        let truncate_to: usize = if n_inner == allowed_idx.len() {
            slots.len()
        } else if n_inner == 0 {
            // Keep only denial slots that precede the first allowed
            // (which is `allowed_idx[0]` if any); denials at or
            // after that index are dropped because we never executed
            // the allowed call between them.
            allowed_idx.first().copied().unwrap_or(slots.len())
        } else {
            // Last covered original index is `allowed_idx[n_inner - 1]`;
            // include that slot and any denial slots preceding it.
            allowed_idx[n_inner - 1] + 1
        };
        slots.truncate(truncate_to);

        // Every retained slot is populated: denial slots were filled
        // at construction, and the inner-outcome loop covered all
        // allowed slots up through the truncation point.
        let outcomes = slots
            .into_iter()
            .map(|o| o.expect("retained slot must be populated"))
            .collect();

        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

fn surface_profile_denied_kind() -> CapabilityDeniedReasonKind {
    CapabilityDeniedReasonKind::unknown("surface_profile_denied")
        .expect("static literal validates")
}
```

### 3.4 `surface_version` semantics under the decorator

Master doc §9 locks `surface_version` per iteration. The filter does
**not** mint a new `CapabilitySurfaceVersion` after retaining a subset
— it returns the inner port's version verbatim. Reasons:

1. The allowset is frozen for the run (§3.1), so the filtered view's
   fingerprint can be expressed as `(inner_version, allow_set_hash)`
   but `allow_set_hash` is constant. The pair changes iff
   `inner_version` does.
2. Tying surface identity to the inner version means stale-surface
   detection (§9) still works: an extension install mid-run bumps the
   inner version, the filter still rejects post-bump descriptors that
   would otherwise be allowed, the executor reloads the surface, and
   the new filtered view is consistent.

If a future PR ever makes the allowset mutable mid-run (it will not
under §5 layer-1, but in case), the brief author should revisit this
section — the decorator would need to mint a derived version.

### 3.5 Batch invocation: `stopped_on_suspension`

The filter's `stopped_on_suspension` flag is computed by two rules,
not by inspecting whether denials occurred:

1. **At least one call forwarded to the inner port** — propagate
   `inner_batch.stopped_on_suspension` verbatim. A mixed batch with
   both denials *and* a real suspension at the inner port still
   surfaces the suspension signal; denial presence does not mask it.
   The truncation logic in §3.3 already drops any allowed/denial
   slots that lie beyond the suspension boundary, so the flag and
   the outcome length agree.
2. **Pure-denial batch (zero forwarded calls)** — set `false`.
   Denials are synchronous fast-fails, not suspensions. The executor
   (§8) consults `RecoveryStrategy` on `Denied`, which is the right
   ladder for this case.

Denial presence by itself never forces the flag to `false`. The
sketch in §3.3 implements both rules: the `if allowed.is_empty()`
branch yields `false`, and the populated branch threads
`inner_batch.stopped_on_suspension` through unchanged.

### 3.6 `CapabilityOutcome::Denied` mapping

`CapabilityOutcome::Denied(CapabilityDenied { reason_kind, ... })` is
defined in `crates/ironclaw_turns/src/run_profile/host.rs:912–944`.
The decorator uses the existing `Unknown(...)` reason kind seeded with
the constant string `surface_profile_denied`. The string sentinel is
temporary scaffolding, not the long-term contract.

The executor's denial-handling already exists in master doc §8:
`Denied(reason)` is treated as a non-recoverable failure for that
call; `RecoveryStrategy` may skip-and-continue or abort the batch.
Production-safe escape (§10) catches a tight denial loop via the
`recent_failure_kinds` no-progress detector — three denials of the
same call class in a row trip `NoProgressDetected` and exit cleanly.

- **Follow-up (committed, not deferred):** add a typed
  `CapabilityDeniedReasonKind::SurfaceProfileDenied` variant to the
  enum at `crates/ironclaw_turns/src/run_profile/host.rs:946` in a
  dedicated micro-PR against `ironclaw_turns`. WS-9's implementation
  PR MUST open that follow-up issue/PR concurrently with landing the
  string-sentinel code path — not after the fact, and not on a
  best-effort basis. The micro-PR is small (one variant + serde
  alias for `"surface_profile_denied"` so legacy persisted rows
  continue to deserialize). Downstream code (executor
  `RecoveryStrategy`, audit observers) must continue to handle
  `CapabilityDeniedReasonKind::Unknown(...)` for legacy rows that
  reference the string sentinel after migration, but all new code
  must match the typed `SurfaceProfileDenied` variant.

### 3.7 Audit + obligations

Audit (`ActionRecord`), approval gates, resource leases, obligations,
and process spawning all live below `CapabilityHost` and are exercised
by `HostRuntimeLoopCapabilityPort` invocation passthrough. Profile-filter
denials happen before `CapabilityHost`, so they do **not** create a tool
action record, but they still need durable redacted telemetry:
`CapabilitySurfaceProfileFilter` emits a WS-12 progress milestone
(`CapabilityDenied { reason: SurfaceProfileDenied, capability_id }`)
before returning `CapabilityOutcome::Denied`. Caller-level tests must
prove two facts together: denied/unapproved calls never reach the inner
dispatcher, and the denial/approval evidence is still recorded through
the host-owned telemetry path.

## 4. Composition in `PlannedDriver`

```rust
//! crates/ironclaw_reborn/src/planned_driver.rs (delta; WS-7 lands the rest)

pub struct PlannedDriverConfig {
    // ... existing fields ...
    pub capability_host: Arc<dyn CapabilityHost>,
    pub progress_port: Arc<dyn LoopProgressPort>,
    pub surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
}

impl PlannedDriver {
    async fn build_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let allow_set = Arc::new(
            self.config.surface_resolver.resolve(run_context).await
                .map_err(|e| AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    format!("surface resolver: {e}"),
                ))?
        );
        let inner = Arc::new(HostRuntimeLoopCapabilityPort::new(
            self.config.capability_host.clone(),
        )) as Arc<dyn LoopCapabilityPort>;
        Ok(Arc::new(CapabilitySurfaceProfileFilter::new(
            inner,
            allow_set,
            self.config.progress_port.clone(),
        )))
    }
}
```

`build_capability_port` is called once during `AgentLoopDriverHost`
facade construction (i.e. inside `PlannedDriver::run`/`resume` right
after the runner claim, before invoking the executor).

## 5. Verification

Unit tests (in `crates/ironclaw_loop_support`):
- `capability_allow_set::tests::all_permits_anything`.
- `capability_allow_set::tests::allowlist_permits_listed`.
- `capability_surface_filter::tests::visible_capabilities_filters_descriptors`
  — inner returns 5 descriptors, allowlist of 2, surface returns 2.
- `capability_surface_filter::tests::strategy_filter_narrows_visible_surface`
  — request filter `AllowOnly(["memory_read"])` plus profile allowset of
  three capabilities returns only `memory_read`.
- `capability_surface_filter::tests::invoke_denied_when_not_in_allowlist`
  — inner port spy receives zero invocations; outcome is `Denied`
  with `surface_profile_denied` reason.
- `capability_surface_filter::tests::profile_denial_records_redacted_progress`
  — denied call records the WS-12 denial milestone and still sends zero
  calls to the inner `CapabilityHost`.
- `host_runtime_capability_port::tests::unapproved_call_never_reaches_dispatcher`
  — host facade returns `ApprovalRequired`; dispatcher spy remains
  untouched while approval telemetry is recorded.
- `capability_surface_filter::tests::batch_partitions_correctly` —
  mixed batch: 2 allowed + 1 denied; inner receives only the 2
  allowed; final outcomes ordering matches the input ordering with
  the denied entry slotted at the original index.
- `capability_surface_filter::tests::partial_inner_outcomes_truncate_correctly`
  — 4-call batch at original indices `[0=allowed, 1=denied,
  2=allowed, 3=allowed]`; inner port returns only 2 outcomes for the
  3 forwarded allowed calls (simulating `stop_on_first_suspension`
  firing inside the inner port) with
  `inner_batch.stopped_on_suspension = true`. Filter output has
  length 3 (covering original indices 0..=2): the denial at index 1
  is retained, the index-3 slot is dropped, and the outer
  `stopped_on_suspension = true`.
- `capability_surface_filter::tests::surface_version_preserved` —
  filter does not mutate `surface.version`.

Integration tests (in `crates/ironclaw_reborn`, gated behind
`#[cfg(feature = "integration")]` or the existing
`ironclaw_agent_loop/test-support` feature from WS-8):
- `planned_driver_capability_e2e_happy_path` — model emits a
  capability call to an allowed tool; outcome flows back as
  `Completed`; loop continues.
- `planned_driver_capability_e2e_denied_then_recover` — model emits a
  call to a disallowed tool; outcome is `Denied`; default
  `RecoveryStrategy` skips-and-continues; loop reaches `Completed`.
- `planned_driver_capability_e2e_denial_loop_trips_no_progress` —
  model emits the same disallowed call three iterations in a row;
  loop exits `Failed { reason_kind: NoProgressDetected }` per master
  doc §10.

### 5.1 Composition-seam architecture test (PR #3523 follow-up)

Per the hooks-as-middleware design in master doc §9.1, this brief adds a lightweight architecture test in `ironclaw_loop_support` that proves no crate other than `ironclaw_loop_support` constructs `HostRuntimeLoopCapabilityPort` directly. That gives the future hooks composition seam (which sits as middleware around `HostRuntimeLoopCapabilityPort` in this same crate) a free composition-bypass check: any code path that bypasses the middleware by reaching the raw capability port outside `ironclaw_loop_support` is caught by the test.

```rust
// crates/ironclaw_loop_support/tests/host_capability_port_composition.rs (NEW)
//
// Grep-based architecture test: walks the workspace source tree (excluding
// this crate and `tests/`/`target/`) and asserts that no `.rs` file constructs
// `HostRuntimeLoopCapabilityPort::new(...)` or matches it as a literal type
// name. The constructor is `pub` (it has to be — `ironclaw_reborn` composes
// it through its host-factory function) but the convention is that ONLY
// `ironclaw_loop_support` constructs raw instances. Downstream crates wrap
// it through the host factory.
//
// When the hooks middleware lands (PR #3524 / #3523), this test gains a
// second pattern asserting hook composition wraps every raw construction.

#[test]
fn no_external_construction_of_host_runtime_capability_port() {
    let workspace_root = std::env::var("CARGO_WORKSPACE_DIR")
        .unwrap_or_else(|_| "..".to_string());
    let mut offenders = Vec::new();
    for entry in walkdir::WalkDir::new(&workspace_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().map_or(false, |x| x == "rs"))
    {
        let path = entry.path();
        // Skip this crate and any tests/ directories
        if path.components().any(|c| {
            c.as_os_str() == "ironclaw_loop_support"
                || c.as_os_str() == "tests"
                || c.as_os_str() == "target"
        }) {
            continue;
        }
        let src = std::fs::read_to_string(path).unwrap_or_default();
        if src.contains("HostRuntimeLoopCapabilityPort::new(")
            || src.contains("HostRuntimeLoopCapabilityPort {")
        {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "external construction of HostRuntimeLoopCapabilityPort detected: {offenders:?}\n\
         only ironclaw_loop_support's host factory should construct this port directly",
    );
}
```

The test is cheap to maintain — it adds one `walkdir` dev-dependency and one test file. If/when `ironclaw_reborn` adopts the hook middleware, the test extends to assert that every construction path also wraps through `HookedCapabilityPort` (defined in `ironclaw_hooks`).

## 6. Out of scope (for this brief)

- Typed `CapabilityDeniedReasonKind::SurfaceProfileDenied` variant —
  not landed in this brief's PR (uses
  `Unknown("surface_profile_denied")` string sentinel); the typed
  variant lands in the concurrent `ironclaw_turns` micro-PR
  committed in §3.6.
- Mid-run allowset mutation — explicitly disallowed by §5 layer-1
  immutability rule.
- Per-user / per-tenant tool gating below the profile level — the
  resolver implementation chooses how to derive allowset from profile
  + user state; the brief defines only the seam.
- The concrete `CapabilitySurfaceProfileResolver` impl — lands in
  `ironclaw_reborn` or a `src/`-side adapter PR; consults
  `ExtensionRegistry`, skill trust ceiling, user roles. Out of scope
  here because resolver policy is the actual product question and
  deserves its own design doc.
- `MCP`, `WASM`, `built-in` capability-runtime routing differences —
  all handled below `CapabilityDispatcher`; the loop-side filter is
  runtime-agnostic.
- Approval gates and capability leases — already implemented in
  `CapabilityHost`; this brief just respects their flow-through.
