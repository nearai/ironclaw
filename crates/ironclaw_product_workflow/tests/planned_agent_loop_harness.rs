mod support;

use ironclaw_product_workflow::InboundTurnOutcome;
use ironclaw_reborn::planned_driver_factory::PLANNED_DEFAULT_PROFILE_ID;
use ironclaw_threads::MessageStatus;
use ironclaw_turns::TurnStatus;

use support::planned_agent_loop::{ProductLiveAgentLoopHarness, ProductLiveAgentLoopHarnessConfig};

#[tokio::test]
async fn product_live_harness_runs_planned_loop_and_persists_reply() {
    let harness = ProductLiveAgentLoopHarness::new(ProductLiveAgentLoopHarnessConfig {
        assistant_reply: "hello from planned loop".to_string(),
        ..ProductLiveAgentLoopHarnessConfig::default()
    })
    .await;
    let envelope = harness.user_message("planned-harness-basic", "hello world");

    let outcome = harness
        .accept_user_message(&envelope)
        .await
        .expect("harness inbound turn should submit");
    let InboundTurnOutcome::Submitted {
        submitted_run_id, ..
    } = outcome
    else {
        panic!("expected submitted outcome, got {outcome:?}");
    };
    let state = harness.wait_for_terminal(submitted_run_id).await;

    assert_eq!(state.status, TurnStatus::Completed);
    assert_eq!(
        state.resolved_run_profile_id.as_str(),
        PLANNED_DEFAULT_PROFILE_ID
    );
    assert_eq!(harness.model_requests().len(), 1);

    let history = harness.thread_history().await;
    assert!(history.iter().any(|message| {
        message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(submitted_run_id.to_string().as_str())
            && message.content.as_deref() == Some("hello from planned loop")
    }));

    harness.shutdown().await;
}
