//! Reborn integration test — synthetic `skill_activate` capability + skill
//! context injection (E-SKILL seam).
//!
//! A `greet` system skill is seeded by `skill_activation_tools()`. The model
//! explicitly activates it through the REAL local-dev synthetic capability
//! (`builtin.skill_activate`), and the test proves BOTH halves of the seam:
//! the capability dispatched and reported the skill activated (`count: 1`),
//! and its instructions reached a subsequent model request through the wired
//! `skill_context_source` (`assert_model_request_contains`).
//!
//! The user message omits the skill's `greet` keyword, so the injected
//! `GREET_SKILL_PROMPT_SENTINEL` can only originate from the explicit
//! `skill_activate` call, not keyword auto-activation.
//!
//! The second test below pins the OTHER half of E-SKILL: criteria-based
//! (keyword) auto-activation stays OFF on the coordinator path (product
//! decision, closed issue #5530).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
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
    harness
        .assert_tool_result_contains("\"activated\":[\"greet\"]")
        .await
        .expect("skill_activate reported the explicitly requested skill");

    // Model-invoked skill discovery: before activation the skill advertises
    // itself as a one-line `- name: description` listing entry (the first
    // model request is built before `skill_activate` runs).
    harness
        .assert_model_request_contains("- greet: greets the user warmly")
        .await
        .expect("discovered skill must appear as a one-line listing entry");

    // Half B: the activated skill's instructions reached a later model request
    // through the wired `skill_context_source`.
    harness
        .assert_model_request_contains("GREET_SKILL_PROMPT_SENTINEL")
        .await
        .expect("activated skill instructions must inject into a model request");
}

// INTENTIONAL: `SkillActivationMode::ActivationCriteria` (keyword/regex
// auto-activation) does not fire on the `TurnCoordinator`/agent-loop path —
// disabled on purpose (product decision, closed issue #5530). It only runs
// when `record_user_message` populates `take_message_for_run`, whose sole
// caller is the legacy `RebornRuntime::submit_user_turn`; the coordinator
// stack never records the message, so criteria selection stays inert here.
// The explicit `builtin.skill_activate` path (proven above) is supported.
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

    // Specific error check (not generic `is_err()`): `assert_model_request_contains`
    // has a second, unrelated `Err` path (JSON serialization failure of the
    // captured request) — asserting the exact "not found" message text rules
    // that out, so an infra-level failure can't masquerade as proof the
    // sentinel was absent.
    let err = harness
        .assert_model_request_contains("GREET_SKILL_PROMPT_SENTINEL")
        .await
        .expect_err(
            "criteria auto-activation is intentionally OFF on the coordinator path — \
             a keyword-matching message alone must not inject the skill prompt (#5530)",
        );
    assert!(
        err.to_string().starts_with("no model request contained"),
        "expected the intended \"not found\" assertion failure, got a different harness error: {err}"
    );

    // Listing-mode discovery (default `IRONCLAW_REBORN_SKILL_INJECTION=listing`):
    // the non-activated skill still ADVERTISES itself as a one-line
    // `- name: description` entry under the skill_activate header, so the
    // model can choose to activate it — its body stays out (asserted above).
    harness
        .assert_model_request_contains("- greet: greets the user warmly")
        .await
        .expect("non-activated skill must appear as a one-line listing entry");
    harness
        .assert_model_message_content_in_order(&[
            "Review the available skills below before answering.",
            "If a listed skill could help with any part of the task, call builtin.skill_activate with its exact name before continuing; its full instructions will then be loaded.",
            "Choose the smallest relevant set, with at most four active skills total per run; large skills may reduce that number.",
            "If activation reports an ambiguous name, do not guess; explain the conflict.",
            "Do not activate skills that are unrelated to the task.",
        ])
        .await
        .expect("listing header must require deliberate model-invoked skill selection");

    // And the explicit capability was never dispatched either.
    assert!(
        harness
            .assert_tool_invoked("builtin.skill_activate")
            .await
            .is_err(),
        "builtin.skill_activate must NOT have been invoked without an explicit call"
    );
}

#[tokio::test]
async fn coordinator_listing_uses_descriptor_order_without_criteria_input() {
    let group = RebornIntegrationGroup::skill_activation_tools()
        .await
        .expect("skill-activation group builds");
    let capability_harness = group
        .capability_harness()
        .expect("skill-activation group has a host-runtime capability harness");
    let harness = group
        .thread("conv-skill-listing-order")
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("thread builds");
    capability_harness
        .seed_user_skill_for_test(
            &harness.binding.tenant_id,
            &harness.binding.actor_user_id,
            "alpha",
            "alpha baseline skill",
            "ALPHA_SKILL_SENTINEL",
        )
        .expect("alpha user skill seeds");
    capability_harness
        .seed_user_skill_for_test(
            &harness.binding.tenant_id,
            &harness.binding.actor_user_id,
            "zulu",
            "zulu baseline skill",
            "ZULU_SKILL_SENTINEL",
        )
        .expect("zulu user skill seeds");

    harness
        .submit_turn("a request unrelated to the seeded activation keywords")
        .await
        .expect("turn completes");

    harness
        .assert_model_message_content_in_order(&[
            "- greet: greets the user warmly",
            "- alpha: alpha baseline skill",
            "- zulu: zulu baseline skill",
        ])
        .await
        .expect("coordinator listing must preserve source-then-name descriptor order");
}

#[tokio::test]
async fn unknown_skill_activation_is_a_noop_and_the_run_continues() {
    let group = RebornIntegrationGroup::skill_activation_tools()
        .await
        .expect("skill-activation group builds");
    let harness = group
        .thread("conv-skill-activate-unknown")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.skill_activate",
                json!({"names": ["missing-helper"]}),
            ),
            RebornScriptedReply::text("the requested skill was unavailable"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("use the missing helper")
        .await
        .expect("turn completes despite the missing skill");

    harness
        .assert_tool_invoked("builtin.skill_activate")
        .await
        .expect("skill_activate dispatched through the real capability");
    harness
        .assert_tool_result_contains("\"count\":0")
        .await
        .expect("unknown skill currently produces an empty activation result");
    harness
        .assert_tool_result_contains("\"activated\":[]")
        .await
        .expect("unknown skill must not activate another candidate");
    harness
        .assert_reply_contains("requested skill was unavailable")
        .await
        .expect("model received another turn after the no-op activation");
    let err = harness
        .assert_model_request_contains("GREET_SKILL_PROMPT_SENTINEL")
        .await
        .expect_err("a missing request must not inject a different skill");
    assert!(
        err.to_string().starts_with("no model request contained"),
        "expected the intended \"not found\" assertion failure, got a different harness error: {err}"
    );
}

#[tokio::test]
async fn installed_skill_is_listed_but_not_model_activatable() {
    let group = RebornIntegrationGroup::skill_activation_tools()
        .await
        .expect("skill-activation group builds");
    let capability_harness = group
        .capability_harness()
        .expect("skill-activation group has a host-runtime capability harness");
    let harness = group
        .thread("conv-skill-activate-installed")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.skill_activate",
                json!({"names": ["remote-helper"]}),
            ),
            RebornScriptedReply::text("the installed skill was unavailable"),
        ])
        .build()
        .await
        .expect("thread builds");
    capability_harness
        .seed_installed_user_skill_for_test(
            &harness.binding.tenant_id,
            &harness.binding.actor_user_id,
            "remote-helper",
            "a remotely installed helper",
            "REMOTE_INSTALLED_SKILL_SENTINEL",
        )
        .expect("installed user skill seeds");

    harness
        .submit_turn("use the remote helper")
        .await
        .expect("turn completes despite installed skill trust");

    harness
        .assert_model_request_contains("- remote-helper: a remotely installed helper")
        .await
        .expect("installed skill remains safely discoverable");
    harness
        .assert_tool_result_contains("\"count\":0")
        .await
        .expect("installed skill is not admitted by model-selected activation");
    harness
        .assert_tool_result_contains("\"activated\":[]")
        .await
        .expect("installed skill must not activate another candidate");
    let err = harness
        .assert_model_request_contains("REMOTE_INSTALLED_SKILL_SENTINEL")
        .await
        .expect_err("installed skill prompt body must remain hidden");
    assert!(
        err.to_string().starts_with("no model request contained"),
        "expected the intended \"not found\" assertion failure, got a different harness error: {err}"
    );
}

/// C-SYNTH failure route — `skill_activate` `ContextBudgetExceeded` is a
/// MODEL-VISIBLE `Failed` tool error (recoverable), not a terminal driver
/// error. An oversized system skill (~10k tokens, over
/// `DEFAULT_MAX_SKILL_CONTEXT_TOKENS = 4000`) drives the real selection path
/// (`reserve_skill_budget` → `ContextBudgetExceeded` → `Failed`).
#[test]
fn skill_activate_over_budget_surfaces_recoverable_failed() {
    run_async_test_with_stack(
        "skill_activate_over_budget_surfaces_recoverable_failed",
        || async {
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
                    RebornScriptedReply::tool_call(
                        "builtin.skill_activate",
                        json!({"names": ["bloat"]}),
                    ),
                    RebornScriptedReply::text("could not activate"),
                ])
                .build()
                .await
                .expect("thread builds");

            harness
                .submit_turn("activate the big one")
                .await
                .expect("turn completes despite the failed activation");

            // Model-visible Failed, not a terminal driver_unavailable.
            harness
                .assert_tool_error(ToolErrorClass::Failed, "skill context budget")
                .await
                .expect("over-budget activation surfaced as a recoverable Failed tool error");
            harness
                .assert_reply_contains("could not activate")
                .await
                .expect("run recovered and finalized");
            // Specific error check (not generic `is_err()`): `assert_model_request_contains`
            // has a second, unrelated `Err` path (JSON serialization failure of the
            // captured request) — asserting the exact "not found" message text rules
            // that out, so an infra-level failure can't masquerade as proof the
            // skill's instructions were absent.
            let err = harness
                .assert_model_request_contains("BLOAT_SKILL_FILLER")
                .await
                .expect_err("a failed activation must not inject the skill's instructions");
            assert!(
                err.to_string().starts_with("no model request contained"),
                "expected the intended \"not found\" assertion failure, got a different harness error: {err}"
            );
        },
    );
}

/// C-SYNTH `AmbiguousSkill` seeding arm — a skill name resolving to TWO
/// Trusted candidates (system-scoped + user-scoped, same name) drives the
/// real `validate_explicit_mentions_are_unambiguous` reject path
/// (`AmbiguousSkill` → `Failed(InvalidInput)`), distinct from the
/// `ContextBudgetExceeded` arm above. Proves neither candidate's instructions
/// leak into a later model request despite the name matching both.
#[test]
fn skill_activate_ambiguous_name_surfaces_recoverable_failed() {
    run_async_test_with_stack(
        "skill_activate_ambiguous_name_surfaces_recoverable_failed",
        || async {
            let group = RebornIntegrationGroup::skill_activation_tools()
                .await
                .expect("skill-activation group builds");
            let capability_harness = group
                .capability_harness()
                .expect("skill-activation group has a host-runtime capability harness");
            capability_harness
                .seed_system_skill_for_test(
                    "duplicate",
                    "a system-scoped skill",
                    "SYSTEM_DUPLICATE_SKILL_SENTINEL",
                )
                .expect("system-scoped duplicate skill seeds");

            let harness = group
                .thread("conv-skill-activate-ambiguous")
                .script([
                    RebornScriptedReply::tool_call(
                        "builtin.skill_activate",
                        json!({"names": ["duplicate"]}),
                    ),
                    RebornScriptedReply::text("that name is ambiguous"),
                ])
                .build()
                .await
                .expect("thread builds");
            // The user-scoped root is seeded under the SAME (tenant, actor) the built
            // thread's run resolves under (`harness.binding`) — only then does the
            // user `/skills` mount the run actually reads from contain this file.
            capability_harness
                .seed_user_skill_for_test(
                    &harness.binding.tenant_id,
                    &harness.binding.actor_user_id,
                    "duplicate",
                    "a user-scoped skill",
                    "USER_DUPLICATE_SKILL_SENTINEL",
                )
                .expect("user-scoped duplicate skill seeds");

            harness
                .submit_turn("activate the duplicate skill")
                .await
                .expect("turn completes despite the ambiguous activation");

            // Model-visible Failed, not a terminal driver_unavailable.
            harness
                .assert_tool_error(ToolErrorClass::Failed, "ambiguous skill")
                .await
                .expect("ambiguous skill name surfaced as a recoverable Failed tool error");
            harness
                .assert_reply_contains("that name is ambiguous")
                .await
                .expect("run recovered and finalized");
            harness
                .assert_model_message_content_in_order(&[
                    "- duplicate: a system-scoped skill",
                    "- greet: greets the user warmly",
                    "- duplicate: a user-scoped skill",
                ])
                .await
                .expect(
                    "the baseline listing exposes ambiguous bare names without qualified recovery identifiers",
                );
            // Neither candidate's instructions may leak into a later model request —
            // an ambiguous selection activates nothing.
            for sentinel in [
                "SYSTEM_DUPLICATE_SKILL_SENTINEL",
                "USER_DUPLICATE_SKILL_SENTINEL",
            ] {
                let err = harness
                    .assert_model_request_contains(sentinel)
                    .await
                    .expect_err(
                        "an ambiguous activation must not inject either candidate's instructions",
                    );
                assert!(
                    err.to_string().starts_with("no model request contained"),
                    "expected the intended \"not found\" assertion failure, got a different harness error: {err}"
                );
            }
        },
    );
}

fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
