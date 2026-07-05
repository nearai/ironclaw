//! W5-WEBUI-API-1: `ironclaw_webui_v2`'s router mounted over a REAL
//! `ironclaw_product_workflow::RebornServices` facade, wired by hand from
//! int-tier harness parts (thread service, turn coordinator, and per-scenario
//! `with_*` collaborators) — not the `MinimalWebuiServices` fake
//! `webui_v2_router_smoke.rs` uses. That file's fake rejects 24 of 25 methods
//! and never exercises the real facade logic (thread-scope resolution,
//! pagination, operator-config precedence); this suite exercises exactly
//! that real logic, over the SAME router the smoke suite mounts.
//!
//! Each scenario builds its own `RebornServices` chain from harness parts
//! (mirroring `crates/ironclaw_reborn_composition/src/webui.rs`'s
//! `build_webui_services_with_connectable_channels` call sequence by hand,
//! since the composition function itself takes a `&RebornRuntime` the
//! int-tier harness never builds) and mounts it via
//! `reborn_support::webui_mount::mount_webui_v2_router`.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use ironclaw_filesystem::{CompositeRootFilesystem, LibSqlRootFilesystem};
use ironclaw_host_api::{CapabilityId, EffectKind, ExtensionId, PermissionMode};
use ironclaw_product_workflow::{RebornOperatorToolCatalog, RebornOperatorToolInfo, RebornServices};
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

/// InMemory sibling of the LibSql reopen above, mirroring
/// `assert_reply_persists_after_reopen`'s own two-branch convention: proves
/// service re-instantiation over the same in-process handle (not on-disk
/// durability — nothing is written to disk in `InMemory` mode).
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

/// Test-local `RebornOperatorToolCatalog`: the trait is a single read-only
/// method with no internal dependencies worth exercising through a
/// production impl (unlike the automation facade's mapping/visibility
/// logic), so a hand-rolled double is the correct choice here (no enabler).
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
async fn settings_tool_permission_put_then_cold_read() {
    // NOTE (plan deviation): the plan called for `builtin_tools()`, but that
    // group's "core_builtin" profile builds its `HostRuntime` by hand
    // (`core_builtin_tools_from_runtime`, `harness/profiles/core_builtin.rs`)
    // rather than through `new_with_options`/`ToolsProfile::build()` — so it
    // never captures `tool_permission_overrides`/`auto_approve_settings`/
    // `persistent_approval_policies` (all `None` there, confirmed by running
    // this test against it first: it failed fast on the `None` unwrap, not a
    // false pass). `live_approvals()` flows through `ToolsProfile::build()`
    // (`file_tools_requiring_approval_profile()` in `group_constructors.rs`)
    // and captures all three unconditionally, matching every other
    // `new_with_options`-built group. The capability domain is otherwise
    // irrelevant here — this scenario never dispatches a tool call.
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
            overrides.clone(),
            auto_approve.clone(),
            persistent_policies.clone(),
            Arc::new(TestOperatorToolCatalog),
        );
    let router = mount_webui_v2_router(Arc::new(services), caller.clone());

    // `disabled` (not `always_allow`/`ask_each_time`): `builtin.http`'s
    // `default_permission = Allow` means `effective_tool_permission`'s
    // unset-override fallback always resolves to EITHER `always_allow` (global
    // auto-approve on) OR `ask_each_time` (global auto-approve off,
    // `live_approvals()`'s own precondition) — never `disabled`. Asserting
    // either of those two after the PUT would pass even if the override store
    // were never wired/persisted (confirmed empirically: both were tried and
    // both false-passed against a fresh-override-store mutation). `disabled`
    // is the one state that can only come from a real override record,
    // making this assertion — and the mutation below — actually discriminate.
    let (status, body) = post_json(
        router,
        "/api/webchat/v2/settings/tools/builtin.http",
        serde_json::json!({"state": "disabled"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT response body: {body}");
    assert_eq!(body["entry"]["value"]["state"], "disabled");

    // Cold read: a SECOND `RebornServices` instance over a fresh thread
    // service, wired with the SAME `overrides`/`auto_approve`/
    // `persistent_policies` `Arc`s — those stores are the durable state under
    // test, not the thread history.
    let fresh_thread_service = h
        .thread_harness
        .service_instance()
        .expect("fresh thread service instance");
    let cold_services = RebornServices::new(Arc::new(fresh_thread_service), h.coordinator.clone())
        .with_operator_approval_config(
            overrides,
            auto_approve,
            persistent_policies,
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
        "PUT'd permission state must survive the cold read: {entry}"
    );
}
