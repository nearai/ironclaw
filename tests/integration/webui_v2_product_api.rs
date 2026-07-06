//! W5-WEBUI-API-1: `webui_v2` router mounted over a REAL `RebornServices`
//! facade, hand-built from int-tier harness parts — not the
//! `MinimalWebuiServices` fake in `webui_v2_router_smoke.rs` (rejects 24/25
//! methods). Hand-built because `webui.rs`'s builder needs a `&RebornRuntime`
//! the harness never constructs.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use ironclaw_events::InMemoryDurableEventLog;
use ironclaw_filesystem::{CompositeRootFilesystem, LibSqlRootFilesystem};
use ironclaw_host_api::{CapabilityId, EffectKind, ExtensionId, PermissionMode};
use ironclaw_product_adapters::ProductOutboundPayload;
use ironclaw_product_workflow::{
    RebornOperatorToolCatalog, RebornOperatorToolInfo, RebornServices, RebornServicesApi,
    RebornStreamEventsRequest,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource};
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::session_thread::RebornThreadHarness;
use reborn_support::webui_mount::{get_json, mount_webui_v2_router, post_json, webui_caller_for};

#[tokio::test]
async fn thread_history_cold_get_and_libsql_reopen() {
    let h = RebornIntegrationHarness::builder("conv-webui-timeline")
        .storage(StorageMode::LibSql)
        .script([RebornScriptedReply::text("pong")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("ping").await.expect("turn completes");

    // Cold-GET mechanics mirror `assert_reply_persists_after_reopen`'s LibSql
    // branch: a genuinely fresh `libsql::Database` connection to the on-disk
    // file, independent of the live composite `Arc`.
    let db_path = h
        ._shared
        .libsql_db_path
        .clone()
        .expect("LibSql storage mode has a db path");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("open fresh libsql for reopen"),
    );
    let fresh_fs = Arc::new(LibSqlRootFilesystem::new(db));
    fresh_fs
        .run_migrations()
        .await
        .expect("migrations on fresh libsql reopen are idempotent");
    let mut fresh_composite = CompositeRootFilesystem::new();
    ironclaw_reborn_composition::test_support::mount_local_dev_database_roots_for_test(
        &mut fresh_composite,
        fresh_fs,
    )
    .expect("mount fresh composite");
    let fresh_thread_harness = RebornThreadHarness::filesystem_shared_composite(
        h.thread_harness.scope.clone(),
        Arc::new(fresh_composite),
        Arc::clone(&h._shared.turn_root),
    )
    .expect("fresh thread harness over reopened composite");

    let services = RebornServices::new(fresh_thread_harness.service.clone(), h.coordinator.clone());
    let caller = webui_caller_for(&h.binding);
    let router = mount_webui_v2_router(Arc::new(services), caller);

    let (status, body) = get_json(
        router,
        &format!(
            "/api/webchat/v2/threads/{}/timeline",
            h.binding.thread_id.as_str()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let messages = body["messages"].as_array().expect("messages array");
    assert!(
        messages.iter().any(|message| message["kind"] == "assistant"
            && message["status"] == "finalized"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("pong"))),
        "expected a finalized assistant message containing 'pong' after a fresh libsql reopen: {body}"
    );
}

/// InMemory sibling of the LibSql reopen above: proves service
/// re-instantiation over the same in-process handle, not on-disk durability
/// (nothing is written to disk in `InMemory` mode).
#[tokio::test]
async fn thread_history_cold_get_after_in_memory_reopen() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("pong")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("ping").await.expect("turn completes");

    let fresh_service = h
        .thread_harness
        .service_instance()
        .expect("fresh in-memory service instance");
    let services = RebornServices::new(Arc::new(fresh_service), h.coordinator.clone());
    let caller = webui_caller_for(&h.binding);
    let router = mount_webui_v2_router(Arc::new(services), caller);

    let (status, body) = get_json(
        router,
        &format!(
            "/api/webchat/v2/threads/{}/timeline",
            h.binding.thread_id.as_str()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let messages = body["messages"].as_array().expect("messages array");
    assert!(
        messages.iter().any(|message| message["kind"] == "assistant"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("pong"))),
        "expected the finalized reply after an in-memory service re-instantiation: {body}"
    );
}

/// Hand-rolled double: single read-only method, no logic worth exercising
/// via a production impl (no enabler needed).
struct TestOperatorToolCatalog;

impl RebornOperatorToolCatalog for TestOperatorToolCatalog {
    fn list_operator_tools(&self) -> Vec<RebornOperatorToolInfo> {
        vec![RebornOperatorToolInfo {
            capability_id: CapabilityId::new("builtin.http").expect("capability id"),
            provider: ExtensionId::new("builtin").expect("extension id"),
            description: Arc::from("Make outbound HTTP requests"),
            default_permission: PermissionMode::Allow,
            effects: Arc::from(vec![EffectKind::Network]),
        }]
    }
}

#[tokio::test]
async fn settings_tool_permission_post_then_cold_read() {
    // `builtin_tools()`'s "core_builtin" profile builds its `HostRuntime` by
    // hand and never captures tool_permission_overrides/auto_approve_settings/
    // persistent_approval_policies (all `None` there). `live_approvals()`
    // flows through `ToolsProfile::build()` and captures all three.
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-webui-settings")
        .build()
        .await
        .expect("thread builds");
    let capability_harness = group
        .capability_harness()
        .expect("live_approvals group uses a host-runtime capability backend");

    let overrides = capability_harness
        .tool_permission_overrides_for_test()
        .expect("local-dev tool permission override store");
    let auto_approve = capability_harness
        .auto_approve_settings_for_test()
        .expect("local-dev auto-approve store");
    let persistent_policies = capability_harness
        .persistent_approval_policies_for_test()
        .expect("local-dev persistent approval-policy store");
    let caller = webui_caller_for(&h.binding);

    let services = RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
        .with_operator_approval_config(
            overrides,
            auto_approve,
            persistent_policies,
            Arc::new(TestOperatorToolCatalog),
        );
    let router = mount_webui_v2_router(Arc::new(services), caller.clone());

    // `disabled` is the only override-derived state distinguishable from
    // defaults: `builtin.http`'s default_permission=Allow means an unset
    // override always resolves to always_allow/ask_each_time, so only
    // `disabled` proves the override store round-trips.
    let (status, body) = post_json(
        router,
        "/api/webchat/v2/settings/tools/builtin.http",
        serde_json::json!({"state": "disabled"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "POST response body: {body}");
    assert_eq!(body["entry"]["value"]["state"], "disabled");

    // Cold read: a SECOND `RebornServices` over a fresh thread service AND
    // freshly-reopened tool-permission-override/auto-approve/persistent-policy
    // stores at the same on-disk `storage_root` (not the live `Arc`s above) —
    // this is what actually proves the POSTed state survives a store reopen,
    // rather than a second facade reading the same in-process handles.
    let fresh_thread_service = h
        .thread_harness
        .service_instance()
        .expect("fresh thread service instance");
    let (fresh_overrides, fresh_auto_approve, fresh_persistent_policies) =
        ironclaw_reborn_composition::test_support::open_local_dev_approval_settings_stores_for_test(
            &capability_harness.storage_root_for_test(),
        )
        .await
        .expect("reopen fresh local-dev approval-settings stores");
    let cold_services = RebornServices::new(Arc::new(fresh_thread_service), h.coordinator.clone())
        .with_operator_approval_config(
            fresh_overrides,
            fresh_auto_approve,
            fresh_persistent_policies,
            Arc::new(TestOperatorToolCatalog),
        );
    let cold_router = mount_webui_v2_router(Arc::new(cold_services), caller);

    let (status, body) = get_json(cold_router, "/api/webchat/v2/settings/tools").await;
    assert_eq!(status, StatusCode::OK, "GET response body: {body}");
    let entries = body["entries"].as_array().expect("entries array");
    let entry = entries
        .iter()
        .find(|entry| entry["key"] == "tool.builtin.http")
        .unwrap_or_else(|| panic!("tool.builtin.http entry present in cold read: {body}"));
    assert_eq!(
        entry["value"]["state"], "disabled",
        "POSTed permission state must survive the cold read: {entry}"
    );
}

/// W5-WEBUI-API-1 scenario 2: drives `RebornServicesApi::stream_events`
/// directly (SSE handler is a polling wrapper over the same drain, per
/// W5-WEBUI-SPIKE). Proves a lifecycle event delivers once and reconnect
/// with `after_cursor` past it doesn't redeliver. Uses Enabler A's narrowed
/// `build_webui_event_stream_for_test` (see its doc for the divergence).
#[tokio::test]
async fn sse_activity_stream_replay_and_reconnect() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("hello")])
        .build()
        .await
        .expect("harness builds");

    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("webui-api-1-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services = RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
        .with_event_stream(event_stream);

    h.submit_turn("hello").await.expect("turn completes");

    let caller = webui_caller_for(&h.binding);
    let thread_id = h.binding.thread_id.as_str().to_string();

    // Action 2 (drain): first poll sees the turn's lifecycle event(s).
    let first = services
        .stream_events(
            caller.clone(),
            RebornStreamEventsRequest {
                thread_id: thread_id.clone(),
                after_cursor: None,
            },
        )
        .await
        .expect("first drain succeeds");
    assert!(
        !first.events.is_empty(),
        "expected at least one turn-lifecycle event on the first drain"
    );
    assert!(
        first
            .events
            .iter()
            .any(|envelope| !matches!(envelope.payload, ProductOutboundPayload::KeepAlive)),
        "expected a real (non-KeepAlive) turn-lifecycle payload: {:?}",
        first.events
    );
    let last_cursor = first
        .events
        .last()
        .expect("non-empty first drain")
        .projection_cursor
        .clone();

    // Action 3 (reconnect): draining again with `after_cursor` past the last
    // delivered event must not redeliver it.
    let second = services
        .stream_events(
            caller,
            RebornStreamEventsRequest {
                thread_id,
                after_cursor: Some(last_cursor),
            },
        )
        .await
        .expect("reconnect drain succeeds");
    assert!(
        second.events.is_empty(),
        "reconnect with after_cursor must not redeliver the same event(s): {:?}",
        second.events
    );
}
