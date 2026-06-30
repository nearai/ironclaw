mod support;

use ironclaw_product_workflow::InboundTurnOutcome;
use ironclaw_reborn::planned_driver_factory::PLANNED_DEFAULT_PROFILE_ID;
use ironclaw_threads::MessageStatus;
use ironclaw_turns::TurnStatus;

use ironclaw_loop_support::HostManagedModelResponse;

use support::planned_agent_loop::{
    HarnessCapabilityConfig, HostRuntimeCapabilityConfig, ProductLiveAgentLoopHarness,
    ProductLiveAgentLoopHarnessConfig, capability_call_response,
};

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

#[tokio::test]
async fn ported_product_live_fixture_uses_shared_harness_for_no_profile_reply() {
    let harness = ProductLiveAgentLoopHarness::new(ProductLiveAgentLoopHarnessConfig {
        assistant_reply: "planned product reply".to_string(),
        user_id: "user:product-live".to_string(),
        thread_id: "thread:product-live".to_string(),
        ..ProductLiveAgentLoopHarnessConfig::default()
    })
    .await;
    let envelope = harness.user_message("planned-product-live-ported", "hello world");

    let outcome = harness
        .accept_user_message(&envelope)
        .await
        .expect("ported product live submit");
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
            && message.content.as_deref() == Some("planned product reply")
    }));

    harness.shutdown().await;
}

#[tokio::test]
async fn ported_product_live_fixture_cancels_through_public_turn_path() {
    let harness = ProductLiveAgentLoopHarness::new(ProductLiveAgentLoopHarnessConfig {
        assistant_reply: "reply after cancel".to_string(),
        user_id: "user:product-live".to_string(),
        thread_id: "thread:product-live-cancel-live".to_string(),
        pause_model_until_released: true,
        ..ProductLiveAgentLoopHarnessConfig::default()
    })
    .await;
    let envelope = harness.user_message("planned-product-live-cancel-ported", "hello world");

    let outcome = harness
        .accept_user_message(&envelope)
        .await
        .expect("ported product live submit");
    let InboundTurnOutcome::Submitted {
        submitted_run_id, ..
    } = outcome
    else {
        panic!("expected submitted outcome, got {outcome:?}");
    };

    harness.wait_for_model_request_count(1).await;
    assert_eq!(
        harness.cancel_run(submitted_run_id).await,
        TurnStatus::CancelRequested
    );
    harness
        .wait_for_cancellation_observed(submitted_run_id)
        .await;

    harness.release_model();
    let state = harness.wait_for_terminal(submitted_run_id).await;

    assert_eq!(state.status, TurnStatus::Cancelled);
    let history = harness.thread_history().await;
    assert!(history.iter().any(|message| {
        message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(submitted_run_id.to_string().as_str())
            && message.content.as_deref() == Some("reply after cancel")
    }));

    harness.shutdown().await;
}

#[tokio::test]
async fn product_live_harness_invokes_capability_then_persists_final_reply() {
    let harness = ProductLiveAgentLoopHarness::new(ProductLiveAgentLoopHarnessConfig {
        assistant_reply: "unused fallback".to_string(),
        model_responses: vec![
            capability_call_response("harness.echo", "input:harness-echo-1"),
            HostManagedModelResponse::assistant_reply("final reply after capability"),
        ],
        capability: Some(HarnessCapabilityConfig {
            capability_id: "harness.echo".to_string(),
            result_ref: "result:harness-echo-1".to_string(),
            safe_summary: "echo completed".to_string(),
            terminate_hint: false,
        }),
        ..ProductLiveAgentLoopHarnessConfig::default()
    })
    .await;
    let envelope = harness.user_message("planned-harness-capability", "use echo");

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
    assert_eq!(harness.model_requests().len(), 2);
    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id.as_str(), "harness.echo");
    assert_eq!(invocations[0].input_ref.as_str(), "input:harness-echo-1");

    let history = harness.thread_history().await;
    assert!(history.iter().any(|message| {
        message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(submitted_run_id.to_string().as_str())
            && message.content.as_deref() == Some("final reply after capability")
    }));

    harness.shutdown().await;
}

#[tokio::test]
async fn product_live_harness_invokes_builtin_echo_through_host_runtime() {
    let harness = ProductLiveAgentLoopHarness::new(ProductLiveAgentLoopHarnessConfig {
        assistant_reply: "final reply after builtin echo".to_string(),
        host_runtime_capability: Some(HostRuntimeCapabilityConfig {
            capability_id: ironclaw_host_runtime::ECHO_CAPABILITY_ID.to_string(),
            input: serde_json::json!({ "message": "hello from builtin echo" }),
        }),
        ..ProductLiveAgentLoopHarnessConfig::default()
    })
    .await;
    let envelope = harness.user_message("planned-harness-builtin-echo", "use builtin echo");

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
    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0].capability_id.as_str(),
        ironclaw_host_runtime::ECHO_CAPABILITY_ID
    );
    assert_eq!(
        harness.capability_results(),
        vec![serde_json::json!("hello from builtin echo")]
    );

    let history = harness.thread_history().await;
    assert!(history.iter().any(|message| {
        message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(submitted_run_id.to_string().as_str())
            && message.content.as_deref() == Some("final reply after builtin echo")
    }));

    harness.shutdown().await;
}
