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
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityCallCandidate,
    CapabilityDenied, CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome,
    LoopCapabilityPort, ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolDefinition,
    RegisterProviderToolCallRequest, VisibleCapabilityRequest, VisibleCapabilitySurface,
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

    pub(crate) async fn hook_context(
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
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(request).await
    }

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
                outcome: Box<CapabilityOutcome>,
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
                        outcome: Box::new(translated),
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
                Slot::Resolved { outcome, provider } => Some((*outcome, provider)),
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
                        approval_resume: None,
                    }),
                    Err(_) => Some(fail_closed_gate_ref_unavailable(reason.as_str())),
                }
            }
            GateDecisionInner::PauseAuth { reason } => {
                match self.gate_ref_factory.mint_auth_ref(reason.as_str()).await {
                    Ok(gate_ref) => Some(CapabilityOutcome::AuthRequired {
                        gate_ref,
                        credential_requirements: Vec::new(),
                        safe_summary: reason.as_str().to_string(),
                        auth_resume: None,
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
mod tests {
    use super::*;
    use crate::dispatch::BeforeCapabilityHookImpl;
    use crate::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
    use crate::middleware::gate_ref::UuidHookGateRefFactory;
    use crate::ordering::HookPhase;
    use crate::ordering::HookPriority;
    use crate::registry::{HookBinding, HookBindingScope, HookPointSpec, HookRegistry};
    use crate::sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink};
    use crate::trust::HookTrustClass;
    use async_trait::async_trait;
    use ironclaw_host_api::{CapabilityId, RuntimeKind};
    use ironclaw_turns::LoopResultRef;
    use ironclaw_turns::run_profile::{
        CapabilityDescriptorView, CapabilityInputRef, CapabilityResultMessage,
        CapabilitySurfaceVersion,
    };
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    struct AlwaysCompletedPort {
        calls: Mutex<Vec<CapabilityId>>,
        /// Number of times `invoke_capability_batch` was called on the inner
        /// port. Distinct from `calls.len()`, which counts the per-entry
        /// `invoke_capability` invocations the batch impl makes underneath.
        batch_calls: Mutex<Vec<Vec<CapabilityId>>>,
    }

    impl AlwaysCompletedPort {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                batch_calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<CapabilityId> {
            self.calls.lock().expect("not poisoned").clone()
        }

        fn batch_calls(&self) -> Vec<Vec<CapabilityId>> {
            self.batch_calls.lock().expect("not poisoned").clone()
        }
    }

    #[async_trait]
    impl LoopCapabilityPort for AlwaysCompletedPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                callable_capability_ids: None,
                version: CapabilitySurfaceVersion::new("v1").expect("ok"),
                descriptors: vec![CapabilityDescriptorView {
                    capability_id: CapabilityId::new("cap.x").expect("ok"),
                    provider: None,
                    runtime: RuntimeKind::Wasm,
                    safe_name: "cap.x".to_string(),
                    safe_description: "test capability".to_string(),
                    concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::Exclusive,
                    parameters_schema: serde_json::Value::Null,
                }],
            })
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.calls
                .lock()
                .expect("not poisoned")
                .push(request.capability_id.clone());
            Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new(format!("result:{}", request.capability_id))
                    .expect("ok"),
                safe_summary: format!("ran {}", request.capability_id),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: 0,
                output_digest: None,
                model_observation: None,
            }))
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            self.batch_calls.lock().expect("not poisoned").push(
                request
                    .invocations
                    .iter()
                    .map(|i| i.capability_id.clone())
                    .collect(),
            );
            let mut outcomes = Vec::with_capacity(request.invocations.len());
            for invocation in request.invocations {
                outcomes.push(self.invoke_capability(invocation).await?);
            }
            Ok(CapabilityBatchOutcome {
                outcomes,
                stopped_on_suspension: false,
            })
        }
    }

    struct DenyingHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for DenyingHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.deny("blocked by extension policy");
        }
    }

    struct PauseApprovalHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for PauseApprovalHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.pause_approval("needs approval for this capability");
        }
    }

    struct PauseAuthHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for PauseAuthHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.pause_auth("needs auth for this capability");
        }
    }

    /// Records the digest a real `BeforeCapability` hook receives, then passes.
    #[derive(Clone)]
    struct CapturingBeforeCapabilityHook {
        captured: Arc<Mutex<Vec<[u8; 32]>>>,
    }

    impl CapturingBeforeCapabilityHook {
        fn new() -> Self {
            Self {
                captured: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn captured(&self) -> Vec<[u8; 32]> {
            self.captured.lock().expect("not poisoned").clone()
        }
    }

    #[async_trait]
    impl RestrictedBeforeCapabilityHook for CapturingBeforeCapabilityHook {
        async fn evaluate(
            &self,
            ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            self.captured
                .lock()
                .expect("not poisoned")
                .push(ctx.arguments_digest);
            // Restricted hooks must make an explicit decision; no-op would fail closed.
            sink.pass();
        }
    }

    fn dispatcher_with_restricted_hook(
        local: &str,
        hook: Box<dyn RestrictedBeforeCapabilityHook>,
    ) -> (Arc<HookDispatcher>, HookId) {
        let hook_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new(local).expect("valid HookLocalId in test"),
            HookVersion::ONE,
        );
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            event_kind_filter: None,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        let mut registry = HookRegistry::new();
        registry.insert(binding).expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(hook_id, BeforeCapabilityHookImpl::Restricted(hook));
        (Arc::new(dispatcher), hook_id)
    }

    /// Test-only gate-ref factory that always errors. Used to exercise the
    /// fail-closed path when the host's gate-router refuses to mint a ref.
    struct FailingGateRefFactory;
    #[async_trait]
    impl crate::middleware::gate_ref::HookGateRefFactory for FailingGateRefFactory {
        async fn mint_approval_ref(
            &self,
            _reason: &str,
        ) -> Result<ironclaw_turns::LoopGateRef, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
                "no router",
            ))
        }
        async fn mint_auth_ref(
            &self,
            _reason: &str,
        ) -> Result<ironclaw_turns::LoopGateRef, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
                "no router",
            ))
        }
    }

    // Canonical digest fixture shared by helper-level and caller-driven pins.
    // If the hex changes, audit every caller that keys on `arguments_digest`.
    const SNAPSHOT_FIXTURE_DIGEST_HEX: &str =
        "4d0ab78e009b32615c2766bd1c26921bd59ef81b5741a75387707f82f0344315";

    fn snapshot_fixture_invocation() -> CapabilityInvocation {
        CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion::new(
                "snapshot:v1",
            )
            .expect("surface version literal is valid"),
            capability_id: CapabilityId::new("cap.snapshot.fixture")
                .expect("capability id literal is valid"),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:cap.snapshot.fixture",
            )
            .expect("input ref literal is valid"),
            approval_resume: None,
            auth_resume: None,
        }
    }

    fn digest_hex(digest: &[u8; 32]) -> String {
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn dispatcher_with_capturing_hook(hook: CapturingBeforeCapabilityHook) -> Arc<HookDispatcher> {
        let (dispatcher, _) = dispatcher_with_restricted_hook("capture-digest", Box::new(hook));
        dispatcher
    }

    fn assert_hook_observed_snapshot_digest(hook: &CapturingBeforeCapabilityHook, path: &str) {
        let captured = hook.captured();
        assert_eq!(
            captured.len(),
            1,
            "hook must observe exactly one BeforeCapability context"
        );
        assert_eq!(
            digest_hex(&captured[0]),
            SNAPSHOT_FIXTURE_DIGEST_HEX,
            "arguments_digest observed through {path} shifted; this is a \
             hook-visible wire-contract break"
        );
    }

    /// Snapshot regression: pins `invocation_arguments_digest` for a known
    /// `(capability_id, input_ref)` pair. If this assertion fails, the
    /// digest's hashing structure changed — see the stability contract on
    /// `invocation_arguments_digest`. **Do not update the expected hex
    /// without auditing every caller that keys on `arguments_digest`.**
    #[test]
    fn invocation_arguments_digest_is_stable_for_known_inputs() {
        let invocation = CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion::new(
                "snapshot:v1",
            )
            .expect("surface version literal is valid"),
            capability_id: CapabilityId::new("cap.snapshot.fixture")
                .expect("capability id literal is valid"),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:cap.snapshot.fixture",
            )
            .expect("input ref literal is valid"),
            approval_resume: None,
            auth_resume: None,
        };
        let digest = invocation_arguments_digest(&invocation);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex, "4d0ab78e009b32615c2766bd1c26921bd59ef81b5741a75387707f82f0344315",
            "invocation_arguments_digest shifted for a fixed input — \
             this is a wire-contract break. See the stability-contract \
             rustdoc on `invocation_arguments_digest`."
        );
    }

    /// serrrfirat #3637 regression: pin the digest at the boundary that
    /// hook authors actually observe — `BeforeCapabilityHookContext.arguments_digest`
    /// produced by `HookedLoopCapabilityPort::hook_context`. If caller-side
    /// wiring drifts (wrong field set, transform inserted, default value
    /// leaked, or an alternate path bypassing the helper), this assertion
    /// catches it while the helper-only snapshot would stay green.
    #[tokio::test]
    async fn hook_context_arguments_digest_is_stable_at_middleware_boundary() {
        use ironclaw_host_api::TenantId;
        use std::sync::Arc as StdArc;
        struct NoopInner;
        #[async_trait]
        impl LoopCapabilityPort for NoopInner {
            async fn visible_capabilities(
                &self,
                _request: VisibleCapabilityRequest,
            ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
                unreachable!("snapshot test never calls visible_capabilities")
            }
            async fn invoke_capability(
                &self,
                _request: CapabilityInvocation,
            ) -> Result<CapabilityOutcome, AgentLoopHostError> {
                unreachable!("snapshot test never invokes through inner port")
            }
            async fn invoke_capability_batch(
                &self,
                _request: CapabilityBatchInvocation,
            ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
                unreachable!("snapshot test never invokes through inner port")
            }
        }

        let port = HookedLoopCapabilityPort::new(
            StdArc::new(NoopInner),
            StdArc::new(HookDispatcher::new(HookRegistry::new())),
            TenantId::new("alpha").expect("ok"),
        );
        let invocation = CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion::new(
                "snapshot:v1",
            )
            .expect("surface version literal is valid"),
            capability_id: CapabilityId::new("cap.snapshot.fixture")
                .expect("capability id literal is valid"),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:cap.snapshot.fixture",
            )
            .expect("input ref literal is valid"),
            approval_resume: None,
            auth_resume: None,
        };
        let ctx = port.hook_context(&invocation, None).await;
        let hex: String = ctx
            .arguments_digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hex, "4d0ab78e009b32615c2766bd1c26921bd59ef81b5741a75387707f82f0344315",
            "BeforeCapabilityHookContext.arguments_digest shifted at the \
             middleware boundary; this is a hook-visible wire-contract \
            break, not just a helper-output drift."
        );
    }

    /// Caller-driven regression pin for the digest hook authors observe.
    #[tokio::test]
    async fn invoke_capability_arguments_digest_is_stable_at_middleware_boundary() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let hook = CapturingBeforeCapabilityHook::new();
        let dispatcher = dispatcher_with_capturing_hook(hook.clone());
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let outcome = wrapped
            .invoke_capability(snapshot_fixture_invocation())
            .await
            .expect("ok");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(
            inner.calls().len(),
            1,
            "allowed invocation must reach inner"
        );
        assert_hook_observed_snapshot_digest(&hook, "invoke_capability");
    }

    /// Batch-path variant of the caller-driven digest pin.
    #[tokio::test]
    async fn invoke_capability_batch_arguments_digest_is_stable_at_middleware_boundary() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let hook = CapturingBeforeCapabilityHook::new();
        let dispatcher = dispatcher_with_capturing_hook(hook.clone());
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![snapshot_fixture_invocation(), snapshot_fixture_invocation()],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");

        assert_eq!(outcome.outcomes.len(), 2);
        assert!(
            outcome
                .outcomes
                .iter()
                .all(|o| matches!(o, CapabilityOutcome::Completed(_)))
        );

        let captured = hook.captured();
        assert_eq!(
            captured.len(),
            2,
            "hook must observe one BeforeCapability context per batch entry"
        );
        for digest in &captured {
            assert_eq!(
                digest_hex(digest),
                SNAPSHOT_FIXTURE_DIGEST_HEX,
                "arguments_digest observed through invoke_capability_batch shifted; \
                 this is a hook-visible wire-contract break"
            );
        }

        let batch_calls = inner.batch_calls();
        assert_eq!(
            batch_calls.len(),
            1,
            "allowed batch must reach inner as exactly one batched call"
        );
        assert_eq!(batch_calls[0].len(), 2, "both entries batched together");
    }

    /// Distinct inputs must produce distinct digests (sanity check; the
    /// snapshot test pins one specific point in input space, this widens
    /// coverage to the structural property).
    #[test]
    fn invocation_arguments_digest_differs_for_different_input_refs() {
        let cap_id = CapabilityId::new("cap.x").expect("ok");
        let surface = ironclaw_turns::run_profile::CapabilitySurfaceVersion::new("v").expect("ok");
        let a = CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface.clone(),
            capability_id: cap_id.clone(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:a").expect("ok"),
            approval_resume: None,
            auth_resume: None,
        };
        let b = CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface,
            capability_id: cap_id,
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:b").expect("ok"),
            approval_resume: None,
            auth_resume: None,
        };
        assert_ne!(
            invocation_arguments_digest(&a),
            invocation_arguments_digest(&b)
        );
    }

    fn invocation(capability: &str) -> CapabilityInvocation {
        CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new("v1").expect("ok"),
            capability_id: CapabilityId::new(capability).expect("ok"),
            input_ref: CapabilityInputRef::new(format!("input:{capability}")).expect("ok"),
            approval_resume: None,
            auth_resume: None,
        }
    }

    fn dispatcher_with_deny_hook() -> (Arc<HookDispatcher>, HookId) {
        let hook_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new("deny").expect("valid HookLocalId in test"),
            HookVersion::ONE,
        );
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            event_kind_filter: None,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        let mut registry = HookRegistry::new();
        registry.insert(binding).expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            hook_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(DenyingHook)),
        );
        (Arc::new(dispatcher), hook_id)
    }

    #[tokio::test]
    async fn deny_hook_short_circuits_invocation() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) = dispatcher_with_deny_hook();
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert!(matches!(outcome, CapabilityOutcome::Denied(_)));
        assert!(
            inner.calls().is_empty(),
            "inner port must not be invoked when a hook denies"
        );
    }

    #[tokio::test]
    async fn no_hooks_passes_through_to_inner() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(inner.calls().len(), 1);
    }

    #[tokio::test]
    async fn batch_fires_dispatch_per_invocation() {
        // With the always-deny hook installed, every invocation in the batch
        // gets denied by hook dispatch and the inner port is never reached.
        // This verifies the wrapper's per-invocation dispatch loop, not just
        // the single-invocation path.
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) = dispatcher_with_deny_hook();
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 2);
        assert!(inner.calls().is_empty(), "inner must not be invoked");
        for entry in &outcome.outcomes {
            assert!(matches!(entry, CapabilityOutcome::Denied(_)));
        }
    }

    #[tokio::test]
    async fn pause_approval_decision_surfaces_as_approval_required() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) =
            dispatcher_with_restricted_hook("pause-approval", Box::new(PauseApprovalHook));
        // Explicitly opt into the dev-only UUID gate-ref factory; the
        // middleware default is fail-closed (Critical #3).
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
            .with_gate_ref_factory(Arc::new(UuidHookGateRefFactory));

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        match outcome {
            CapabilityOutcome::ApprovalRequired {
                gate_ref,
                safe_summary,
                ..
            } => {
                assert!(gate_ref.as_str().starts_with("gate:hook-approval-"));
                assert_eq!(safe_summary, "needs approval for this capability");
            }
            other => panic!("expected ApprovalRequired, got {other:?}"),
        }
        assert!(inner.calls().is_empty(), "inner must not be invoked");
    }

    #[tokio::test]
    async fn pause_auth_decision_surfaces_as_auth_required() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) =
            dispatcher_with_restricted_hook("pause-auth", Box::new(PauseAuthHook));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
            .with_gate_ref_factory(Arc::new(UuidHookGateRefFactory));

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        match outcome {
            CapabilityOutcome::AuthRequired {
                gate_ref,
                safe_summary,
                ..
            } => {
                assert!(gate_ref.as_str().starts_with("gate:hook-auth-"));
                assert_eq!(safe_summary, "needs auth for this capability");
            }
            other => panic!("expected AuthRequired, got {other:?}"),
        }
        assert!(inner.calls().is_empty(), "inner must not be invoked");
    }

    #[tokio::test]
    async fn gate_ref_factory_failure_falls_back_to_denied() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) =
            dispatcher_with_restricted_hook("pause-approval-fail", Box::new(PauseApprovalHook));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
            .with_gate_ref_factory(Arc::new(FailingGateRefFactory));

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        match outcome {
            CapabilityOutcome::Denied(denied) => {
                assert_eq!(
                    denied.reason_kind,
                    CapabilityDeniedReasonKind::unknown("hook_gate_ref_unavailable").expect("ok"),
                );
                // Sanitized hook reason is preserved; underlying error text
                // ("no router") must not leak.
                assert_eq!(denied.safe_summary, "needs approval for this capability");
            }
            other => panic!("expected Denied fallback, got {other:?}"),
        }
        assert!(inner.calls().is_empty(), "inner must not be invoked");
    }

    /// Companion to `gate_ref_factory_failure_falls_back_to_denied` covering
    /// the `PauseAuth` arm. Deferred from the PR #3573 round-3 review.
    /// When the gate-ref factory's `mint_auth_ref` errors, the middleware
    /// must fail closed: surface `Denied` with the `hook_gate_ref_unavailable`
    /// reason_kind, preserve the hook's sanitized summary, and never reach
    /// the inner port.
    #[tokio::test]
    async fn gate_ref_factory_failure_for_pause_auth_falls_back_to_denied() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) =
            dispatcher_with_restricted_hook("pause-auth-fail", Box::new(PauseAuthHook));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
            .with_gate_ref_factory(Arc::new(FailingGateRefFactory));

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        match outcome {
            CapabilityOutcome::Denied(denied) => {
                assert_eq!(
                    denied.reason_kind,
                    CapabilityDeniedReasonKind::unknown("hook_gate_ref_unavailable").expect("ok"),
                );
                // Sanitized hook reason is preserved; underlying error text
                // ("no router") must not leak.
                assert_eq!(denied.safe_summary, "needs auth for this capability");
            }
            other => panic!("expected Denied fallback, got {other:?}"),
        }
        assert!(inner.calls().is_empty(), "inner must not be invoked");
    }

    /// serrrfirat P2 #3 on PR #3573: when an inner-port `invoke_capability`
    /// in the batch loop returns `Err`, the previous implementation
    /// propagated the error before dispatching `AfterCapability` observers.
    /// This dropped failed batch entries from observer telemetry, in
    /// contrast with the single-invocation path which dispatches observers
    /// regardless. Pin the fixed behavior: the observer fires for the
    /// failing entry, and the error still propagates.
    #[tokio::test]
    async fn batch_dispatches_after_capability_observers_on_inner_error() {
        use crate::points::ObserverHookContext;
        use crate::sink::{ObserverHook, ObserverSink};

        struct FailingPort;
        #[async_trait]
        impl LoopCapabilityPort for FailingPort {
            async fn visible_capabilities(
                &self,
                _request: VisibleCapabilityRequest,
            ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
                unreachable!()
            }
            async fn invoke_capability(
                &self,
                _request: CapabilityInvocation,
            ) -> Result<CapabilityOutcome, AgentLoopHostError> {
                Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                    "inner port failed",
                ))
            }
            async fn invoke_capability_batch(
                &self,
                _request: CapabilityBatchInvocation,
            ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
                // After the batched-dispatch refactor, the wrapper forwards
                // hook-allowed invocations as a single batched call. Surface
                // the same Unavailable error here so the observer-on-error
                // contract from PR #3573 still has coverage.
                Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                    "inner port failed",
                ))
            }
        }

        struct CountingObserver {
            seen: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl ObserverHook for CountingObserver {
            async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
                *self.seen.lock().expect("not poisoned") += 1;
            }
        }

        // Dispatcher with only an AfterCapability observer (no before-cap
        // gate → hooks allow → inner runs and fails).
        let seen = Arc::new(Mutex::new(0u32));
        let observer_id = HookId::for_builtin("test::after_cap_obs", HookVersion::ONE);
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: observer_id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Builtin,
                phase: HookPhase::Telemetry,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::AfterCapability,
                event_kind_filter: None,
                owning_extension: None,
                scope: HookBindingScope::Global,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(
            observer_id,
            crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver {
                seen: seen.clone(),
            })),
        );

        let wrapped =
            HookedLoopCapabilityPort::new(Arc::new(FailingPort), Arc::new(dispatcher), tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![invocation("cap.x")],
            stop_on_first_suspension: false,
        };
        let err = wrapped
            .invoke_capability_batch(batch)
            .await
            .expect_err("inner err propagates");
        assert_eq!(
            err.kind,
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable
        );
        assert_eq!(
            *seen.lock().expect("not poisoned"),
            1,
            "AfterCapability observer must fire even when inner port errors \
             so failed batch entries are visible to telemetry"
        );
    }

    #[tokio::test]
    async fn batch_passes_through_when_no_hooks() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 2);
        assert_eq!(inner.calls().len(), 2);
        for entry in &outcome.outcomes {
            assert!(matches!(entry, CapabilityOutcome::Completed(_)));
        }
    }

    // ── Batched-dispatch regression coverage ───────────────────────────────
    //
    // The middleware previously degraded `invoke_capability_batch` into N
    // sequential `invoke_capability` calls whenever any hook was registered,
    // wiping out the O(1)-batch property that the inner port relies on for
    // bulk-dispatch performance. The tests below pin the restored behavior
    // (PR #3573 deferred refactor).

    /// When NO hook denies, the wrapper must call the inner port's
    /// `invoke_capability_batch` exactly once, with every invocation in a
    /// single payload — not N times via `invoke_capability`.
    #[tokio::test]
    async fn batch_invocation_remains_batched_when_no_hooks_deny() {
        struct AllowingHook;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for AllowingHook {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                sink.pass();
            }
        }

        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) = dispatcher_with_restricted_hook("allowing", Box::new(AllowingHook));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![
                invocation("cap.alpha"),
                invocation("cap.beta"),
                invocation("cap.gamma"),
            ],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 3);
        // The critical perf invariant: ONE batched call, not three sequential.
        let batch_calls = inner.batch_calls();
        assert_eq!(
            batch_calls.len(),
            1,
            "inner port must see exactly one batched call when hooks allow all entries; \
             saw {} batched calls (sequential degradation)",
            batch_calls.len()
        );
        assert_eq!(
            batch_calls[0].len(),
            3,
            "all three entries batched together"
        );
    }

    /// Partial denial: a hook denies some entries; the surviving entries
    /// must still be forwarded as a SINGLE batched call to the inner port,
    /// and the merged outcomes must be in original index order.
    #[tokio::test]
    async fn batch_invocation_filters_denied_entries_and_preserves_index_mapping() {
        /// Denies only the capability whose name matches `target`.
        struct SelectiveDenyHook {
            target: &'static str,
        }
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for SelectiveDenyHook {
            async fn evaluate(
                &self,
                ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                if ctx.capability_name == self.target {
                    sink.deny("selective deny");
                } else {
                    sink.pass();
                }
            }
        }

        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) = dispatcher_with_restricted_hook(
            "selective",
            Box::new(SelectiveDenyHook { target: "cap.beta" }),
        );
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![
                invocation("cap.alpha"),
                invocation("cap.beta"),
                invocation("cap.gamma"),
            ],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 3);
        // Index 0 and 2 forwarded to inner; index 1 short-circuited to Denied.
        assert!(matches!(
            outcome.outcomes[0],
            CapabilityOutcome::Completed(_)
        ));
        assert!(matches!(outcome.outcomes[1], CapabilityOutcome::Denied(_)));
        assert!(matches!(
            outcome.outcomes[2],
            CapabilityOutcome::Completed(_)
        ));

        let batch_calls = inner.batch_calls();
        assert_eq!(
            batch_calls.len(),
            1,
            "remaining entries must be forwarded in a single batched call"
        );
        assert_eq!(
            batch_calls[0].len(),
            2,
            "only the two non-denied entries reach the inner port"
        );
        // Order must match the original (allowed) order: alpha then gamma.
        assert_eq!(batch_calls[0][0].as_str(), "cap.alpha");
        assert_eq!(batch_calls[0][1].as_str(), "cap.gamma");
    }

    /// Even though the inner port is called only ONCE, `AfterCapability`
    /// observers must fire per-entry against the merged outcome vec — the
    /// per-entry telemetry contract from PR #3573 (serrrfirat finding #3)
    /// is independent of the inner-port call topology.
    #[tokio::test]
    async fn batch_invocation_dispatches_after_capability_observer_per_entry() {
        use crate::points::ObserverHookContext;
        use crate::sink::{ObserverHook, ObserverSink};

        struct CountingObserver {
            seen: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl ObserverHook for CountingObserver {
            async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
                *self.seen.lock().expect("not poisoned") += 1;
            }
        }

        let seen = Arc::new(Mutex::new(0u32));
        let observer_id = HookId::for_builtin("test::after_cap_per_entry", HookVersion::ONE);
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: observer_id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Builtin,
                phase: HookPhase::Telemetry,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::AfterCapability,
                owning_extension: None,
                scope: HookBindingScope::Global,
                event_kind_filter: None,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(
            observer_id,
            crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver {
                seen: seen.clone(),
            })),
        );

        let inner = Arc::new(AlwaysCompletedPort::new());
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![
                invocation("cap.alpha"),
                invocation("cap.beta"),
                invocation("cap.gamma"),
            ],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 3);
        assert_eq!(
            inner.batch_calls().len(),
            1,
            "inner port called exactly once (batched)"
        );
        assert_eq!(
            *seen.lock().expect("not poisoned"),
            3,
            "AfterCapability observer must fire per merged entry (3 entries) \
             even though the inner port was batched into a single call"
        );
    }

    // ── C3 regression: provider resolver populates hook context ────────────

    use crate::middleware::resolver::CapabilityProviderResolver;
    use crate::points::BeforeCapabilityHookContext as HookCtxForTest;
    use ironclaw_host_api::ExtensionId as HostExtensionId;

    /// Resolver that records every capability_id it was queried for and
    /// returns a fixed provider for each call.
    struct RecordingProviderResolver {
        provider: HostExtensionId,
        queried: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CapabilityProviderResolver for RecordingProviderResolver {
        async fn provider_for(&self, capability_id: &str) -> Option<HostExtensionId> {
            self.queried
                .lock()
                .expect("recording resolver not poisoned")
                .push(capability_id.to_string());
            Some(self.provider.clone())
        }
    }

    /// Hook that records the provider observed in `ctx.provider`. Always
    /// passes (no opinion) so the inner port still runs.
    struct ProviderRecordingHook {
        observed: Arc<Mutex<Option<Option<HostExtensionId>>>>,
    }

    #[async_trait]
    impl RestrictedBeforeCapabilityHook for ProviderRecordingHook {
        async fn evaluate(&self, ctx: &HookCtxForTest, sink: &mut dyn RestrictedGateSink) {
            *self.observed.lock().expect("observed mutex ok") = Some(ctx.provider.clone());
            sink.pass();
        }
    }

    #[tokio::test]
    async fn provider_resolver_populates_hook_context() {
        let provider = HostExtensionId::new("ext-resolver-test").expect("valid ext id");
        let resolver = Arc::new(RecordingProviderResolver {
            provider: provider.clone(),
            queried: Mutex::new(Vec::new()),
        });

        // Use Global scope so the hook fires; we're testing the *context*,
        // not the scope filter.
        let hook_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new("recording").expect("valid HookLocalId in test"),
            HookVersion::ONE,
        );
        let observed = Arc::new(Mutex::new(None));
        let hook = ProviderRecordingHook {
            observed: Arc::clone(&observed),
        };
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_installed_before_capability(
                hook_id,
                HookPhase::Policy,
                HostExtensionId::new("ext-resolver-test").expect("valid"),
                crate::registry::HookBindingScope::Global,
                Box::new(hook),
            )
            .expect("install ok");

        let inner = Arc::new(AlwaysCompletedPort::new());
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant())
            .with_provider_resolver(Arc::clone(&resolver) as Arc<_>);

        let _ = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        let observed = observed.lock().expect("observed mutex ok").clone();
        assert_eq!(
            observed,
            Some(Some(provider.clone())),
            "hook ctx must carry the resolver-supplied provider"
        );

        let queried = resolver.queried.lock().expect("queries").clone();
        assert_eq!(queried, vec!["cap.x".to_string()]);
    }

    // ── Lazy capability-input resolution (PR #3573 HIGH follow-up) ──────────
    //
    // The middleware must not pay the cost of resolving capability inputs
    // when no active hook would read them. For inputs that are file blobs
    // or other expensive sources, eager resolution can be wasteful or
    // fatal. These tests pin the lazy-probe contract end-to-end via
    // `invoke_capability`, not just through the helper, so that future
    // changes to the dispatch path can't reintroduce eager resolution
    // without also breaking a test.

    use crate::evaluator::PredicateEvaluator;
    use crate::installed_hook::PredicateBackedBeforeCapabilityHook;
    use crate::predicate::{
        CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
    };
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering as AtomicOrdering};

    /// Resolver that records how many times `resolve` and `size_hint` were
    /// called and returns a configurable value/size. Used to prove the
    /// middleware skips work when no predicate needs input.
    struct ProbingResolver {
        resolve_calls: AtomicU32,
        size_hint_calls: AtomicU32,
        size: Option<u64>,
        value: serde_json::Value,
    }

    impl ProbingResolver {
        fn new(value: serde_json::Value) -> Self {
            Self {
                resolve_calls: AtomicU32::new(0),
                size_hint_calls: AtomicU32::new(0),
                size: None,
                value,
            }
        }

        fn with_size(mut self, size: u64) -> Self {
            self.size = Some(size);
            self
        }

        fn resolve_calls(&self) -> u32 {
            self.resolve_calls.load(AtomicOrdering::SeqCst)
        }

        fn size_hint_calls(&self) -> u32 {
            self.size_hint_calls.load(AtomicOrdering::SeqCst)
        }
    }

    #[async_trait]
    impl CapabilityInputResolver for ProbingResolver {
        async fn resolve(&self, _invocation: &CapabilityInvocation) -> Option<serde_json::Value> {
            self.resolve_calls.fetch_add(1, AtomicOrdering::SeqCst);
            Some(self.value.clone())
        }

        async fn size_hint(&self, _invocation: &CapabilityInvocation) -> Option<u64> {
            self.size_hint_calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.size
        }
    }

    fn install_predicate_hook(
        dispatcher: &mut HookDispatcher,
        local: &str,
        spec: HookPredicateSpec,
        evaluator: Arc<PredicateEvaluator>,
    ) -> HookId {
        let hook_id = HookId::derive(
            &ExtensionId::new("ext").expect("ext literal is valid"),
            "1.0",
            &HookLocalId::new(local).expect("local id literal is valid"),
            HookVersion::ONE,
        );
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id, spec, evaluator);
        dispatcher
            .install_installed_before_capability(
                hook_id,
                HookPhase::Policy,
                ironclaw_host_api::ExtensionId::new("ext-test").expect("valid"),
                HookBindingScope::Global,
                Box::new(hook),
            )
            .expect("install ok");
        hook_id
    }

    /// Pin lazy-resolution: when every active predicate-backed hook gates
    /// only on invocation count (no input access), the dispatcher must
    /// not consult the resolver at all. Instrument the resolver and
    /// drive an `invoke_capability` through the middleware to assert
    /// zero reads.
    #[tokio::test]
    async fn dispatch_skips_input_resolution_when_no_predicate_needs_input() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let evaluator = Arc::new(PredicateEvaluator::new());
        // InvocationCount: pure rate counter, never reads input.
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::InvocationCount {
                max: 100,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "rate".to_string(),
            },
        };
        install_predicate_hook(&mut dispatcher, "rate-only", spec, evaluator);

        let resolver = Arc::new(ProbingResolver::new(serde_json::json!({"amount": 1})));
        let wrapped = HookedLoopCapabilityPort::new(inner, Arc::new(dispatcher), tenant())
            .with_resolver(Arc::clone(&resolver) as Arc<_>);

        let _ = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert_eq!(
            resolver.resolve_calls(),
            0,
            "no active predicate needs the capability input — resolver must not be consulted"
        );
        assert_eq!(
            resolver.size_hint_calls(),
            0,
            "size_hint must also be skipped when no hook needs input"
        );
    }

    /// Pin the inverse: when a `NumericSum` predicate is active, the
    /// resolver IS consulted (so the predicate can read the field). This
    /// is the regression complement to the skip test — together they
    /// pin the lazy-probe behavior in both directions.
    #[tokio::test]
    async fn dispatch_reads_input_when_numericsum_predicate_active() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::NumericSum {
                max: "1000".to_string(),
                field: "amount".to_string(),
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "value".to_string(),
            },
        };
        install_predicate_hook(&mut dispatcher, "value-cap", spec, evaluator);

        let resolver = Arc::new(ProbingResolver::new(serde_json::json!({"amount": 1})));
        let wrapped = HookedLoopCapabilityPort::new(inner, Arc::new(dispatcher), tenant())
            .with_resolver(Arc::clone(&resolver) as Arc<_>);

        let _ = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert_eq!(
            resolver.resolve_calls(),
            1,
            "NumericSum predicate needs input; resolver must be consulted exactly once"
        );
    }

    /// Pin the streaming size guard: when `size_hint` reports a value
    /// above `MAX_PREDICATE_INPUT_BYTES`, the middleware must fail
    /// closed without calling `resolve`. The predicate evaluator then
    /// treats the input as unresolved and the dispatch denies per the
    /// `on_exceeded` action — the inner port is never invoked.
    #[tokio::test]
    async fn dispatch_fails_closed_when_input_exceeds_max_bytes() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::NumericSum {
                max: "1000".to_string(),
                field: "amount".to_string(),
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "value".to_string(),
            },
        };
        install_predicate_hook(&mut dispatcher, "value-cap-oversized", spec, evaluator);

        // Size hint reports a value above the cap — resolve must be skipped.
        let resolver = Arc::new(
            ProbingResolver::new(serde_json::json!({"amount": 1}))
                .with_size(MAX_PREDICATE_INPUT_BYTES + 1),
        );
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant())
            .with_resolver(Arc::clone(&resolver) as Arc<_>);

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert!(
            matches!(outcome, CapabilityOutcome::Denied(_)),
            "oversized input must fail closed to a Denied outcome (got {outcome:?})"
        );
        assert_eq!(
            resolver.size_hint_calls(),
            1,
            "size_hint must be consulted exactly once"
        );
        assert_eq!(
            resolver.resolve_calls(),
            0,
            "resolve must be skipped when size_hint exceeds the cap"
        );
        assert!(
            inner.calls().is_empty(),
            "inner port must not be invoked when the hook denies"
        );
    }

    /// Pin the streaming order-of-operations: `size_hint` is consulted
    /// **before** `resolve` for input-reading predicates. Without this
    /// ordering, the middleware would materialize the value first and
    /// only then notice it was too large — defeating the purpose of the
    /// streaming pre-check.
    #[tokio::test]
    async fn dispatch_streams_size_check_before_full_materialization() {
        // Resolver that records the order of its calls. If `resolve` is
        // ever observed before `size_hint`, the test fails.
        struct OrderingResolver {
            sequence: AtomicU64,
            size_hint_seq: AtomicU64,
            resolve_seq: AtomicU64,
            size: u64,
        }
        #[async_trait]
        impl CapabilityInputResolver for OrderingResolver {
            async fn resolve(
                &self,
                _invocation: &CapabilityInvocation,
            ) -> Option<serde_json::Value> {
                let s = self.sequence.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                self.resolve_seq.store(s, AtomicOrdering::SeqCst);
                Some(serde_json::json!({"amount": 1}))
            }
            async fn size_hint(&self, _invocation: &CapabilityInvocation) -> Option<u64> {
                let s = self.sequence.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                self.size_hint_seq.store(s, AtomicOrdering::SeqCst);
                Some(self.size)
            }
        }

        // Use a size just below the cap so resolve still runs after the
        // pre-check — we want to assert *ordering*, not skip.
        let resolver = Arc::new(OrderingResolver {
            sequence: AtomicU64::new(0),
            size_hint_seq: AtomicU64::new(0),
            resolve_seq: AtomicU64::new(0),
            size: MAX_PREDICATE_INPUT_BYTES - 1,
        });

        let inner = Arc::new(AlwaysCompletedPort::new());
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::Always,
            bound: ValueOrRateBound::NumericSum {
                max: "1000".to_string(),
                field: "amount".to_string(),
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "value".to_string(),
            },
        };
        install_predicate_hook(&mut dispatcher, "ordering", spec, evaluator);

        let wrapped = HookedLoopCapabilityPort::new(inner, Arc::new(dispatcher), tenant())
            .with_resolver(Arc::clone(&resolver) as Arc<_>);

        let _ = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        let size_hint_seq = resolver.size_hint_seq.load(AtomicOrdering::SeqCst);
        let resolve_seq = resolver.resolve_seq.load(AtomicOrdering::SeqCst);
        assert!(size_hint_seq > 0, "size_hint must have been called");
        assert!(resolve_seq > 0, "resolve must have been called");
        assert!(
            size_hint_seq < resolve_seq,
            "size_hint (seq {size_hint_seq}) must be consulted before resolve (seq {resolve_seq}) so oversized inputs never get materialized"
        );
    }

    /// henrypark133 L2 on PR #3913: when several BeforeCapability hooks
    /// share a scope+provider, `before_capability_needs_input` must
    /// return `true` as soon as ANY active binding's hook reports
    /// `needs_input() = true`. The short-circuit doesn't depend on which
    /// hook the trait's default lives on, so a mixed pair (one false,
    /// one true) must still cause `resolve_arguments` to be called by
    /// the middleware. Drive the whole path via `invoke_capability` and
    /// an instrumented resolver to confirm the call actually happens.
    #[tokio::test]
    async fn before_capability_needs_input_returns_true_when_any_active_binding_needs_input() {
        struct NoInputHook;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for NoInputHook {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                sink.pass();
            }
            fn needs_input(&self) -> bool {
                false
            }
        }

        struct NeedsInputHook;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for NeedsInputHook {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                sink.pass();
            }
            fn needs_input(&self) -> bool {
                true
            }
        }

        // Two BeforeCapability bindings on the same scope. One opts
        // out of input access; the other opts in. The dispatcher's
        // input-needed probe must short-circuit to true on the second
        // binding regardless of registration order.
        let no_input_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid"),
            "1.0",
            &HookLocalId::new("no-input").expect("valid"),
            HookVersion::ONE,
        );
        let needs_input_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid"),
            "1.0",
            &HookLocalId::new("needs-input").expect("valid"),
            HookVersion::ONE,
        );
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: no_input_id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Installed,
                phase: HookPhase::Policy,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::BeforeCapability,
                owning_extension: None,
                scope: HookBindingScope::Global,
                event_kind_filter: None,
                poisoned: false,
            })
            .expect("insert no_input");
        registry
            .insert(HookBinding {
                hook_id: needs_input_id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Installed,
                phase: HookPhase::Policy,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::BeforeCapability,
                owning_extension: None,
                scope: HookBindingScope::Global,
                event_kind_filter: None,
                poisoned: false,
            })
            .expect("insert needs_input");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            no_input_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(NoInputHook)),
        );
        dispatcher.install_before_capability(
            needs_input_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(NeedsInputHook)),
        );

        // First half: probe directly via the dispatcher's
        // `before_capability_needs_input` API.
        let dispatcher = Arc::new(dispatcher);
        assert!(
            dispatcher.before_capability_needs_input(None),
            "with a mixed pair (one needs_input=false, one needs_input=true) the \
             dispatcher must return true so the middleware materializes input",
        );

        // Second half: drive `invoke_capability` end-to-end with an
        // instrumented resolver. The middleware must consult `resolve`
        // because at least one active binding needs input — confirming
        // the short-circuit is wired all the way through the call site,
        // not just the helper.
        let resolver = Arc::new(ProbingResolver::new(serde_json::json!({"amount": 1})));
        let inner = Arc::new(AlwaysCompletedPort::new());
        let wrapped = HookedLoopCapabilityPort::new(inner, dispatcher, tenant())
            .with_resolver(Arc::clone(&resolver) as Arc<_>);

        let _ = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert_eq!(
            resolver.resolve_calls(),
            1,
            "resolver must be called exactly once when at least one active \
             binding's hook reports needs_input()=true",
        );
    }

    /// henrypark133 M1 on PR #3911: in the merged-batch path, when an
    /// allowed (Pending) entry precedes a hook-resolved suspension entry
    /// and `stop_on_first_suspension` is true, the merge loop must still
    /// fire the `AfterCapability` observer for the suspension slot. The
    /// pre-fix loop initialized `stopped_on_suspension` from
    /// `stopped_in_preflight` and broke after the first iteration —
    /// dropping the observer for the trailing Resolved suspension slot
    /// and losing it from `outcomes` entirely. This pins the fixed
    /// behavior: every slot's observer fires and every Resolved slot
    /// surfaces in `outcomes`, even when `stop_on_first_suspension` is
    /// set and the suspension appears after an allowed entry.
    #[tokio::test]
    async fn batch_invocation_fires_observer_for_hook_suspended_entry_after_allowed_entry_with_stop_on_first_suspension()
     {
        use crate::points::ObserverHookContext;
        use crate::sink::{ObserverHook, ObserverSink};

        /// Allows `cap.alpha`, pauses `cap.beta` for approval. The
        /// mixed-batch scenario the merge loop has to handle correctly.
        struct SelectivePauseHook;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for SelectivePauseHook {
            async fn evaluate(
                &self,
                ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                if ctx.capability_name == "cap.beta" {
                    sink.pause_approval("needs approval");
                } else {
                    sink.pass();
                }
            }
        }

        struct CountingObserver {
            seen: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl ObserverHook for CountingObserver {
            async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
                *self.seen.lock().expect("not poisoned") += 1;
            }
        }

        // Set up a dispatcher with both the gating hook AND an
        // AfterCapability observer so we can count per-entry observer
        // dispatch.
        let gating_id = HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new("selective-pause").expect("valid HookLocalId in test"),
            HookVersion::ONE,
        );
        let gating_binding = HookBinding {
            hook_id: gating_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            event_kind_filter: None,
            poisoned: false,
        };

        let observer_id = HookId::for_builtin("test::after_cap_mixed_susp", HookVersion::ONE);
        let observer_binding = HookBinding {
            hook_id: observer_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Builtin,
            phase: HookPhase::Telemetry,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::AfterCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            event_kind_filter: None,
            poisoned: false,
        };

        let mut registry = HookRegistry::new();
        registry.insert(gating_binding).expect("insert gating");
        registry.insert(observer_binding).expect("insert observer");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            gating_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(SelectivePauseHook)),
        );
        let seen = Arc::new(Mutex::new(0u32));
        dispatcher.install_observer_impl(
            observer_id,
            crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver {
                seen: seen.clone(),
            })),
        );

        let inner = Arc::new(AlwaysCompletedPort::new());
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant())
            .with_gate_ref_factory(Arc::new(UuidHookGateRefFactory));

        // alpha (hook-allowed → Pending) precedes beta (hook-suspension
        // → Resolved). `stop_on_first_suspension` is true so the bug
        // path: the merge loop sees `stopped_in_preflight = true` from
        // Phase 1 (beta was the suspension trigger), pushes alpha, then
        // breaks before reaching beta's slot — dropping beta's observer
        // and outcome.
        let batch = CapabilityBatchInvocation {
            invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
            stop_on_first_suspension: true,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");

        assert_eq!(
            outcome.outcomes.len(),
            2,
            "merged outcomes must contain both entries: the hook-allowed alpha \
             (from inner) and the hook-suspended beta (from hook resolution). \
             Pre-fix this dropped beta to a length of 1.",
        );
        assert!(
            matches!(outcome.outcomes[0], CapabilityOutcome::Completed(_)),
            "alpha was hook-allowed and inner produced a Completed outcome",
        );
        assert!(
            matches!(
                outcome.outcomes[1],
                CapabilityOutcome::ApprovalRequired { .. }
            ),
            "beta was hook-resolved to ApprovalRequired",
        );
        assert!(
            outcome.stopped_on_suspension,
            "stop_on_first_suspension was set and a suspension surfaced",
        );
        assert_eq!(
            *seen.lock().expect("not poisoned"),
            2,
            "AfterCapability observer must fire for BOTH entries — the \
             hook-allowed alpha AND the hook-resolved suspension beta. \
             Pre-fix the merge loop broke before firing beta's observer.",
        );
        // No inner work for beta — the hook short-circuited it. The
        // inner test port's `invoke_capability_batch` impl delegates
        // per-entry to `invoke_capability`, so `calls()` counts the
        // entries the inner port saw; only alpha should appear.
        let calls = inner.calls();
        assert_eq!(
            calls.len(),
            1,
            "only alpha must reach the inner port; beta was hook-suspended",
        );
        assert_eq!(calls[0].as_str(), "cap.alpha");
        let batch_calls = inner.batch_calls();
        assert_eq!(
            batch_calls.len(),
            1,
            "the inner port must be batched-called exactly once with just alpha",
        );
        assert_eq!(batch_calls[0].len(), 1);
        assert_eq!(batch_calls[0][0].as_str(), "cap.alpha");
    }
}
