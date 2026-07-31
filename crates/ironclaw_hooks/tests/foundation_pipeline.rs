//! End-to-end smoke test for the foundation slice: parse a manifest entry,
//! derive a hook id, build a binding, register a stub hook impl in the
//! dispatcher, dispatch, and assert the composed outcome reflects the
//! manifest's intent.
//!
//! This test does *not* cover predicate evaluation (no evaluator ships in
//! this slice) and does *not* cover Reborn middleware composition (next
//! slice). It exists to prove the cross-module shapes fit together.

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_hooks::{
    dispatch::HookDispatcherBuilder,
    identity::{ExtensionId, HookId, HookLocalId, HookVersion},
    manifest::{HookManifestBody, HookManifestEntry, HookManifestKind},
    points::BeforeCapabilityHookContext,
    predicate::{CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound},
    registry::{HookBindingScope, HookRegistry},
    self_authored::{
        GenerationTraceRef, SelfAuthoredBeforeCapabilityHook, SelfAuthoredHookSink,
        SelfAuthoredHookSpec, SelfAuthoredReason, SelfAuthorshipProvenance,
    },
    sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink},
};

fn tenant() -> ironclaw_host_api::ids::TenantId {
    ironclaw_host_api::ids::TenantId::new("alpha").expect("valid tenant")
}

/// Stand-in for the host's eventual predicate evaluator. In production the
/// evaluator would inspect the manifest's `HookPredicateSpec` and produce
/// the appropriate decision; here we just verify the binding/dispatch wiring
/// fires by hardcoding a deny.
struct DenyEverythingFromManifest;

#[async_trait]
impl RestrictedBeforeCapabilityHook for DenyEverythingFromManifest {
    async fn evaluate(
        &self,
        _ctx: &BeforeCapabilityHookContext,
        sink: &mut dyn RestrictedGateSink,
    ) {
        sink.deny("denied by predicate stub");
    }
}

#[tokio::test]
async fn manifest_to_dispatch_pipeline() {
    // 1. Author publishes a manifest entry.
    let manifest_entry = HookManifestEntry::new(
        HookLocalId::new("daily-order-cap").expect("valid HookLocalId in test"),
        HookManifestKind::BeforeCapability,
        HookManifestBody::Predicate {
            spec: HookPredicateSpec::RateOrValueCap {
                when: CapabilityPredicate::NameEquals {
                    name: "polymarket.place_order".to_string(),
                },
                bound: ValueOrRateBound::InvocationCount {
                    max: 10,
                    window: "24h".to_string(),
                },
                on_exceeded: OnExceededAction::Deny {
                    reason: "daily cap".to_string(),
                },
            },
        },
    )
    .with_description("Cap at 10 orders/day");
    manifest_entry.validate().expect("manifest validates");

    // 2. Registry installer pins a content-addressed hook id. (In production
    //    this happens inside the installer; here we drive the same pieces
    //    directly through the tier-specific public installer.)
    let hook_id = HookId::derive(
        &ExtensionId::new("polymarket-trader").expect("valid ExtensionId in test"),
        "0.4.2",
        &manifest_entry.id,
        HookVersion::ONE,
    );

    // 3. The dispatcher consumes the binding and an installed impl. The
    //    Installed-tier installer constructs the binding internally and
    //    enforces the trust × phase × impl-tier pairing.
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            manifest_entry.phase,
            ironclaw_host_api::ids::ExtensionId::new("polymarket-trader").expect("valid ext id"),
            // Use Global so the dispatcher fires the hook regardless of the
            // ctx's `provider` field (the dispatch ctx in this test has no
            // provider configured). Scope filtering itself is covered by
            // dedicated tests in `dispatch.rs`.
            HookBindingScope::Global,
            Box::new(DenyEverythingFromManifest),
        )
        .expect("installed-tier hook installs at policy phase")
        .build_arc();

    // 4. Dispatch sees the deny decision; the composed outcome reflects it.
    let ctx = BeforeCapabilityHookContext::new_unresolved(
        tenant(),
        "polymarket.place_order".to_string(),
        [42u8; 32],
    );
    let outcome = dispatcher.dispatch_before_capability(&ctx).await;
    assert!(!outcome.decision.permits());
    assert!(outcome.failures.is_empty());
}

struct SelfAuthoredDispatcherAdapter(SelfAuthoredBeforeCapabilityHook);

#[async_trait]
impl RestrictedBeforeCapabilityHook for SelfAuthoredDispatcherAdapter {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn RestrictedGateSink) {
        struct SinkBridge<'a> {
            sink: &'a mut dyn RestrictedGateSink,
        }

        impl SelfAuthoredHookSink for SinkBridge<'_> {
            fn deny(&mut self, reason: SelfAuthoredReason) {
                self.sink.deny(reason.label());
            }

            fn pause_approval(&mut self, reason: SelfAuthoredReason) {
                self.sink.pause_approval(reason.label());
            }

            fn pause_auth(&mut self, reason: SelfAuthoredReason) {
                self.sink.pause_auth(reason.label());
            }

            fn pass(&mut self) {
                self.sink.pass();
            }
        }

        let mut bridge = SinkBridge { sink };
        self.0.evaluate(ctx, &mut bridge);
    }
}

#[tokio::test]
async fn self_authored_deny_flows_through_dispatcher() {
    let extension = ExtensionId::new("self-authored").expect("valid extension id");
    let hook_local_id = HookLocalId::new("deny-shell").expect("valid hook local id");
    let hook_id = HookId::derive(&extension, "1.0.0", &hook_local_id, HookVersion::ONE);
    let spec = SelfAuthoredHookSpec::DenyCapability {
        when: CapabilityPredicate::NameEquals {
            name: "shell.exec".to_string(),
        },
        reason: SelfAuthoredReason::AgentObservedNearMiss,
    };
    let hook = SelfAuthoredBeforeCapabilityHook::new(
        hook_id,
        spec.clone(),
        SelfAuthorshipProvenance {
            authored_by_run: ironclaw_turns::TurnRunId::new(),
            authored_by_turn: ironclaw_turns::TurnId::new(),
            authored_at: Utc::now(),
            spec_digest: spec.digest(),
            user_ratification: None,
            generation_trace_ref: GenerationTraceRef::new("trace://test".to_string()),
        },
    );
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            ironclaw_hooks::HookPhase::Policy,
            ironclaw_host_api::ids::ExtensionId::new("self-authored")
                .expect("valid host extension id"),
            HookBindingScope::Global,
            Box::new(SelfAuthoredDispatcherAdapter(hook)),
        )
        .expect("self-authored adapter installs through dispatcher")
        .build_arc();

    let denied = dispatcher
        .dispatch_before_capability(&BeforeCapabilityHookContext::new_unresolved(
            tenant(),
            "shell.exec".to_string(),
            [7u8; 32],
        ))
        .await;
    assert!(!denied.decision.permits());
    assert!(denied.failures.is_empty());
}
