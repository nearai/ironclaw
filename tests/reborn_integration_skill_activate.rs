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

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::assertions::ToolErrorClass;
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

/// C-SYNTH failure route — `skill_activate` `ContextBudgetExceeded` is a
/// MODEL-VISIBLE `Failed` tool error (recoverable), not a terminal driver
/// error.
///
/// An oversized system skill (prompt ≈ 10k tokens at the ~4-bytes-per-token
/// estimate, over the `DEFAULT_MAX_SKILL_CONTEXT_TOKENS = 4000` budget) is
/// seeded through the SAME `seed_system_skill_for_test` seam the group
/// constructor uses for `greet` — no new harness wiring. Activating it drives
/// the real selection path (`reserve_skill_budget` →
/// `SkillActivationSelectionError::ContextBudgetExceeded` →
/// `CapabilityOutcome::Failed`, skill_activation.rs `selection_outcome`), and
/// the run completes with the model seeing the failure — the
/// synthetic-capability Failed-routing proof for this cap.
#[tokio::test]
async fn skill_activate_over_budget_surfaces_recoverable_failed() {
    let group = RebornIntegrationGroup::skill_activation_tools()
        .await
        .expect("skill-activation group builds");
    let capability_harness = group
        .capability_harness()
        .expect("skill-activation group has a host-runtime capability harness");
    let oversized_prompt = "BLOAT_SKILL_FILLER ".repeat(2200); // ~41.8k chars ≈ 10k tokens > 4000 budget
    capability_harness
        .seed_system_skill_for_test("bloat", "an oversized skill", &oversized_prompt)
        .expect("oversized system skill seeds");

    let harness = group
        .thread("conv-skill-activate-over-budget")
        .script([
            RebornScriptedReply::tool_call("builtin.skill_activate", json!({"names": ["bloat"]})),
            RebornScriptedReply::text("could not activate"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("activate the big one")
        .await
        .expect("turn completes despite the failed activation");

    // The budget failure surfaced as a model-visible Failed tool error carrying
    // the budget summary — not a terminal driver_unavailable.
    harness
        .assert_tool_error(ToolErrorClass::Failed, "skill context budget")
        .await
        .expect("over-budget activation surfaced as a recoverable Failed tool error");
    harness
        .assert_reply_contains("could not activate")
        .await
        .expect("run recovered and finalized");
    // The oversized skill's instructions must NOT have been injected.
    assert!(
        harness
            .assert_model_request_contains("BLOAT_SKILL_FILLER")
            .await
            .is_err(),
        "a failed activation must not inject the skill's instructions"
    );
}
