//! QA use-case coverage: "In WebUI, ask IronClaw 'connect to <service>'.
//! Go through the auth flow. Expected result: <service> is connected."
//!
//! Models the manual QA connect flows on the Reborn binary-E2E harness:
//! the user asks to connect a service, the agent stages the connection
//! credential behind a local-dev approval gate (the "go through the
//! flow" step), the tester approves the gate, and the turn completes
//! with a "connected" reply. External OAuth/BotFather traffic is not
//! exercised; the gate raise/resume path and credential persistence are.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::WRITE_FILE_CAPABILITY_ID;
use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

struct ConnectFlowCase {
    room: &'static str,
    event_id: &'static str,
    service_slug: &'static str,
    user_request: &'static str,
    connected_reply: &'static str,
}

async fn run_connect_flow(case: ConnectFlowCase) {
    let write_file = CapabilityId::new(WRITE_FILE_CAPABILITY_ID).expect("valid capability id");
    let credential_path = format!("/workspace/connections/{}.json", case.service_slug);
    let credential_content = format!(
        r#"{{"service":"{}","credential":"qa-test-credential"}}"#,
        case.service_slug
    );
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                write_file.clone(),
                format!("call_connect_{}", case.service_slug),
                serde_json::json!({
                    "path": credential_path,
                    "content": credential_content,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(case.connected_reply),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_file_capabilities_requiring_approval(
            case.room,
            model_gateway,
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(case.event_id, case.user_request)
        .await
        .expect("submit connect request");

    // "Go through the auth flow": the connection blocks on a local-dev
    // approval gate that the tester (QA) must resolve before the
    // credential is staged.
    let blocked = harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedApproval)
        .await
        .expect("connect flow blocks on auth gate");
    let gate_ref = blocked.gate_ref.expect("blocked connect gate ref");
    assert!(
        gate_ref.as_str().starts_with("gate:approval-"),
        "expected local-dev approval gate ref, got {gate_ref:?}"
    );

    let resolved = harness
        .approve_and_resume_local_dev_gate(submitted.run_id)
        .await
        .expect("approve connect auth gate");
    assert_eq!(resolved, gate_ref);

    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("connect flow completes after approval");
    harness
        .assert_final_reply(case.connected_reply)
        .await
        .expect("connected reply");

    let invocations = harness.capability_invocations();
    assert_eq!(
        invocations.len(),
        2,
        "connect credential staging should run once blocked and once resumed"
    );
    assert_eq!(invocations[0].capability_id, write_file);
    assert!(invocations[0].approval_resume.is_none());
    assert_eq!(invocations[1].capability_id, write_file);
    assert!(
        invocations[1].approval_resume.is_some(),
        "approved gate must resume the original blocked connect call"
    );

    // The staged connection credential must be persisted after approval.
    let staged_path = harness
        .host_workspace_file_path(&format!("connections/{}.json", case.service_slug))
        .expect("staged credential path");
    assert_eq!(
        std::fs::read_to_string(staged_path).expect("staged credential file"),
        credential_content
    );

    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::GateBlocked { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_connect_gmail_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-gmail",
        event_id: "event-qa-connect-gmail",
        service_slug: "gmail",
        user_request: "connect to Gmail",
        connected_reply: "Gmail is connected",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_google_calendar_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-google-calendar",
        event_id: "event-qa-connect-google-calendar",
        service_slug: "google_calendar",
        user_request: "connect to Google Calendar",
        connected_reply: "Google Calendar is connected",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_google_drive_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-google-drive",
        event_id: "event-qa-connect-google-drive",
        service_slug: "google_drive",
        user_request: "connect to Google Drive",
        connected_reply: "Google Drive is connected",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_google_sheets_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-google-sheets",
        event_id: "event-qa-connect-google-sheets",
        service_slug: "google_sheets",
        user_request: "connect to Google Sheets",
        connected_reply: "Google Sheets is connected",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_slack_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-slack",
        event_id: "event-qa-connect-slack",
        service_slug: "slack",
        user_request: "connect to Slack",
        connected_reply: "Slack is connected",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_slack_channel_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-slack-channel",
        event_id: "event-qa-connect-slack-channel",
        service_slug: "slack_product_channel",
        user_request: "connect to Slack, using channel #product",
        connected_reply: "Slack is connected to channel #product",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_github_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-github",
        event_id: "event-qa-connect-github",
        service_slug: "github",
        user_request: "connect to GitHub",
        connected_reply: "GitHub is connected",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_connect_telegram_auth_flow() {
    run_connect_flow(ConnectFlowCase {
        room: "room-qa-connect-telegram",
        event_id: "event-qa-connect-telegram",
        service_slug: "telegram",
        user_request: "connect to Telegram",
        connected_reply: "Telegram is connected",
    })
    .await;
}
