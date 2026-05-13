//! End-to-end smoke test for the foundation slice: parse a manifest entry,
//! derive a hook id, build a binding, register a stub hook impl in the
//! dispatcher, dispatch, and assert the composed outcome reflects the
//! manifest's intent.
//!
//! This test does *not* cover predicate evaluation (no evaluator ships in
//! this slice) and does *not* cover Reborn middleware composition (next
//! slice). It exists to prove the cross-module shapes fit together.

use async_trait::async_trait;
use ironclaw_hooks::{
    dispatch::HookDispatcherBuilder,
    identity::{ExtensionId, HookId, HookLocalId, HookVersion},
    manifest::{HookManifestBody, HookManifestEntry, HookManifestKind, HookManifestScope},
    ordering::{HookPhase, HookPriority},
    points::BeforeCapabilityHookContext,
    predicate::{CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound},
    registry::{HookBindingScope, HookRegistry},
    sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink},
};

fn tenant() -> ironclaw_host_api::TenantId {
    ironclaw_host_api::TenantId::new("alpha").expect("valid tenant")
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
    let manifest_entry = HookManifestEntry {
        id: HookLocalId("daily-order-cap".to_string()),
        kind: HookManifestKind::BeforeCapability,
        scope: HookManifestScope::OwnCapabilities,
        phase: HookPhase::Policy,
        priority: HookPriority::DEFAULT,
        description: Some("Cap at 10 orders/day".to_string()),
        requires_grant: None,
        body: HookManifestBody::Predicate {
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
    };
    manifest_entry.validate().expect("manifest validates");

    // 2. Registry installer pins a content-addressed hook id. (In production
    //    this happens inside the installer; here we drive the same pieces
    //    directly through the tier-specific public installer.)
    let hook_id = HookId::derive(
        &ExtensionId("polymarket-trader".to_string()),
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
            ironclaw_host_api::ExtensionId::new("polymarket-trader").expect("valid ext id"),
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
