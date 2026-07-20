use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::{Method, StatusCode};
use ironclaw_auth::{
    AuthFlowOutcome, AuthFlowState, AuthResolved, OAuthCallbackState, OAuthCallbackStateKind,
};
use ironclaw_product_workflow::{
    AuthResolutionDispatchOutcome, ProductAuthTurnGateResumeDispatcher,
};
use ironclaw_turns::{GetRunStateRequest, TurnStatus};
use serde_json::json;
use std::time::Duration;
use url::Url;

const QA_SLACK_TEAM: &str = "T-QA-WORKSPACE";
const SLACK_CALLBACK_PATH: &str = "/api/reborn/product-auth/oauth/slack_personal/callback";

/// QA regression journey (2026-07-18): a paired Telegram user asked what
/// other people were saying, the model selected Slack, and the resulting
/// authorization URL opened Slack in the browser's unrelated active
/// workspace. Slack rejected the undistributed QA app with
/// `invalid_team_for_non_distributed_app`. Closing/canceling that popup then
/// left the Telegram thread parked on `BlockedAuth`.
///
/// This journey pins the complete user contract at production seams:
///
/// 1. the Telegram DM installs and activates Slack through the real lifecycle
///    capabilities and receives the real blocked-auth prompt;
/// 2. the Slack authorization URL carries exactly the configured workspace as
///    `team` (plus the expected client and callback, never the client secret);
/// 3. the user denies authorization through the real public OAuth callback
///    route, which renders the sanitized popup failure page and durably records
///    `Resolved(ProviderDenied)` with its resolution-delivery marker;
/// 4. the exact `BlockedAuth` run resumes with a denied gate outcome, completes,
///    and delivers the model's recovery response back to Telegram;
/// 5. a new DM on the same paired conversation receives a normal reply, proving
///    the thread is no longer held by the failed authorization.
#[tokio::test]
async fn telegram_dm_slack_oauth_targets_workspace_and_popup_cancel_resumes_thread() {
    let stack = build_journey_stack_with_slack_oauth(
        [
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text(
                "Slack authorization was canceled, so I continued without it.",
            ),
            RebornScriptedReply::text("Yes — this Telegram thread is still active."),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
        ],
        QA_SLACK_TEAM,
    )
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    let request_text = "connect Slack and tell me what people are saying";
    // Do not drain the immediate-ACK task yet: in production the webhook
    // returns immediately and the delivery loop remains alive while the user
    // acts in the popup. Draining here would artificially wait for that loop's
    // parked-auth timeout before this test could send the callback.
    let status = stack
        .webhook_dm_without_drain(&secret, 2, request_text)
        .await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");

    let prompt = stack
        .wait_for_dm_send(|text| text.contains("slack.com/oauth/v2/authorize"))
        .await
        .expect("Slack activation DMs its actionable authorization link");
    let prompt_text = prompt["text"].as_str().expect("auth prompt text");
    let authorization_url = prompt_text
        .split_whitespace()
        .find(|part| part.starts_with("https://slack.com/oauth/v2/authorize?"))
        .expect("auth prompt contains a standalone Slack authorization URL");
    let authorization_url = Url::parse(authorization_url).expect("Slack authorization URL parses");
    let query: Vec<(String, String)> = authorization_url
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let values = |name: &str| {
        query
            .iter()
            .filter_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
            .collect::<Vec<_>>()
    };
    assert_eq!(values("team"), vec![QA_SLACK_TEAM]);
    assert_eq!(values("client_id"), vec!["journey-slack-client"]);
    assert_eq!(
        values("redirect_uri"),
        vec!["https://tg-journey.example/api/reborn/product-auth/oauth/slack_personal/callback"]
    );
    assert_eq!(values("response_type"), vec!["code"]);
    assert_eq!(values("code_challenge_method"), vec!["S256"]);
    assert!(
        !values("user_scope").is_empty(),
        "Slack user scopes are present"
    );
    assert!(
        !authorization_url.as_str().contains("journey-slack-secret"),
        "the OAuth client secret must never enter the authorization URL"
    );

    let state = values("state")
        .into_iter()
        .next()
        .expect("Slack authorization URL carries opaque state")
        .to_string();
    let callback_state = OAuthCallbackState::decode(OAuthCallbackStateKind::SLACK_PERSONAL, &state)
        .expect("Slack callback state decodes");
    let flow_id = callback_state.flow_id();
    let flow_scope = callback_state.scope().clone();
    let (turn_scope, run_id) = stack.run_for_dm_message(request_text).await;
    wait_for_run_status(
        &stack.runtime.webui_turn_coordinator_for_test(),
        &turn_scope,
        run_id,
        TurnStatus::BlockedAuth,
    )
    .await
    .expect("the exact Telegram run is parked on its Slack auth gate");

    // The user clicks Cancel in Slack's popup. Drive that provider redirect
    // through the real public callback route with a browser Accept header.
    let encoded_state = url::form_urlencoded::byte_serialize(state.as_bytes()).collect::<String>();
    let callback_path = format!("{SLACK_CALLBACK_PATH}?state={encoded_state}&error=access_denied");
    let (callback_status, callback_body) = call_route(
        stack
            .oauth_callback_public
            .clone()
            .expect("Slack journey exposes the public OAuth callback"),
        Method::GET,
        &callback_path,
        None,
        &[("accept", "text/html,application/xhtml+xml")],
        None,
    )
    .await;
    assert_eq!(callback_status, StatusCode::BAD_REQUEST);
    let callback_html = callback_body.as_str().expect("callback renders HTML");
    assert!(callback_html.contains("Authorization failed"));
    assert!(callback_html.contains("BroadcastChannel"));
    assert!(callback_html.contains(&flow_id.to_string()));
    assert!(
        !callback_html.contains(&state),
        "opaque state stays out of the failure page"
    );
    assert!(
        !callback_html.contains("access_denied"),
        "raw provider errors stay out of the failure page"
    );

    let product_auth = stack
        .webui
        .product_auth
        .as_ref()
        .expect("journey runtime exposes product auth");
    let mut failed_flow = None;
    for _ in 0..400 {
        let flow = product_auth
            .flow_manager()
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("read denied Slack flow")
            .expect("denied Slack flow remains durable");
        if flow.state == AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied)
            && flow.resolution_delivered_at.is_some()
        {
            failed_flow = Some(flow);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let failed_flow = failed_flow.expect(
        "provider denial durably fails the flow and acknowledges its continuation within 10s",
    );
    assert_eq!(
        failed_flow.state,
        AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied)
    );
    let old_resolution = AuthResolved {
        flow_id: failed_flow.id,
        scope: failed_flow.scope.clone(),
        continuation: failed_flow.continuation.clone(),
        provider: failed_flow.provider.clone(),
        outcome: AuthFlowOutcome::ProviderDenied,
        resolved_at: failed_flow.updated_at,
    };

    wait_for_run_status(
        &stack.runtime.webui_turn_coordinator_for_test(),
        &turn_scope,
        run_id,
        TurnStatus::Completed,
    )
    .await
    .expect("provider denial resumes and completes the exact blocked run");
    stack
        .wait_for_dm_send(|text| text.contains("continued without it"))
        .await
        .expect("the resumed run explains the canceled Slack authorization in Telegram");
    stack.drain_webhook_tasks().await;

    let sends_before_follow_up = stack.network.request_bodies_for("/sendMessage").len();
    let status = stack.webhook_dm(&secret, 3, "are you still there?").await;
    assert_eq!(status, StatusCode::OK, "follow-up DM webhook acks 200");
    let follow_up = stack
        .wait_for_dm_send(|text| text.contains("Telegram thread is still active"))
        .await
        .expect("the same Telegram thread accepts a normal follow-up turn");
    assert_eq!(follow_up["chat_id"], TG_CHAT_ID);
    assert!(
        stack.network.request_bodies_for("/sendMessage").len() > sends_before_follow_up,
        "the normal reply is attributable to the post-cancel follow-up"
    );
    assert_eq!(stack.model_trace.calls(), 4);

    // A later user request creates a distinct, newer Slack auth gate. Delivering
    // the old exact terminal event again through the single resolution seam must
    // return its typed no-op result and cannot release or cancel this newer gate.
    let retry_text = "please try connecting Slack again";
    let retry_baseline = stack.network.request_bodies_for("/sendMessage").len();
    let status = stack.webhook_dm_without_drain(&secret, 4, retry_text).await;
    assert_eq!(status, StatusCode::OK, "retry DM webhook acks 200");
    let retry_prompt = stack
        .wait_for_dm_send_after(retry_baseline, |text| {
            text.contains("slack.com/oauth/v2/authorize")
        })
        .await
        .expect("the retry creates a newer Slack authorization gate");
    let retry_url = retry_prompt["text"]
        .as_str()
        .and_then(|text| {
            text.split_whitespace()
                .find(|part| part.starts_with("https://slack.com/oauth/v2/authorize?"))
        })
        .and_then(|part| Url::parse(part).ok())
        .expect("retry Slack authorization URL parses");
    let retry_state = retry_url
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("retry Slack authorization URL carries opaque state");
    let retry_callback_state =
        OAuthCallbackState::decode(OAuthCallbackStateKind::SLACK_PERSONAL, &retry_state)
            .expect("retry Slack callback state decodes");
    assert_ne!(
        retry_callback_state.flow_id(),
        flow_id,
        "the later request owns a distinct auth flow"
    );
    let (retry_turn_scope, retry_run_id) = stack.run_for_dm_message(retry_text).await;
    wait_for_run_status(
        &stack.runtime.webui_turn_coordinator_for_test(),
        &retry_turn_scope,
        retry_run_id,
        TurnStatus::BlockedAuth,
    )
    .await
    .expect("the newer Telegram run is parked on its own Slack auth gate");
    let coordinator = stack.runtime.webui_turn_coordinator_for_test();
    let retry_gate_ref = coordinator
        .get_run_state(GetRunStateRequest {
            scope: retry_turn_scope.clone(),
            run_id: retry_run_id,
        })
        .await
        .expect("newer run state is readable")
        .gate_ref
        .expect("newer run carries an auth gate ref");
    let dispatch = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
        .dispatch_auth_resolved(old_resolution)
        .await
        .expect("stale resolution delivery is an idempotent success");
    assert_eq!(dispatch, AuthResolutionDispatchOutcome::Ignored);
    for _ in 0..40 {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: retry_turn_scope.clone(),
                run_id: retry_run_id,
            })
            .await
            .expect("newer run state remains readable");
        assert_eq!(state.status, TurnStatus::BlockedAuth);
        assert_eq!(
            state.gate_ref.as_ref(),
            Some(&retry_gate_ref),
            "redelivering the old terminal result cannot replace the newer exact gate"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(stack.model_trace.calls(), 5);

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
