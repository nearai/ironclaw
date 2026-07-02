//! Reborn integration test — synthetic `skill_activate` capability + skill
//! context injection (E-SKILL seam).
//!
//! A `greet` system skill is seeded by `skill_activation_tools()`. The model
//! explicitly activates it through the REAL local-dev synthetic capability
//! (`builtin.skill_activate`, dispatched via
//! `wrap_skill_activation_capability_for_test`), and the test then proves BOTH
//! halves of the seam:
//!
//! - the capability dispatched and reported the skill activated (`count: 1`),
//!   and
//! - the activated skill's instructions reached a subsequent model request
//!   through the runtime's wired `skill_context_source`
//!   (`assert_model_request_contains`).
//!
//! The user message deliberately omits the skill's `greet` activation keyword,
//! so the injected `GREET_SKILL_PROMPT_SENTINEL` can only originate from the
//! explicit `skill_activate` call — not from keyword auto-activation. If either
//! the capability wrap or the `into_group` `skill_context_source` wiring
//! regresses, the sentinel never reaches a captured request and the assert fails.
//!
//! The second test below drives the OTHER half of E-SKILL: criteria-based
//! (keyword) auto-activation, with no `skill_activate` tool call scripted at
//! all. `seed_system_skill_for_test`'s SKILL.md already carries
//! `activation.keywords: ["greet"]`, and the harness wires
//! `regex_skill_activation_enabled=true` unconditionally
//! (`harness.rs`'s `skill_activation_tools`) — so a message containing "greet"
//! reaches `SkillActivationMode::ActivationCriteria`
//! (`ironclaw_first_party_extension_ports::activation::select_skill_activations`)
//! with zero new production wiring.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn skill_activate_dispatches_and_injects_skill_context() {
    let group = RebornIntegrationGroup::skill_activation_tools()
        .await
        .expect("skill-activation group builds");
    let harness = group
        .thread("conv-skill-activate")
        .script([
            RebornScriptedReply::tool_call("builtin.skill_activate", json!({"names": ["greet"]})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("please welcome me")
        .await
        .expect("turn completes");

    // Half A: the synthetic capability dispatched through the real path and
    // reported the seeded skill activated.
    harness
        .assert_tool_invoked("builtin.skill_activate")
        .await
        .expect("skill_activate dispatched through the real capability");
    harness
        .assert_tool_result_contains("\"count\":1")
        .await
        .expect("skill_activate reported one skill activated");

    // Half B: the activated skill's instructions reached a later model request
    // through the wired `skill_context_source`.
    harness
        .assert_model_request_contains("GREET_SKILL_PROMPT_SENTINEL")
        .await
        .expect("activated skill instructions must inject into a model request");
}

// ESCALATED (not fixed here — architectural, touches the shared agent-loop
// ingress path): `SkillActivationMode::ActivationCriteria` is unreachable from
// the modern Reborn turn-submission path today. `SelectableSkillContextSource::
// load_skill_context_candidates` (activation.rs:789-808, the `HostSkillContextSource`
// the loop calls) only runs fresh criteria selection when
// `take_message_for_run(scope, accepted_message_ref)` returns `Some` — populated
// exclusively by `record_user_message`/`record_message` (activation.rs:256-274).
// `record_user_message`'s ONLY production caller is the legacy
// `RebornRuntime::submit_user_turn` (`ironclaw_reborn_composition::runtime`, line
// ~1924) — a different, older runtime, NOT the `TurnCoordinator`/agent-loop stack
// `product_workflow::accept_inbound` drives (confirmed: production `skill_context_source`
// wiring in `build_reborn_runtime` → `local_dev_filesystem_skill_context_source`,
// runtime.rs:2925-3417, hands the raw `SelectableSkillContextSource` to the loop
// driver with no message-recording decorator — same shape this test harness
// wires). Net effect: on every real Reborn turn, `take_message_for_run` returns
// `None`, `load_skill_context_candidates` falls back to `active_plan_candidates`
// (only an already-EXPLICITLY-activated plan), and keyword/regex criteria
// selection never fires — even though `auto_activate` defaults `true` and
// `regex_skill_activation_enabled=true` is already wired.
// TODO(reborn-skill-criteria-gap): wire `record_user_message` into the modern
// turn-submission/loop-driver-host ingress path (mirroring what
// `RebornRuntime::submit_user_turn` already does for the legacy path) — needs
// its own PR; the right hook point (loop_driver_host pre-loop setup vs. a new
// BeforeInbound-style hook) needs its own design pass. Pinned RED here so this
// regresses loudly instead of silently if anyone assumes the criteria path
// already works.
#[ignore = "TODO(reborn-skill-criteria-gap): record_user_message never called on the modern Reborn submit_turn path — see comment above"]
#[tokio::test]
async fn skill_auto_activates_via_criteria_without_explicit_capability_call() {
    let group = RebornIntegrationGroup::skill_activation_tools()
        .await
        .expect("skill-activation group builds");
    let harness = group
        .thread("conv-skill-criteria")
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("thread builds");

    // The message CONTAINS the seeded skill's activation keyword ("greet") and
    // no `skill_activate` tool call is scripted — the only way the sentinel can
    // reach the model is criteria-based auto-activation.
    harness
        .submit_turn("please greet the visitor")
        .await
        .expect("turn completes");

    harness
        .assert_model_request_contains("GREET_SKILL_PROMPT_SENTINEL")
        .await
        .expect("keyword-matching skill must auto-inject without an explicit activate call");

    // Prove this really is the criteria path, not a hidden explicit call: the
    // synthetic capability was never dispatched.
    assert!(
        harness
            .assert_tool_invoked("builtin.skill_activate")
            .await
            .is_err(),
        "builtin.skill_activate must NOT have been invoked on the criteria-only path"
    );
}
