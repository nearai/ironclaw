#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "support/ironclaw_parity_qa/mod.rs"]
mod parity_qa_support;
mod support;

use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use parity_qa_support::binary_e2e::{IronClawBinaryE2EHarness, assert_milestone_order};

#[tokio::test]
async fn ironclaw_response_order_parity() {
    let mut harness =
        IronClawBinaryE2EHarness::reply_only("room-response-order", "ordered final reply")
            .await
            .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-response-order", "check ordering")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("ordered final reply")
        .await
        .expect("final reply");

    let milestones = harness.milestones();
    assert_milestone_order(
        &milestones,
        |kind| matches!(kind, LoopHostMilestoneKind::ModelCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );

    harness.shutdown().await;
}
