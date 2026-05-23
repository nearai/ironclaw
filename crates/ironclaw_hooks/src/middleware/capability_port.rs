//! Capability-port middleware that runs `dispatch_before_capability` ahead of
//! every invocation and translates hook decisions into the existing
//! `CapabilityOutcome` vocabulary.
//!
//! Translation:
//!
//! - `GateDecisionInner::Allow` → forward to inner port unchanged.
//! - `GateDecisionInner::Deny` → return `CapabilityOutcome::Denied` with
//!   `CapabilityDeniedReasonKind::Unknown("hook_denied")` and the sanitized
//!   reason as `safe_summary`.
//! - `GateDecisionInner::PauseApproval` → mint an approval gate ref via the
//!   configured [`HookGateRefFactory`] and return
//!   `CapabilityOutcome::ApprovalRequired { gate_ref, safe_summary }`.
//! - `GateDecisionInner::PauseAuth` → mint an auth gate ref via the factory
//!   and return `CapabilityOutcome::AuthRequired { gate_ref, safe_summary }`.
//!
//! If the factory itself fails (e.g. the host's gate-router rejected the
//! mint), the middleware fails closed and surfaces the call as
//! `CapabilityOutcome::Denied` with a sanitized `hook_gate_ref_unavailable`
//! reason kind — better to refuse the call than route the loop through an
//! unresolvable suspension.
//!
//! Failure cases from the dispatcher (panic, timeout, missing impl) also map
//! to `Denied` per the [`crate::failure_policy`] rules.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
    CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};

use crate::dispatch::{BeforeCapabilityDispatchOutcome, HookDispatcher};
use crate::kinds::gate::GateDecisionInner;
use crate::middleware::gate_ref::{FailClosedHookGateRefFactory, HookGateRefFactory};
use crate::middleware::resolver::{
    CapabilityInputResolver, CapabilityProviderResolver, NullCapabilityInputResolver,
    NullCapabilityProviderResolver,
};
use crate::points::{BeforeCapabilityHookContext, SanitizedArguments};

/// Maximum byte length of a capability input that the middleware will
/// hand to predicate evaluation. When [`CapabilityInputResolver::size_hint`]
/// reports a value larger than this, the middleware fails closed (treats
/// the input as unresolved) without calling
/// [`CapabilityInputResolver::resolve`]. A post-materialization check
/// against the serialized JSON length acts as a defense-in-depth backstop
/// when the size hint is unavailable.
///
/// This cap is deliberately conservative — its purpose is to prevent
/// accidental fatality (a multi-gigabyte file blob fed to a predicate
/// that scans for a numeric field) rather than to express a tight
/// production limit. Production deployments that need to evaluate
/// predicates against larger inputs should raise the cap once the
/// streaming-extraction story exists; today the predicate evaluator only
/// reads small numeric fields and 1 MiB is well above any realistic
/// `NumericSum` payload while being orders of magnitude below the cost
/// that would matter to a host.
pub const MAX_PREDICATE_INPUT_BYTES: u64 = 1024 * 1024;

/// Wraps an inner `LoopCapabilityPort`, fires `before_capability` hooks ahead
/// of each invocation, and translates the dispatcher's composed decision into
/// the `CapabilityOutcome` vocabulary the loop driver already speaks.
pub struct HookedLoopCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    dispatcher: Arc<HookDispatcher>,
    tenant_id: TenantId,
    resolver: Arc<dyn CapabilityInputResolver>,
    provider_resolver: Arc<dyn CapabilityProviderResolver>,
    gate_ref_factory: Arc<dyn HookGateRefFactory>,
}

impl HookedLoopCapabilityPort {
    /// Construct a middleware with the bundled
    /// [`NullCapabilityInputResolver`]. Predicate evaluators that depend on
    /// argument contents (e.g., `ValueOrRateBound::NumericSum`) will fail
    /// closed; use [`Self::with_resolver`] to wire in a production resolver.
    pub fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            inner,
            dispatcher,
            tenant_id,
            resolver: Arc::new(NullCapabilityInputResolver),
            provider_resolver: Arc::new(NullCapabilityProviderResolver),
            // Default to fail-closed: minting a syntactically-valid but
            // router-unregistered ref is worse than refusing the suspension.
            // Callers must explicitly opt into UuidHookGateRefFactory for
            // tests/dev, or install a router-backed factory for production
            // (henrypark133 review Critical #3).
            gate_ref_factory: Arc::new(FailClosedHookGateRefFactory),
        }
    }

    /// Override the resolver used to surface sanitized arguments to hook
    /// predicates. Returns `self` so callers can chain after `new`.
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn CapabilityInputResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Override the resolver used to populate
    /// [`crate::points::BeforeCapabilityHookContext::provider`] with the
    /// extension that owns the invoked capability. Required for
    /// `OwnCapabilities`-scoped Installed hooks to fire — without a
    /// production resolver the bundled [`NullCapabilityProviderResolver`]
    /// returns `None` and those hooks never see their own capabilities.
    #[must_use]
    pub fn with_provider_resolver(
        mut self,
        provider_resolver: Arc<dyn CapabilityProviderResolver>,
    ) -> Self {
        self.provider_resolver = provider_resolver;
        self
    }

    /// Override the gate-ref factory. Production code wires a factory that
    /// is bound to the current `LoopRunContext` and the host's approval-
    /// router so the resulting `ApprovalRequired` / `AuthRequired` outcomes
    /// resolve correctly. Tests and the foundation slice can rely on the
    /// default [`UuidHookGateRefFactory`].
    #[must_use]
    pub fn with_gate_ref_factory(mut self, factory: Arc<dyn HookGateRefFactory>) -> Self {
        self.gate_ref_factory = factory;
        self
    }

    async fn hook_context(
        &self,
        invocation: &CapabilityInvocation,
        provider: Option<ironclaw_host_api::ExtensionId>,
    ) -> BeforeCapabilityHookContext {
        // Lazy input resolution probe (PR #3573 follow-up): when no
        // active hook would actually read the capability arguments, we
        // skip both the size hint and the materializing `resolve` call.
        // Eager resolution was a HIGH-priority cost finding because file/
        // blob-shaped inputs can be expensive — or fatal — to materialize
        // even when no predicate needs them.
        let arguments = if self
            .dispatcher
            .before_capability_needs_input(provider.as_ref())
        {
            self.resolve_arguments(invocation).await
        } else {
            SanitizedArguments::unresolved()
        };
        BeforeCapabilityHookContext::new(
            self.tenant_id.clone(),
            invocation.capability_id.to_string(),
            invocation_arguments_digest(invocation),
            arguments,
            provider,
        )
    }

    /// Resolve capability arguments with a streaming size pre-check.
    ///
    /// Order of operations:
    ///
    /// 1. Ask the resolver for a [`CapabilityInputResolver::size_hint`].
    ///    If the hint is `Some(n) > MAX_PREDICATE_INPUT_BYTES`, return
    ///    `Unresolved` immediately — predicates that need input fail
    ///    closed via the evaluator's existing unresolved-path policy.
    /// 2. Call [`CapabilityInputResolver::resolve`]. If it returns
    ///    `None`, return `Unresolved`.
    /// 3. Re-check the serialized JSON length against
    ///    `MAX_PREDICATE_INPUT_BYTES`. This is a defense-in-depth
    ///    backstop for resolvers whose `size_hint` returns `None`
    ///    (default-impl, or sources that don't know the size up
    ///    front).
    async fn resolve_arguments(&self, invocation: &CapabilityInvocation) -> SanitizedArguments {
        if let Some(size) = self.resolver.size_hint(invocation).await
            && size > MAX_PREDICATE_INPUT_BYTES
        {
            tracing::debug!(
                capability = %invocation.capability_id,
                size_bytes = size,
                cap_bytes = MAX_PREDICATE_INPUT_BYTES,
                "capability input exceeds MAX_PREDICATE_INPUT_BYTES; failing closed before resolve"
            );
            return SanitizedArguments::unresolved();
        }
        let Some(value) = self.resolver.resolve(invocation).await else {
            return SanitizedArguments::unresolved();
        };
        // Defense-in-depth: even when the resolver's `size_hint`
        // returned `None`, refuse to expose payloads larger than the cap
        // to predicate evaluation. We measure the serialized byte cost
        // by streaming into a counting writer rather than calling
        // `serde_json::to_vec` and discarding the buffer — avoids one
        // `Vec<u8>` allocation per resolved invocation on the happy
        // path (henrypark133 review L1 on PR #3913). `SanitizedArguments::from_json`
        // sanitizes the in-memory `serde_json::Value` directly; it does
        // not re-serialize, so handing it the unmodified `value` is the
        // cheapest path.
        match serialized_len(&value) {
            Ok(bytes) if bytes > MAX_PREDICATE_INPUT_BYTES => {
                tracing::debug!(
                    capability = %invocation.capability_id,
                    size_bytes = bytes,
                    cap_bytes = MAX_PREDICATE_INPUT_BYTES,
                    "materialized capability input exceeds MAX_PREDICATE_INPUT_BYTES; failing closed"
                );
                SanitizedArguments::unresolved()
            }
            // Serialization failure means the resolver produced a value
            // we can't measure or surface safely; fail closed.
            Err(_) => SanitizedArguments::unresolved(),
            Ok(_) => SanitizedArguments::from_json(value),
        }
    }

    async fn run_dispatch(
        &self,
        invocation: &CapabilityInvocation,
        provider: Option<ironclaw_host_api::ExtensionId>,
    ) -> BeforeCapabilityDispatchOutcome {
        let ctx = self.hook_context(invocation, provider).await;
        self.dispatcher.dispatch_before_capability(&ctx).await
    }
}

#[async_trait]
impl LoopCapabilityPort for HookedLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        // Visible-surface queries don't go through hooks (the surface itself
        // is owned by profile-scoped filtering; hooks gate invocation, not
        // listing).
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let provider = self
            .provider_resolver
            .provider_for(&request.capability_id.to_string())
            .await;
        let outcome = self.run_dispatch(&request, provider.clone()).await;
        let result = match self.decision_to_outcome(&outcome).await {
            Some(translated) => Ok(translated),
            None => self.inner.invoke_capability(request).await,
        };
        // Fire AfterCapability observers regardless of whether the hook
        // short-circuited or the inner port ran. Observer-only point — no
        // gate decisions composed here. Telemetry must reflect both denied
        // and allowed invocations. The resolved provider is threaded so the
        // dispatcher can enforce `OwnCapabilities` scope on Installed
        // observers (serrrfirat finding #3).
        let _ = self
            .dispatcher
            .dispatch_observer_at_with_provider(
                crate::registry::HookPointSpec::AfterCapability,
                self.tenant_id.clone(),
                provider,
            )
            .await;
        result
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        // Two-phase batch dispatch that preserves the inner port's batch
        // semantics when hooks are active:
        //
        //   Phase 1 — preflight: walk invocations in order, run each through
        //   `BeforeCapability` hook dispatch, and translate restrictive
        //   decisions (deny / pause / fail-closed) into outcome slots
        //   immediately. Entries the hooks allow are queued for the inner
        //   port. If a hook-translated outcome is itself a suspension and
        //   `stop_on_first_suspension` is set, preflight stops there and the
        //   remaining invocations are dropped — mirroring the previous
        //   sequential semantics.
        //
        //   Phase 2 — inner batch: forward all queued (hook-allowed)
        //   invocations to the inner port as a SINGLE `invoke_capability_batch`
        //   call, then splice its outcomes back into their original index
        //   positions. The inner port may stop early on its own suspensions;
        //   any queued entry without a corresponding inner outcome is treated
        //   the same as a hook-suspension stop (dropped, no observer).
        //
        //   AfterCapability observers fire per resolved entry in the merged
        //   outcome vec, in original index order, matching the per-entry
        //   semantics established in PR #3573 (serrrfirat P2 #3).
        let CapabilityBatchInvocation {
            invocations,
            stop_on_first_suspension,
        } = request;

        // Phase 1: preflight hooks for each invocation in order.
        enum Slot {
            /// Hook produced a final outcome — no inner call needed.
            Resolved {
                outcome: CapabilityOutcome,
                provider: Option<ironclaw_host_api::ExtensionId>,
            },
            /// Hooks allowed; the inner port will produce the outcome.
            Pending {
                provider: Option<ironclaw_host_api::ExtensionId>,
            },
        }

        let mut slots: Vec<Slot> = Vec::with_capacity(invocations.len());
        let mut pending: Vec<CapabilityInvocation> = Vec::new();
        let mut stopped_in_preflight = false;
        for invocation in invocations {
            let provider = self
                .provider_resolver
                .provider_for(&invocation.capability_id.to_string())
                .await;
            let dispatch = self.run_dispatch(&invocation, provider.clone()).await;
            match self.decision_to_outcome(&dispatch).await {
                Some(translated) => {
                    let is_suspension = translated.is_suspension();
                    slots.push(Slot::Resolved {
                        outcome: translated,
                        provider,
                    });
                    if is_suspension && stop_on_first_suspension {
                        stopped_in_preflight = true;
                        break;
                    }
                }
                None => {
                    slots.push(Slot::Pending {
                        provider: provider.clone(),
                    });
                    pending.push(invocation);
                }
            }
        }

        // Phase 2: forward the surviving (hook-allowed) entries to the inner
        // port as a SINGLE batched call. Empty batches skip the inner call so
        // we don't perturb implementations that special-case empty input.
        let inner_result: Result<CapabilityBatchOutcome, AgentLoopHostError> = if pending.is_empty()
        {
            Ok(CapabilityBatchOutcome {
                outcomes: Vec::new(),
                stopped_on_suspension: false,
            })
        } else {
            self.inner
                .invoke_capability_batch(CapabilityBatchInvocation {
                    invocations: pending,
                    stop_on_first_suspension,
                })
                .await
        };

        // If the inner port errored, we still owe per-entry AfterCapability
        // observers for every slot we already produced an outcome for (Phase 1
        // resolved slots). Pending slots have no outcome to observe against;
        // matching the single-invocation path, we fire one observer per
        // pending slot so failed batch entries remain visible to telemetry
        // (serrrfirat P2 #3 on PR #3573).
        let inner_outcome = match inner_result {
            Ok(outcome) => outcome,
            Err(err) => {
                for slot in slots {
                    let provider = match slot {
                        Slot::Resolved { provider, .. } => provider,
                        Slot::Pending { provider } => provider,
                    };
                    let _ = self
                        .dispatcher
                        .dispatch_observer_at_with_provider(
                            crate::registry::HookPointSpec::AfterCapability,
                            self.tenant_id.clone(),
                            provider,
                        )
                        .await;
                }
                return Err(err);
            }
        };
        let CapabilityBatchOutcome {
            outcomes: mut inner_outcomes,
            stopped_on_suspension: inner_stopped,
        } = inner_outcome;
        // We pop from the front by reversing so we can take in original order.
        inner_outcomes.reverse();

        // Merge: walk slots in order, splicing inner outcomes into pending
        // slots. Dispatch AfterCapability observer per merged entry.
        //
        // Suspension handling preserves the per-entry observer contract
        // from PR #3573 (serrrfirat P2 #3): a hook-resolved suspension
        // slot that follows an allowed slot must still fire its
        // observer, and must still surface in `outcomes`, even when
        // `stop_on_first_suspension` is set. The pre-fix loop seeded
        // `stopped_on_suspension` from `stopped_in_preflight` and broke
        // on the first iteration, dropping any trailing Resolved
        // suspension slot — see the
        // `batch_invocation_fires_observer_for_hook_suspended_entry_after_allowed_entry_with_stop_on_first_suspension`
        // regression test (henrypark133 review M1 on PR #3911).
        //
        // Today the loop runs to completion when only Resolved
        // (hook-resolved) entries remain — every observer fires and
        // every outcome is pushed — and only breaks early when a
        // Pending slot has no inner outcome (inner port stopped on its
        // own suspension and consumed fewer outcomes than we queued).
        let mut outcomes = Vec::with_capacity(slots.len());
        let mut stopped_on_suspension = stopped_in_preflight;
        let mut pending_after_stop = false;
        for slot in slots {
            let outcome_and_provider = match slot {
                Slot::Resolved { outcome, provider } => Some((outcome, provider)),
                Slot::Pending { provider } => {
                    if pending_after_stop {
                        // We already stopped on a prior suspension and
                        // queued no work for the inner port past that
                        // point. A trailing Pending slot has no outcome
                        // to surface; drop it.
                        None
                    } else {
                        // `pop()` returns `None` when the inner port
                        // stopped early (its own suspension) and
                        // consumed fewer outcomes than we queued. Drop
                        // pending slots without an outcome and continue
                        // — observers on any trailing Resolved slots
                        // must still fire.
                        inner_outcomes.pop().map(|inner| (inner, provider))
                    }
                }
            };
            let Some((outcome, provider)) = outcome_and_provider else {
                continue;
            };
            let _ = self
                .dispatcher
                .dispatch_observer_at_with_provider(
                    crate::registry::HookPointSpec::AfterCapability,
                    self.tenant_id.clone(),
                    provider,
                )
                .await;
            if outcome.is_suspension() && stop_on_first_suspension {
                stopped_on_suspension = true;
                pending_after_stop = true;
            }
            outcomes.push(outcome);
        }
        if inner_stopped {
            stopped_on_suspension = true;
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

impl HookedLoopCapabilityPort {
    /// Translates a dispatcher outcome into a `CapabilityOutcome`. Returns
    /// `Some(outcome)` when the hook decision is restrictive (deny / pause /
    /// failure-closed), or `None` if the hooks allowed the call and the
    /// inner port should be consulted.
    ///
    /// This is async because pause-class decisions await the
    /// `HookGateRefFactory` to mint a real `LoopGateRef`. If the factory
    /// fails, the middleware falls back to `Denied` with a sanitized
    /// `hook_gate_ref_unavailable` reason.
    async fn decision_to_outcome(
        &self,
        dispatched: &BeforeCapabilityDispatchOutcome,
    ) -> Option<CapabilityOutcome> {
        match dispatched.decision.inner() {
            GateDecisionInner::Allow => None,
            GateDecisionInner::Deny { reason } => {
                Some(CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind: CapabilityDeniedReasonKind::unknown("hook_denied")
                        .expect("hook_denied is a valid loop-safe identifier"), // safety: literal ASCII identifier, validated by LoopGateRef constructor contract
                    safe_summary: reason.as_str().to_string(),
                }))
            }
            GateDecisionInner::PauseApproval { reason } => {
                match self
                    .gate_ref_factory
                    .mint_approval_ref(reason.as_str())
                    .await
                {
                    Ok(gate_ref) => Some(CapabilityOutcome::ApprovalRequired {
                        gate_ref,
                        safe_summary: reason.as_str().to_string(),
                    }),
                    Err(_) => Some(fail_closed_gate_ref_unavailable(reason.as_str())),
                }
            }
            GateDecisionInner::PauseAuth { reason } => {
                match self.gate_ref_factory.mint_auth_ref(reason.as_str()).await {
                    Ok(gate_ref) => Some(CapabilityOutcome::AuthRequired {
                        gate_ref,
                        safe_summary: reason.as_str().to_string(),
                    }),
                    Err(_) => Some(fail_closed_gate_ref_unavailable(reason.as_str())),
                }
            }
        }
    }
}

/// Fail-closed translation when the gate-ref factory cannot mint a ref for a
/// pause-class decision. The safe summary intentionally carries only the
/// hook's already-sanitized reason — the underlying host error is dropped to
/// avoid leaking internal gate-router state into model-visible output.
fn fail_closed_gate_ref_unavailable(sanitized_reason: &str) -> CapabilityOutcome {
    CapabilityOutcome::Denied(CapabilityDenied {
        reason_kind: CapabilityDeniedReasonKind::unknown("hook_gate_ref_unavailable")
            .expect("hook_gate_ref_unavailable is a valid loop-safe identifier"), // safety: literal ASCII identifier, validated by LoopGateRef constructor contract
        safe_summary: sanitized_reason.to_string(),
    })
}

/// Counts the JSON-serialized byte length of `value` without allocating
/// an intermediate `Vec<u8>`. `serde_json::to_writer` writes into a
/// trivial `std::io::Write` impl that only increments a counter, so the
/// happy-path measurement skips one buffer allocation and one
/// `Vec<u8>::drop` per resolved invocation (henrypark133 review L1 on
/// PR #3913).
fn serialized_len(value: &serde_json::Value) -> Result<u64, serde_json::Error> {
    struct CountingWriter(u64);
    impl std::io::Write for CountingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(buf.len() as u64);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = CountingWriter(0);
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.0)
}

/// Stable digest of capability invocation identity for hook context. The
/// middleware hashes the `(capability_id, input_ref)` pair so two
/// invocations with the same capability id and the same input ref produce
/// the same digest, enabling repetition / rate-cap logic without exposing
/// raw arguments to hook code. The digest is over input-ref identity, not
/// over the resolved argument content the input-ref points at — two
/// distinct refs that happen to resolve to identical JSON will NOT share a
/// digest, and the same ref representing changed underlying content will
/// keep the same digest.
///
/// # Stability contract
///
/// This digest is part of the **public hook contract**. Repetition-detection
/// hooks key on `BeforeCapabilityHookContext.arguments_digest` across
/// invocations; a shifted digest silently breaks them. Changing the hashing
/// structure (length-prefix ordering, hasher choice, which fields contribute)
/// requires:
///
/// 1. Updating the fixture in
///    `tests::invocation_arguments_digest_is_stable_for_known_inputs` with
///    the new captured hex.
/// 2. Surfacing the change in the cross-crate wire-format contract section
///    of `crate::identity` (the same section that pins `HookId::to_hex()`).
/// 3. Bumping the hook framework's contract version if downstream
///    consumers exist.
///
/// What this digest is NOT:
///
/// - **Not** a content digest of the resolved capability arguments. Hooks
///   that want to key on resolved content should use
///   `CapabilityInputResolver` + `SanitizedArguments`, not this digest.
/// - **Not** suitable as a primary key for cross-process deduplication —
///   two distinct invocations with the same `input_ref` (rare but legal)
///   produce the same digest.
fn invocation_arguments_digest(invocation: &CapabilityInvocation) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let cap = invocation.capability_id.to_string();
    hasher.update(&(cap.len() as u64).to_le_bytes());
    hasher.update(cap.as_bytes());
    // `as_str()` is the stable accessor for `CapabilityInputRef`. We avoid
    // `format!("{:?}", ...)` because `Debug` is not a stability contract —
    // a field rename or stdlib formatter change would silently shift the
    // digest, breaking any repetition-detection hook keyed on it.
    let input = invocation.input_ref.as_str();
    hasher.update(&(input.len() as u64).to_le_bytes());
    hasher.update(input.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
#[path = "tests/capability_port.rs"]
mod tests;
