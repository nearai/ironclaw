//! Glue between extension-manifest-declared predicates and the dispatcher's
//! hook trait surface.
//!
//! The registry installer constructs a [`PredicateBackedBeforeCapabilityHook`]
//! for each `[[hooks]]` entry whose body is `HookManifestBody::Predicate`.
//! The hook holds an `Arc` to the shared [`PredicateEvaluator`] (so sliding-
//! window state is shared across all predicate-backed hooks in a run) plus
//! the spec it was constructed from.

use std::sync::Arc;

use async_trait::async_trait;

use crate::evaluator::{EvaluatorDecision, PredicateEvaluator};
use crate::identity::HookId;
use crate::points::BeforeCapabilityHookContext;
use crate::predicate::HookPredicateSpec;
use crate::sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink};

/// A `before_capability` hook implementation backed by a declarative
/// predicate from an extension manifest. Always `Installed`-tier.
pub struct PredicateBackedBeforeCapabilityHook {
    hook_id: HookId,
    spec: HookPredicateSpec,
    evaluator: Arc<PredicateEvaluator>,
}

impl PredicateBackedBeforeCapabilityHook {
    pub fn new(
        hook_id: HookId,
        spec: HookPredicateSpec,
        evaluator: Arc<PredicateEvaluator>,
    ) -> Self {
        Self {
            hook_id,
            spec,
            evaluator,
        }
    }
}

#[async_trait]
impl RestrictedBeforeCapabilityHook for PredicateBackedBeforeCapabilityHook {
    fn needs_input(&self) -> bool {
        // Predicate-backed hooks read inputs only when their spec does
        // (currently `NumericSum`). Delegating here lets the dispatch
        // middleware skip eager input resolution when every active
        // predicate-backed hook is purely structural / rate-limited.
        self.spec.needs_input()
    }

    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn RestrictedGateSink) {
        // Sinks take `&'static str` reasons to keep adversarial format!-built
        // strings out of the seam. Predicate reasons come from the manifest
        // (author-controlled) and are dynamic, so the evaluator's reason
        // string is leaked as a closed vocabulary of static labels here.
        // Richer reasons surface in audit, not in the model-visible decision.
        //
        // The `DenyReasonCode` / `PauseReasonCode` enums are themselves
        // closed-vocabulary, and each variant's `as_label()` returns a
        // `&'static str` — so we can surface a richer model-visible label
        // (`hook_rate_limit`, `hook_value_cap`, ...) without opening a
        // free-form text channel.
        match self.evaluator.evaluate(self.hook_id, &self.spec, ctx) {
            EvaluatorDecision::Allow => {
                // The predicate did not match — the hook has no opinion. The
                // dispatcher recognizes `pass()` as a no-opinion contribution
                // and continues composing without short-circuiting.
                sink.pass();
            }
            EvaluatorDecision::Deny { code, reason } => {
                // serrrfirat #3636: model sees the closed-vocab label; audit/SSE
                // gets the manifest's free-form reason via a separate channel.
                // The two are intentionally split so a predicate author can name
                // *why* (rich, operator-facing) without minting a model-visible
                // label that could itself be a steering surface.
                sink.record_audit_reason(reason);
                sink.deny(code.as_label());
            }
            EvaluatorDecision::PauseApproval { code, reason } => {
                sink.record_audit_reason(reason);
                sink.pause_approval(code.as_label());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};
    use crate::predicate::{CapabilityPredicate, HookPredicateSpec};
    use crate::sink::RecordingGateSink;
    use ironclaw_host_api::TenantId;

    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion::ONE,
        )
    }

    #[tokio::test]
    async fn deny_predicate_routes_to_sink_deny() {
        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: "ignored: routes to closed-vocab label".to_string(),
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            TenantId::new("alpha").expect("ok"),
            "shell.exec".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        let decision = sink.decision().expect("hook emitted a decision");
        assert!(!decision.permits());
        // Legacy `DenyCapability` (no code) maps to the Generic label,
        // preserving the existing `hook_predicate_denied` model-visible
        // string. Back-compat property.
        match decision.view() {
            crate::kinds::gate::GateDecisionView::Deny { reason } => {
                assert_eq!(reason.as_str(), "hook_predicate_denied");
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// `DenyWithCode` surfaces the code's label as the model-visible
    /// reason, instead of the generic `hook_predicate_denied`. This is
    /// the load-bearing affirmative-path test for the new variant.
    #[tokio::test]
    async fn rate_or_value_cap_with_deny_code_routes_to_code_label() {
        use crate::predicate::{DenyReasonCode, OnExceededAction, ValueOrRateBound};

        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "polymarket.place_order".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 0,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::DenyWithCode {
                code: DenyReasonCode::RateLimit,
                reason: "audit-only: daily cap exceeded".to_string(),
            },
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            TenantId::new("alpha").expect("ok"),
            "polymarket.place_order".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        let decision = sink.decision().expect("hook emitted a decision");
        match decision.view() {
            crate::kinds::gate::GateDecisionView::Deny { reason } => {
                assert_eq!(
                    reason.as_str(),
                    "hook_rate_limit",
                    "DenyWithCode {{ code: RateLimit }} must surface the \
                     code's label, not the legacy hook_predicate_denied"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// `PauseApprovalWithCode` is the symmetric affirmative-path test
    /// for the pause variant.
    #[tokio::test]
    async fn rate_or_value_cap_with_pause_code_routes_to_code_label() {
        use crate::predicate::{OnExceededAction, PauseReasonCode, ValueOrRateBound};

        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "polymarket.place_order".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 0,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::PauseApprovalWithCode {
                code: PauseReasonCode::OverThreshold,
                reason: "audit-only: $1000/24h threshold".to_string(),
            },
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            TenantId::new("alpha").expect("ok"),
            "polymarket.place_order".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        let decision = sink.decision().expect("hook emitted a decision");
        match decision.view() {
            crate::kinds::gate::GateDecisionView::PauseApproval { reason } => {
                assert_eq!(reason.as_str(), "hook_pause_over_threshold");
            }
            other => panic!("expected PauseApproval, got {other:?}"),
        }
    }

    /// serrrfirat #3636 regression: the manifest's free-form reason text
    /// must reach the audit channel (the recording sink's `audit_reason`)
    /// while the sink's decision reason stays on the closed-vocab label.
    /// Model sees `hook_rate_limit`; audit sees the manifest text.
    #[tokio::test]
    async fn deny_with_code_records_audit_reason_separately_from_model_label() {
        use crate::predicate::{DenyReasonCode, OnExceededAction, ValueOrRateBound};

        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "polymarket.place_order".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 0,
                window: "1h".to_string(),
            },
            on_exceeded: OnExceededAction::DenyWithCode {
                code: DenyReasonCode::RateLimit,
                reason: "daily cap of $1000 exceeded at 14:32 UTC".to_string(),
            },
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            TenantId::new("alpha").expect("ok"),
            "polymarket.place_order".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        let decision = sink.decision().expect("hook emitted a decision");
        match decision.view() {
            crate::kinds::gate::GateDecisionView::Deny { reason } => {
                assert_eq!(
                    reason.as_str(),
                    "hook_rate_limit",
                    "model-visible reason must be the closed-vocab label, \
                     never the manifest free-form text"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
        assert_eq!(
            sink.audit_reason.as_deref(),
            Some("daily cap of $1000 exceeded at 14:32 UTC"),
            "audit channel must receive the manifest's free-form reason \
             intact for operator-facing SSE/audit consumers"
        );
    }

    #[tokio::test]
    async fn allow_predicate_routes_to_sink_pass() {
        use crate::sink::GateSinkState;

        let evaluator = Arc::new(PredicateEvaluator::new());
        // Spec only fires on `shell.exec`; context invokes a different
        // capability so the evaluator returns Allow.
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: "shell denied".to_string(),
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            TenantId::new("alpha").expect("ok"),
            "memory.read".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        assert!(
            sink.decision().is_none(),
            "no-opinion path must not record a decision"
        );
        assert_eq!(sink.state, GateSinkState::Passed);
    }
}
