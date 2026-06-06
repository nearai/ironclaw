//! Three "real" hooks built against the public `ironclaw_hooks` API.
//!
//! Purpose: surface API ergonomics. Each hook below mimics what an
//! extension author or a system author would actually write. The
//! friction we discovered while writing them is documented in
//! `docs/real-hooks-findings.md` (companion artifact). The tests
//! themselves serve as runnable examples and as a regression net
//! against the public surface drifting.
//!
//! The three hooks intentionally span the framework's design space:
//!
//! 1. **`polymarket-daily-cap`** — Installed-tier predicate hook, no
//!    body-extension code, declarative manifest, rate-cap with
//!    `InvocationCount` and `Deny`. The canonical "rate-limit a
//!    capability" use case the predicate language was built for.
//!
//! 2. **`large-stake-approval-gate`** — Installed-tier predicate hook
//!    using `NumericSum` + `PauseApproval`. Requires resolved capability
//!    arguments (the resolver wiring lives in `ironclaw_reborn`, so the
//!    test here exercises only the manifest + registrar surface and
//!    asserts the bound is well-formed; the end-to-end dispatch
//!    against real numeric inputs lives in
//!    `crates/ironclaw_reborn/tests/hooks_integration.rs`).
//!
//! 3. **`pii-redaction-warning`** — Trusted-tier Rust hook implementing
//!    `PrivilegedBeforePromptHook`, injecting a trusted instruction
//!    snippet that warns the model to redact PII. Demonstrates the path
//!    a system author (not a third-party extension) would take when the
//!    predicate language isn't expressive enough.

use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_hooks::HookTrustClass;
use ironclaw_hooks::dispatch::HookDispatcherBuilder;
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::identity::HookLocalId;
use ironclaw_hooks::kinds::gate::GateDecisionView;
use ironclaw_hooks::kinds::mutator::{HookPatchView, PatchOrdinalHint, SnippetBodyView};
use ironclaw_hooks::manifest::{HookManifestBody, HookManifestEntry, HookManifestKind};
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::{BeforeCapabilityHookContext, BeforePromptHookContext};
use ironclaw_hooks::predicate::{
    CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
};
use ironclaw_hooks::registrar::HookRegistrar;
use ironclaw_hooks::registry::HookRegistry;
use ironclaw_hooks::sink::{PrivilegedBeforePromptHook, PrivilegedMutatorSink};
use ironclaw_host_api::{ExtensionId, TenantId};

// ─────────────────────────────────────────────────────────────────────────
// Hook 1 — polymarket daily cap
// ─────────────────────────────────────────────────────────────────────────

/// Builds the manifest entry an extension would ship in its `[[hooks]]`
/// section. A hand-written extension would put this in TOML; the
/// registrar consumes the same struct either way.
fn polymarket_daily_cap_manifest() -> HookManifestEntry {
    HookManifestEntry::new(
        HookLocalId::new("polymarket-daily-cap").expect("valid HookLocalId in test"),
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
                    reason: "daily place_order cap exceeded".to_string(),
                },
            },
        },
    )
    .with_description("Cap polymarket.place_order at 10 calls per 24h")
}

#[tokio::test]
async fn polymarket_daily_cap_denies_after_ten_invocations() {
    let extension = ExtensionId::new("polymarket-trader").expect("valid ext id");
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let entries = vec![polymarket_daily_cap_manifest()];
    let (builder, ids) = registrar
        .install(extension.clone(), "0.4.2", &entries, builder)
        .expect("manifest installs cleanly");
    assert_eq!(ids.len(), 1);
    let dispatcher = builder.build_arc();

    let tenant = TenantId::new("alice").expect("valid tenant");
    let ctx = |digest: [u8; 32]| {
        BeforeCapabilityHookContext::new(
            tenant.clone(),
            "polymarket.place_order".to_string(),
            digest,
            ironclaw_hooks::points::SanitizedArguments::unresolved(),
            Some(extension.clone()),
        )
    };

    // First 10 invocations should be allowed (under cap).
    for i in 0..10u8 {
        let outcome = dispatcher.dispatch_before_capability(&ctx([i; 32])).await;
        assert!(
            outcome.decision.permits(),
            "invocation {i} should be under cap; got {:?}",
            outcome.decision.view()
        );
    }

    // 11th invocation: cap tripped, expect Deny. Note that the model-visible
    // reason is a *closed-vocabulary label* (`hook_predicate_denied`), not
    // the manifest text. This is intentional — see
    // `installed_hook.rs::evaluate` and the friction-findings doc — and is
    // pinned here so a future change that surfaces manifest-supplied
    // strings to the model has to update this assertion deliberately.
    let denied = dispatcher.dispatch_before_capability(&ctx([99; 32])).await;
    match denied.decision.view() {
        GateDecisionView::Deny { reason } => {
            assert_eq!(
                reason.as_str(),
                "hook_predicate_denied",
                "deny reason vocabulary should remain closed; richer text \
                 belongs in the audit log, not the model-visible decision"
            );
        }
        other => panic!("expected Deny at 11th invocation; got {other:?}"),
    }
}

#[tokio::test]
async fn polymarket_daily_cap_does_not_fire_for_other_capabilities() {
    // Same manifest, different capability invoked — predicate's
    // `NameEquals` clause must filter out non-matching invocations.
    let extension = ExtensionId::new("polymarket-trader").expect("valid ext id");
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let entries = vec![polymarket_daily_cap_manifest()];
    let (builder, _ids) = registrar
        .install(extension.clone(), "0.4.2", &entries, builder)
        .expect("installs");
    let dispatcher = builder.build_arc();

    let tenant = TenantId::new("alice").expect("valid tenant");
    let ctx = BeforeCapabilityHookContext::new(
        tenant,
        "polymarket.get_portfolio".to_string(),
        [0u8; 32],
        ironclaw_hooks::points::SanitizedArguments::unresolved(),
        Some(extension),
    );
    let outcome = dispatcher.dispatch_before_capability(&ctx).await;
    assert!(
        outcome.decision.permits(),
        "non-matching capability must pass through"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Hook 2 — large-stake approval gate (manifest-shape coverage only)
// ─────────────────────────────────────────────────────────────────────────

/// Manifest entry for a `$1000/24h` cumulative-stake approval gate. The
/// `NumericSum` predicate needs resolved capability arguments to fire;
/// the resolver path lives in `ironclaw_reborn`, so this test only
/// exercises that the manifest is well-formed and that the registrar
/// accepts it. The dispatch-time fire-or-not test is in
/// `crates/ironclaw_reborn/tests/hooks_integration.rs::numeric_sum_predicate_caps_total_value_against_real_inputs`.
fn large_stake_approval_manifest() -> HookManifestEntry {
    HookManifestEntry::new(
        HookLocalId::new("large-stake-approval-gate").expect("valid HookLocalId in test"),
        HookManifestKind::BeforeCapability,
        HookManifestBody::Predicate {
            spec: HookPredicateSpec::RateOrValueCap {
                when: CapabilityPredicate::NameEquals {
                    name: "polymarket.place_order".to_string(),
                },
                bound: ValueOrRateBound::NumericSum {
                    max: "1000".to_string(),
                    field: "amount_usd".to_string(),
                    window: "24h".to_string(),
                },
                on_exceeded: OnExceededAction::PauseApproval {
                    reason: "cumulative stake exceeds $1000/24h — approval required".to_string(),
                },
            },
        },
    )
    .with_description("Require user approval when cumulative stake exceeds $1000/24h")
}

#[tokio::test]
async fn large_stake_approval_manifest_validates_and_installs() {
    // Validate the manifest itself first — this surfaces format/scope
    // errors before the registrar runs and reduces the diagnostic
    // surface when something is wrong.
    let entry = large_stake_approval_manifest();
    entry.validate().expect("manifest must validate");

    let extension = ExtensionId::new("polymarket-trader").expect("valid ext id");
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let entries = vec![entry];
    let (_builder, ids) = registrar
        .install(extension, "0.4.2", &entries, builder)
        .expect("approval-gate manifest installs through the registrar");
    assert_eq!(ids.len(), 1);
}

#[tokio::test]
async fn large_stake_approval_with_unresolved_args_fails_closed() {
    // Documents the framework's safety property: when the resolver is
    // not wired in (the default in `ironclaw_hooks` standalone), a
    // NumericSum predicate dispatched against unresolved arguments
    // must NOT permit the call. Production wiring lives in
    // `ironclaw_reborn`; here we confirm the fail-closed posture.
    let extension = ExtensionId::new("polymarket-trader").expect("valid ext id");
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let entries = vec![large_stake_approval_manifest()];
    let (builder, _ids) = registrar
        .install(extension.clone(), "0.4.2", &entries, builder)
        .expect("installs");
    let dispatcher = builder.build_arc();

    let tenant = TenantId::new("alice").expect("valid tenant");
    let ctx = BeforeCapabilityHookContext::new(
        tenant,
        "polymarket.place_order".to_string(),
        [0u8; 32],
        ironclaw_hooks::points::SanitizedArguments::unresolved(),
        Some(extension),
    );
    let outcome = dispatcher.dispatch_before_capability(&ctx).await;
    assert!(
        !outcome.decision.permits(),
        "NumericSum against unresolved args must fail closed; got {:?}",
        outcome.decision.view()
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Hook 3 — PII-redaction warning (Trusted-tier Rust hook)
// ─────────────────────────────────────────────────────────────────────────

/// A Trusted-tier `before_prompt` hook that injects a trusted instruction
/// snippet reminding the model not to echo PII fields back in its output.
/// Trusted-tier because the snippet is *trusted instruction*, not user
/// content — only Builtin/Trusted hooks can mint trusted snippets
/// (Installed hooks are restricted to envelope-wrapped untrusted bodies).
struct PiiRedactionWarningHook {
    /// Instruction body kept short to stay within the snippet budget.
    instruction: &'static str,
}

impl PiiRedactionWarningHook {
    fn new() -> Self {
        Self {
            instruction: "If the user's message contains PII (email, phone, SSN, credit-card \
                          number, home address), do not repeat it back verbatim in your \
                          response. Acknowledge receipt without echoing the sensitive value.",
        }
    }
}

#[async_trait]
impl PrivilegedBeforePromptHook for PiiRedactionWarningHook {
    async fn evaluate(&self, ctx: &BeforePromptHookContext, sink: &mut dyn PrivilegedMutatorSink) {
        // Keep the snippet under the remaining budget; in practice a real
        // hook would have a configured ceiling and refuse to fire when
        // the budget is tight, since dropping a safety snippet silently
        // is worse than failing the dispatch.
        if (ctx.remaining_snippet_byte_budget as usize) < self.instruction.len() {
            // Caller will see no patches from this hook; the budget
            // shortage is the operator's signal to investigate.
            return;
        }
        // `add_trusted_snippet` is the privileged path — only the
        // `PrivilegedMutatorSink` exposes it. An Installed hook trying
        // to call this method would not compile.
        let _ = sink.add_trusted_snippet(self.instruction.to_string(), PatchOrdinalHint::NearTop);
    }
}

#[tokio::test]
async fn pii_redaction_warning_injects_trusted_snippet() {
    use ironclaw_hooks::identity::{ExtensionId as IdentExtensionId, HookId, HookVersion};

    let hook_id = HookId::derive(
        &IdentExtensionId::new("ironclaw-builtin").expect("valid IdentExtensionId in test"),
        "1.0.0",
        &HookLocalId::new("pii-redaction-warning").expect("valid HookLocalId in test"),
        HookVersion::ONE,
    );

    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .install_trusted_before_prompt(
            hook_id,
            HookPhase::Policy,
            Box::new(PiiRedactionWarningHook::new()),
        )
        .expect("trusted before_prompt installs")
        .build_arc();

    let tenant = TenantId::new("alice").expect("valid tenant");
    let ctx = BeforePromptHookContext::new(tenant, 8 * 1024); // 8 KiB remaining
    let outcome = dispatcher.dispatch_before_prompt(&ctx).await;

    assert_eq!(outcome.failures.len(), 0, "no failures expected");
    assert_eq!(
        outcome.patches.len(),
        1,
        "exactly one PII-redaction patch expected"
    );

    // The emitted patch must be a Trusted snippet near the top of the
    // ordering. This is the property a system author would assert
    // against — "my safety instruction got injected, with the trust
    // class I asked for, near the position I asked for."
    match outcome.patches[0].view() {
        HookPatchView::AddSnippet {
            body,
            ordinal_hint,
            trust_class,
            byte_count,
        } => {
            assert_eq!(trust_class, HookTrustClass::Trusted);
            assert_eq!(ordinal_hint, PatchOrdinalHint::NearTop);
            assert!(byte_count > 0);
            match body {
                SnippetBodyView::Trusted { text } => {
                    assert!(text.contains("PII"), "snippet text should mention PII");
                }
                SnippetBodyView::Enveloped { .. } => {
                    panic!("Trusted hook must produce a Trusted snippet, not Enveloped")
                }
            }
        }
        other => panic!("expected AddSnippet patch, got {other:?}"),
    }
}

#[tokio::test]
async fn pii_redaction_warning_skips_when_budget_too_small() {
    use ironclaw_hooks::identity::{ExtensionId as IdentExtensionId, HookId, HookVersion};

    let hook_id = HookId::derive(
        &IdentExtensionId::new("ironclaw-builtin").expect("valid IdentExtensionId in test"),
        "1.0.0",
        &HookLocalId::new("pii-redaction-warning").expect("valid HookLocalId in test"),
        HookVersion::ONE,
    );
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .install_trusted_before_prompt(
            hook_id,
            HookPhase::Policy,
            Box::new(PiiRedactionWarningHook::new()),
        )
        .expect("installs")
        .build_arc();

    let tenant = TenantId::new("alice").expect("valid tenant");
    // 16 bytes is way under the instruction length — the hook should
    // bow out gracefully and the dispatch should yield zero patches
    // and zero failures.
    let ctx = BeforePromptHookContext::new(tenant, 16);
    let outcome = dispatcher.dispatch_before_prompt(&ctx).await;

    assert_eq!(outcome.failures.len(), 0);
    assert_eq!(
        outcome.patches.len(),
        0,
        "hook should bow out cleanly when budget is too small"
    );
}
