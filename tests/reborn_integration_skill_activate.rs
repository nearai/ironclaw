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

// INTENTIONAL: `SkillActivationMode::ActivationCriteria` (keyword/regex
// auto-activation) does NOT fire on the modern `TurnCoordinator`/agent-loop
// path — auto-activation is disabled on purpose (product decision, see closed
// issue #5530). The explicit `builtin.skill_activate` capability path is the
// supported mechanism and is what this binary covers. Mechanically: criteria
// selection only runs when `take_message_for_run` returns `Some`, populated by
// `record_user_message`, whose sole production caller is the legacy
// `RebornRuntime::submit_user_turn` — the coordinator stack never records the
// message, so criteria selection stays inert there by design. This test pins
// that intentional OFF state: a keyword-matching message alone must NOT inject
// the skill.
#[tokio::test]
async fn skill_criteria_auto_activation_stays_off_on_coordinator_path() {
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
    // no `skill_activate` tool call is scripted — if criteria auto-activation
    // ever silently turned on for the coordinator path, the sentinel would
    // reach the model and this test would go RED.
    harness
        .submit_turn("please greet the visitor")
        .await
        .expect("turn completes");

    assert!(
        harness
            .assert_model_request_contains("GREET_SKILL_PROMPT_SENTINEL")
            .await
            .is_err(),
        "criteria auto-activation is intentionally OFF on the coordinator path — \
         a keyword-matching message alone must not inject the skill prompt (#5530)"
    );

    // And the explicit capability was never dispatched either.
    assert!(
        harness
            .assert_tool_invoked("builtin.skill_activate")
            .await
            .is_err(),
        "builtin.skill_activate must NOT have been invoked without an explicit call"
    );
}
