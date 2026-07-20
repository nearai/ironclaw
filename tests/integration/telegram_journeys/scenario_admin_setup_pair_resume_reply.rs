use super::harness::*;
use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_turns::{TurnRunId, TurnScope};

use super::reborn_support;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::Method;
use axum::http::StatusCode;
use ironclaw_turns::TurnStatus;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn telegram_whole_journey_setup_pair_resume_and_dm_reply() {
    // Turn A (WebChat): install + activate telegram; activation parks on the
    // pairing gate and, once resumed, the model reacts to the completed
    // activation with the final text. Turn B (Telegram DM): plain reply.
    let stack = build_journey_stack([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "telegram"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "telegram"}),
        ),
        RebornScriptedReply::text("telegram is ready"),
        RebornScriptedReply::text("hello from your ironclaw bot"),
    ])
    .await;
    let JourneyStack {
        _root,
        network,
        runtime,
        mounts,
        webui,
        caller,
    } = stack;
    let protected = mounts.protected_routes();

    // ── Seam 1: admin save → getMe + setWebhook at the network boundary ────
    let (status, body) = call_route(
        protected.router.clone(),
        Method::PUT,
        "/api/webchat/v2/channels/telegram/setup",
        Some(caller.clone()),
        &[],
        Some(json!({"bot_token": BOT_TOKEN})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin save response: {body}");

    let get_me_urls: Vec<String> = network
        .requests()
        .iter()
        .filter(|request| request.url.ends_with("/getMe"))
        .map(|request| request.url.clone())
        .collect();
    assert_eq!(get_me_urls.len(), 1, "save validates the token via getMe");
    assert!(
        get_me_urls[0].contains(&format!("/bot{BOT_TOKEN}/")),
        "the dispatched getMe URL carries the SUBSTITUTED token (never the placeholder): {}",
        get_me_urls[0]
    );
    let set_webhook_bodies = network.request_bodies_for("/setWebhook");
    assert_eq!(set_webhook_bodies.len(), 1, "save registers the webhook");
    assert_eq!(
        set_webhook_bodies[0]["url"],
        format!("{PUBLIC_BASE}/webhooks/extensions/telegram/updates"),
        "the registered URL derives from the deployment public base"
    );
    let webhook_secret = set_webhook_bodies[0]["secret_token"]
        .as_str()
        .expect("setWebhook carries the minted secret")
        .to_string();
    assert!(!webhook_secret.is_empty(), "webhook secret is minted");

    let (status, body) = call_route(
        protected.router.clone(),
        Method::GET,
        "/api/webchat/v2/channels/telegram/setup",
        Some(caller.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["configured"], true, "redacted status: {body}");
    assert_eq!(body["bot_username"], BOT_USERNAME);
    assert!(
        body.get("bot_token").is_none(),
        "raw secrets are never echoed: {body}"
    );

    // ── Seam 2: in-chat activation parks BlockedAuth (the pairing gate) ────
    let webui_router = || {
        reborn_support::webui_mount::mount_webui_v2_router(Arc::clone(&webui.api), caller.clone())
    };
    let (status, body) = call_route(
        webui_router(),
        Method::POST,
        "/api/webchat/v2/threads",
        None,
        &[],
        Some(json!({"client_action_id": "journey-thread"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "thread create: {body}");
    let thread_id = body
        .get("thread_id")
        .or_else(|| {
            body.get("thread")
                .and_then(|thread| thread.get("thread_id"))
        })
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("thread id in response: {body}"))
        .to_string();

    let (status, body) = call_route(
        webui_router(),
        Method::POST,
        &format!("/api/webchat/v2/threads/{thread_id}/messages"),
        None,
        &[],
        Some(json!({"client_action_id": "journey-activate", "content": "set up telegram for me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "send message: {body}");
    let run_id: TurnRunId = body["run_id"]
        .as_str()
        .expect("submit response carries run_id")
        .parse()
        .expect("run id parses");

    let coordinator = runtime.webui_turn_coordinator_for_test();
    let webchat_scope = TurnScope::new_with_owner(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        None,
        ironclaw_host_api::ThreadId::from_trusted(thread_id.clone()),
        Some(UserId::new(USER).expect("user")),
    );
    wait_for_run_status(
        &coordinator,
        &webchat_scope,
        run_id,
        TurnStatus::BlockedAuth,
    )
    .await
    .expect("unpaired telegram activation parks the run on the pairing gate");

    // ── Seam 3: pairing code → verified /start webhook → binding + resume ──
    let (status, body) = call_route(
        protected.router.clone(),
        Method::POST,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller.clone()),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pairing issue: {body}");
    let code = body["code"].as_str().expect("pairing code").to_string();
    assert_eq!(
        body["deep_link"],
        format!("https://t.me/{BOT_USERNAME}?start={code}"),
        "deep link carries the bot username and code"
    );

    // Forged-secret probe first: the verified route fails closed.
    let (status, _body) = call_route(
        mounts.events.router.clone(),
        Method::POST,
        "/webhooks/extensions/telegram/updates",
        None,
        &[(SECRET_HEADER, "forged-secret")],
        Some(dm_update(1, &format!("/start {code}"))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a forged webhook secret must be rejected before any dispatch"
    );

    let (status, _body) = call_route(
        mounts.events.router.clone(),
        Method::POST,
        "/webhooks/extensions/telegram/updates",
        None,
        &[(SECRET_HEADER, webhook_secret.as_str())],
        Some(dm_update(2, &format!("/start {code}"))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verified /start consume acks 200");
    if let Some(drain) = mounts.events.drain.as_ref() {
        drain.drain().await;
    }

    // Binding row: the production channel-connection facade (over the durable
    // binding store) flips to connected.
    let (status, body) = call_route(
        protected.router.clone(),
        Method::GET,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["connected"], true,
        "consume must bind the telegram account: {body}"
    );

    // Paired confirmation reply into the DM.
    let confirmations = network.request_bodies_for("/sendMessage");
    assert!(
        confirmations
            .iter()
            .any(|body| body["chat_id"] == TG_CHAT_ID
                && body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Paired"))),
        "consume replies with the paired confirmation in the DM: {confirmations:?}"
    );

    // Continuation dispatch resumed the parked run to completion.
    wait_for_run_status(&coordinator, &webchat_scope, run_id, TurnStatus::Completed)
        .await
        .expect("pairing consume must resume the parked activation run");
    let (status, body) = call_route(
        webui_router(),
        Method::GET,
        &format!("/api/webchat/v2/threads/{thread_id}/timeline"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("telegram is ready"))
            })
        }),
        "the resumed run finalizes the post-activation reply: {body}"
    );

    // DM target: the production outbound-target provider (registered on the
    // runtime by the mounts build) lists the paired DM through the REAL
    // WebUI facade surface.
    let installation = format!("tg-bot-{BOT_ID}");
    let targets = webui
        .api
        .list_outbound_delivery_targets(caller.clone())
        .await
        .expect("outbound targets list");
    let expected_target_id = format!("telegram:dm:{installation}:{USER}");
    let dm_target = targets
        .targets
        .iter()
        .find(|option| option.target.target_id.as_str() == expected_target_id)
        .unwrap_or_else(|| {
            panic!(
                "pairing must record the DM delivery target {expected_target_id}; got: {:?}",
                targets
                    .targets
                    .iter()
                    .map(|option| option.target.target_id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        dm_target.target.channel.as_str(),
        "telegram",
        "the DM target is a telegram-channel entry"
    );
    assert!(
        dm_target.capabilities.final_replies,
        "the paired DM target must accept final replies"
    );

    // ── Seam 4: paired DM turn renders the reply through the revision
    //            workflow into the DM ─────────────────────────────────────
    let (status, _body) = call_route(
        mounts.events.router.clone(),
        Method::POST,
        "/webhooks/extensions/telegram/updates",
        None,
        &[(SECRET_HEADER, webhook_secret.as_str())],
        Some(dm_update(3, "hi ironclaw")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");
    if let Some(drain) = mounts.events.drain.as_ref() {
        drain.drain().await;
    }

    let mut rendered = None;
    for _ in 0..200 {
        let sends = network.request_bodies_for("/sendMessage");
        if let Some(body) = sends.iter().find(|body| {
            body["chat_id"] == TG_CHAT_ID
                && body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("hello from your ironclaw bot"))
        }) {
            rendered = Some(body.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let rendered = rendered.expect("the DM turn's final reply renders into the chat");
    assert_eq!(rendered["chat_id"], TG_CHAT_ID);
    let reply_urls: Vec<String> = network
        .requests()
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .map(|request| request.url.clone())
        .collect();
    assert!(
        reply_urls
            .iter()
            .all(|url| url.contains(&format!("/bot{BOT_TOKEN}/"))),
        "every outbound send carries the substituted bot path: {reply_urls:?}"
    );

    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");
}
