//! Prompt-context windowing has no existing flat suite: greeting/safety cover
//! prompt construction, but not dropping old persisted transcript messages
//! from the final model-visible request.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_turns::run_profile::PromptContextTokenBudget;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

const OLD_MARKER: &str = "OLD-MARKER-ALPHA";
const NEW_MARKER: &str = "NEW-MARKER-OMEGA";

#[tokio::test]
async fn tiny_budget_drops_old_transcript_from_final_model_request() {
    let harness = budgeted_harness().await;
    submit_budget_turns(&harness).await;
    assert_prompt_budget_effect(&harness).await;
}

// Twin over a group runtime: pins the group-builder threading
// (`prompt_context_token_budget` → parts → factory → context port) that the
// flat harness above does not touch.
#[tokio::test]
async fn tiny_budget_drops_old_transcript_via_scoped_gateway() {
    let group = RebornIntegrationGroup::builder()
        .prompt_context_token_budget(PromptContextTokenBudget::new(80, 10, 0))
        .builtin_tools()
        .await
        .expect("group builds");
    let harness = group
        .thread("conv-budget-scoped")
        .script([
            RebornScriptedReply::text("turn one done"),
            RebornScriptedReply::text("turn two done"),
            RebornScriptedReply::text("turn three done"),
        ])
        .build()
        .await
        .expect("harness builds");
    submit_budget_turns(&harness).await;
    assert_prompt_budget_effect(&harness).await;
}

async fn budgeted_harness() -> RebornIntegrationHarness {
    RebornIntegrationHarness::test_default()
        .with_prompt_context_token_budget(PromptContextTokenBudget::new(80, 10, 0))
        .script([
            RebornScriptedReply::text("turn one done"),
            RebornScriptedReply::text("turn two done"),
            RebornScriptedReply::text("turn three done"),
        ])
        .build()
        .await
        .expect("harness builds")
}

fn old_turn() -> String {
    format!("{OLD_MARKER} {}", "old-prompt-padding ".repeat(18))
}

async fn submit_budget_turns(harness: &RebornIntegrationHarness) {
    harness
        .submit_turn(&old_turn())
        .await
        .expect("turn 1 completes");
    harness
        .submit_turn("middle turn keeps the thread moving")
        .await
        .expect("turn 2 completes");
    harness
        .submit_turn(NEW_MARKER)
        .await
        .expect("turn 3 completes");
}

async fn assert_prompt_budget_effect(harness: &RebornIntegrationHarness) {
    harness
        .assert_conversation_history_contains(OLD_MARKER)
        .await
        .expect("old turn persisted");
    harness
        .assert_last_model_request_contains(NEW_MARKER)
        .await
        .expect("final request is real");
    // Retention pin: the in-budget middle turn must survive the window, so a
    // corrupted budget that over-drops (e.g. keep-only-newest) fails here.
    harness
        .assert_last_model_request_contains("middle turn keeps the thread moving")
        .await
        .expect("in-budget middle turn retained");
    harness
        .assert_last_model_request_not_contains(OLD_MARKER)
        .await
        .expect("old turn dropped from prompt window");
}
