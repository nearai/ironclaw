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

use std::io::Write;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::StatusCode;
use axum::http::{Method, Request};
use chrono::{DateTime, Utc};
use ironclaw_events::InMemoryDurableEventLog;
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{CompositeRootFilesystem, LibSqlRootFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityId, EffectKind, ExtensionId, PermissionMode, TenantId, UserId,
};
use ironclaw_product_adapters::ProductOutboundPayload;
use ironclaw_product_workflow::{
    RebornOperatorToolCatalog, RebornOperatorToolInfo, RebornServices, RebornServicesApi,
    RebornStreamEventsRequest,
};
use ironclaw_reborn_composition::test_support::BudgetTestGateway;
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
    build_webui_services, local_dev_runtime_policy,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource, TurnStatus};
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::session_thread::RebornThreadHarness;
use reborn_support::webui_mount::{get_json, mount_webui_v2_router, post_json, webui_caller_for};
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

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
    let finalized_reply = messages
        .iter()
        .find(|message| {
            message["kind"] == "assistant"
            && message["status"] == "finalized"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("pong"))
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a finalized assistant message containing 'pong' after a fresh libsql reopen: {body}"
            )
        });
    for field in ["created_at", "updated_at"] {
        finalized_reply[field]
            .as_str()
            .unwrap_or_else(|| panic!("{field} missing after libsql reopen: {body}"))
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|error| panic!("{field} not RFC3339 after libsql reopen: {error}"));
    }
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

#[async_trait::async_trait]
impl RebornOperatorToolCatalog for TestOperatorToolCatalog {
    // Caller-agnostic double; owner filtering is exercised by the
    // composition-tier catalog test (#5459 P1).
    async fn list_operator_tools(
        &self,
        _caller: &ironclaw_host_api::UserId,
    ) -> Vec<RebornOperatorToolInfo> {
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

/// Production-composed WebUI import path: the local-dev composition owns the
/// real lifecycle facade and extension-management port, while this test uses
/// only an inert model/network boundary because no turn or outbound request is
/// needed. The default router mount proves the operator route is forbidden;
/// the operator-capability mount then submits the same valid ZIP through the
/// real route and checks both catalog and filesystem effects.
#[tokio::test]
async fn operator_can_import_extension_bundle_through_production_webui_facade() {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let tenant_id = TenantId::new("webui-import-tenant").expect("tenant id");
    let agent_id = AgentId::new("webui-import-agent").expect("agent id");
    let user_id = UserId::new("webui-import-operator").expect("user id");
    let input = RebornBuildInput::local_dev(user_id.as_str(), storage_root.clone())
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::new(
            reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
        ));
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "webui-import-source".to_string(),
                reply_target_binding_id: "webui-import-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                "unused", 0, 0,
            ))),
    )
    .await
    .expect("production Reborn runtime builds");
    let webui = build_webui_services(&runtime, None).expect("production WebUI facade builds");
    let caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id,
        user_id,
        Some(agent_id),
        None,
    );
    let bundle = importable_extension_zip("webui-uploaded");

    let (status, body) = post_raw(
        mount_webui_v2_router(Arc::clone(&webui.api), caller.clone()),
        "/api/webchat/v2/extensions/import",
        bundle.clone(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-operator response: {body}"
    );
    assert!(
        !storage_root
            .join("system/extensions/webui-uploaded")
            .exists(),
        "forbidden upload must not reach lifecycle storage"
    );

    let operator_router = webui_v2_router(WebUiV2State::new(
        Arc::clone(&webui.api),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(
        caller.clone().with_operator_webui_config(true),
    ))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }));
    let (status, body) =
        post_raw(operator_router, "/api/webchat/v2/extensions/import", bundle).await;
    assert_eq!(status, StatusCode::OK, "operator response: {body}");
    assert_eq!(body["success"], true);

    let (status, body) = get_json(
        mount_webui_v2_router(
            Arc::clone(&webui.api),
            ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
                caller.tenant_id.clone(),
                caller.user_id.clone(),
                caller.agent_id.clone(),
                caller.project_id.clone(),
            ),
        ),
        "/api/webchat/v2/extensions/registry",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "registry response: {body}");
    assert!(
        body["entries"].as_array().is_some_and(|entries| entries
            .iter()
            .any(|entry| entry["package_ref"]["id"] == "webui-uploaded")),
        "imported package must be visible in the real extension catalog: {body}"
    );
    for path in [
        "manifest.toml",
        "wasm/tool.wasm",
        "schemas/run.input.json",
        "schemas/run.output.json",
    ] {
        assert!(
            storage_root
                .join("system/extensions/webui-uploaded")
                .join(path)
                .is_file(),
            "import lifecycle must materialize {path}"
        );
    }

    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn production_runtime_canonicalizes_legacy_multi_row_extension_installs() {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let tenant_id = TenantId::new("webui-legacy-tenant").expect("tenant id");
    let agent_id = AgentId::new("webui-legacy-agent").expect("agent id");
    let operator_id = UserId::new("webui-legacy-operator").expect("operator id");
    let input = RebornBuildInput::local_dev(operator_id.as_str(), storage_root.clone())
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::new(
            reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
        ));
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "webui-legacy-source".to_string(),
                reply_target_binding_id: "webui-legacy-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                "unused", 0, 0,
            ))),
    )
    .await
    .expect("production Reborn runtime builds");
    let webui = build_webui_services(&runtime, None).expect("production WebUI facade builds");
    let operator_caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id.clone(),
        operator_id,
        Some(agent_id.clone()),
        None,
    );

    let operator_router = webui_v2_router(WebUiV2State::new(
        Arc::clone(&webui.api),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(
        operator_caller.with_operator_webui_config(true),
    ))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }));
    let (status, body) = post_raw(
        operator_router,
        "/api/webchat/v2/extensions/import",
        importable_extension_zip("legacy-members"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator response: {body}");
    assert_eq!(body["success"], true);

    let alice_id = UserId::new("alice").expect("alice id");
    let bob_id = UserId::new("bob").expect("bob id");
    let install_request = serde_json::json!({
        "package_ref": {"kind": "extension", "id": "legacy-members"}
    });
    for (name, user_id) in [("Alice", alice_id.clone()), ("Bob", bob_id.clone())] {
        let caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
            tenant_id.clone(),
            user_id,
            Some(agent_id.clone()),
            None,
        );
        let (status, body) = post_json(
            mount_webui_v2_router(Arc::clone(&webui.api), caller),
            "/api/webchat/v2/extensions/install",
            install_request.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{name} install response: {body}");
        assert_eq!(body["success"], true, "{name} install response: {body}");
    }

    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");

    let state_path = storage_root.join("system/extensions/.installations/state.json");
    let mut state: Value = serde_json::from_slice(
        &std::fs::read(&state_path).expect("read extension installation state"),
    )
    .expect("extension installation state is JSON");
    let rows = state["installations"]
        .as_array()
        .expect("installations array");
    assert_eq!(rows.len(), 1, "member installs must already share one row");
    let mut alice_row = rows[0].clone();
    alice_row["installation_id"] = Value::String("legacy-alice-legacy-members".to_string());
    alice_row["owner"] = serde_json::json!({"kind": "user", "user_id": "alice"});
    let mut bob_row = rows[0].clone();
    bob_row["installation_id"] = Value::String("legacy-bob-legacy-members".to_string());
    bob_row["owner"] = serde_json::json!({"kind": "user", "user_id": "bob"});
    state["installations"] = serde_json::json!([alice_row, bob_row]);
    std::fs::write(
        &state_path,
        serde_json::to_vec_pretty(&state).expect("serialize legacy installation state"),
    )
    .expect("write legacy installation state");

    let rebuilt_input = RebornBuildInput::local_dev("webui-legacy-operator", storage_root.clone())
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::new(
            reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
        ));
    let rebuilt_runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(rebuilt_input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "webui-legacy-source".to_string(),
                reply_target_binding_id: "webui-legacy-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                "unused", 0, 0,
            ))),
    )
    .await
    .expect("rebuilt production Reborn runtime builds");
    let rebuilt_webui =
        build_webui_services(&rebuilt_runtime, None).expect("rebuilt WebUI facade builds");

    let store = ironclaw_reborn_composition::test_support::open_local_dev_extension_installation_store_for_test(
        &storage_root,
    )
    .await
    .expect("reopen canonical extension installation store");
    let installations = store
        .list_installations()
        .await
        .expect("list canonical extension installations");
    assert_eq!(
        installations.len(),
        1,
        "legacy rows must canonicalize to one installation"
    );
    let installation = &installations[0];
    assert_eq!(installation.installation_id().as_str(), "legacy-members");
    assert_eq!(installation.extension_id().as_str(), "legacy-members");
    let members = installation
        .owner()
        .members()
        .expect("canonical installation is member-owned");
    assert!(
        members.contains(&alice_id),
        "canonical owner contains Alice"
    );
    assert!(members.contains(&bob_id), "canonical owner contains Bob");

    let alice_caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id.clone(),
        alice_id,
        Some(agent_id.clone()),
        None,
    );
    let (status, body) = get_json(
        mount_webui_v2_router(Arc::clone(&rebuilt_webui.api), alice_caller),
        "/api/webchat/v2/extensions",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Alice extensions response: {body}");
    let alice_extension = body["extensions"]
        .as_array()
        .and_then(|extensions| {
            extensions
                .iter()
                .find(|extension| extension["package_ref"]["id"] == "legacy-members")
        })
        .unwrap_or_else(|| panic!("Alice should see private legacy-members: {body}"));
    assert_eq!(alice_extension["install_scope"], "private");

    let bob_caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id,
        bob_id,
        Some(agent_id),
        None,
    );
    let (status, body) = get_json(
        mount_webui_v2_router(Arc::clone(&rebuilt_webui.api), bob_caller),
        "/api/webchat/v2/extensions",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Bob extensions response: {body}");
    let bob_extension = body["extensions"]
        .as_array()
        .and_then(|extensions| {
            extensions
                .iter()
                .find(|extension| extension["package_ref"]["id"] == "legacy-members")
        })
        .unwrap_or_else(|| panic!("Bob should see private legacy-members: {body}"));
    assert_eq!(bob_extension["install_scope"], "private");

    drop(store);
    drop(rebuilt_webui);
    rebuilt_runtime
        .shutdown()
        .await
        .expect("rebuilt runtime shuts down");
}

/// PR #5499 review finding (High): a persisted installation row whose
/// extension id the catalog does not (yet) materialize a package for — e.g. a
/// placeholder row the standalone v1->Reborn migration tool writes ahead of
/// catalog package materialization — must not abort
/// `restore_extension_lifecycle_state` for every other installation. Before
/// the fix, `catalog.resolve(&package_ref)?` on that one row propagated all
/// the way through `build_reborn_runtime`, so a SINGLE orphan row made every
/// subsequent `ironclaw-reborn serve` startup fail. This mirrors
/// `production_runtime_canonicalizes_legacy_multi_row_extension_installs`'s
/// restart-with-hand-edited-state-file shape, but for a catalog-absent row
/// instead of a legacy multi-row membership shape.
#[tokio::test]
async fn production_runtime_restart_skips_installation_row_absent_from_catalog() {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let tenant_id = TenantId::new("webui-orphan-tenant").expect("tenant id");
    let agent_id = AgentId::new("webui-orphan-agent").expect("agent id");
    let operator_id = UserId::new("webui-orphan-operator").expect("operator id");
    let input = RebornBuildInput::local_dev(operator_id.as_str(), storage_root.clone())
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::new(
            reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
        ));
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "webui-orphan-source".to_string(),
                reply_target_binding_id: "webui-orphan-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                "unused", 0, 0,
            ))),
    )
    .await
    .expect("production Reborn runtime builds");
    let webui = build_webui_services(&runtime, None).expect("production WebUI facade builds");
    let operator_caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id.clone(),
        operator_id.clone(),
        Some(agent_id.clone()),
        None,
    );

    let operator_router = webui_v2_router(WebUiV2State::new(
        Arc::clone(&webui.api),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(
        operator_caller.with_operator_webui_config(true),
    ))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }));
    let (status, body) = post_raw(
        operator_router.clone(),
        "/api/webchat/v2/extensions/import",
        importable_extension_zip("catalog-present"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator response: {body}");
    assert_eq!(body["success"], true);
    let install_request = serde_json::json!({
        "package_ref": {"kind": "extension", "id": "catalog-present"}
    });
    let (status, body) = post_json(
        operator_router,
        "/api/webchat/v2/extensions/install",
        install_request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator install response: {body}");
    assert_eq!(body["success"], true, "operator install response: {body}");

    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");

    // Hand-edit the persisted state file the way a standalone v1->Reborn
    // migration tool would: add a manifest + installation row for an
    // extension id that has no corresponding materialized catalog package
    // (no `/system/extensions/<id>/` files were written for it), simulating
    // a migration that has not yet materialized catalog packages.
    let state_path = storage_root.join("system/extensions/.installations/state.json");
    let mut state: Value = serde_json::from_slice(
        &std::fs::read(&state_path).expect("read extension installation state"),
    )
    .expect("extension installation state is JSON");
    let manifests = state["manifests"]
        .as_array()
        .expect("manifests array")
        .clone();
    let mut orphan_manifest = manifests
        .first()
        .expect("at least one manifest present")
        .clone();
    orphan_manifest["raw_toml"] = Value::String(
        orphan_manifest["raw_toml"]
            .as_str()
            .expect("raw_toml is a string")
            .replace("catalog-present", "orphan-migrated"),
    );
    let mut new_manifests = manifests;
    new_manifests.push(orphan_manifest);
    state["manifests"] = Value::Array(new_manifests);

    let installations = state["installations"]
        .as_array()
        .expect("installations array")
        .clone();
    let mut orphan_row = installations
        .first()
        .expect("at least one installation present")
        .clone();
    orphan_row["installation_id"] = Value::String("orphan-migrated".to_string());
    orphan_row["extension_id"] = Value::String("orphan-migrated".to_string());
    orphan_row["manifest_ref"]["extension_id"] = Value::String("orphan-migrated".to_string());
    let mut new_installations = installations;
    new_installations.push(orphan_row);
    state["installations"] = Value::Array(new_installations);

    std::fs::write(
        &state_path,
        serde_json::to_vec_pretty(&state).expect("serialize orphan installation state"),
    )
    .expect("write orphan installation state");

    let rebuilt_input = RebornBuildInput::local_dev("webui-orphan-operator", storage_root.clone())
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::new(
            reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
        ));
    let rebuilt_runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(rebuilt_input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "webui-orphan-source".to_string(),
                reply_target_binding_id: "webui-orphan-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                "unused", 0, 0,
            ))),
    )
    .await
    .expect("rebuilt production Reborn runtime builds despite the orphan installation row");
    let rebuilt_webui =
        build_webui_services(&rebuilt_runtime, None).expect("rebuilt WebUI facade builds");

    // The orphan row is preserved untouched (never deleted or rewritten) so
    // it can restore once the migration tool later materializes its catalog
    // package.
    let store = ironclaw_reborn_composition::test_support::open_local_dev_extension_installation_store_for_test(
        &storage_root,
    )
    .await
    .expect("reopen canonical extension installation store");
    assert!(
        store
            .get_installation(
                &ironclaw_extensions::ExtensionInstallationId::new("orphan-migrated")
                    .expect("valid installation id")
            )
            .await
            .expect("read orphan installation")
            .is_some(),
        "orphan installation row must be preserved, not deleted"
    );

    // The catalog-present installation still restores and is reachable
    // through the real WebUI facade.
    let operator_caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id,
        operator_id,
        Some(agent_id),
        None,
    );
    let (status, body) = get_json(
        mount_webui_v2_router(Arc::clone(&rebuilt_webui.api), operator_caller),
        "/api/webchat/v2/extensions",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "operator extensions response: {body}"
    );
    let present_extension = body["extensions"]
        .as_array()
        .and_then(|extensions| {
            extensions
                .iter()
                .find(|extension| extension["package_ref"]["id"] == "catalog-present")
        })
        .unwrap_or_else(|| panic!("catalog-present extension must still restore: {body}"));
    assert_eq!(present_extension["install_scope"], "shared");

    drop(store);
    drop(rebuilt_webui);
    rebuilt_runtime
        .shutdown()
        .await
        .expect("rebuilt runtime shuts down");
}

/// Pins PR #5499 private-install membership through the PRODUCTION webui
/// facade, mirroring the crate-tier invariants in
/// `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle/tests/private_install_tests.rs`
/// (`members_install_the_same_tool_independently` +
/// `operator_install_evicts_member_installs_to_tenant_shared`), but driven
/// through the real HTTP router instead of the facade directly.
#[tokio::test]
async fn member_installs_join_then_operator_install_evicts_to_tenant_shared_through_production_webui_facade()
 {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let tenant_id = TenantId::new("webui-eviction-tenant").expect("tenant id");
    let agent_id = AgentId::new("webui-eviction-agent").expect("agent id");
    let operator_id = UserId::new("webui-eviction-operator").expect("operator id");
    let input = RebornBuildInput::local_dev(operator_id.as_str(), storage_root.clone())
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::new(
            reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
        ));
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "webui-eviction-source".to_string(),
                reply_target_binding_id: "webui-eviction-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                "unused", 0, 0,
            ))),
    )
    .await
    .expect("production Reborn runtime builds");
    let webui = build_webui_services(&runtime, None).expect("production WebUI facade builds");

    let extension_id = "member-eviction-fixture";
    let operator_caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        tenant_id.clone(),
        operator_id.clone(),
        Some(agent_id.clone()),
        None,
    );
    let operator_router = webui_v2_router(WebUiV2State::new(
        Arc::clone(&webui.api),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(
        operator_caller.clone().with_operator_webui_config(true),
    ))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }));
    let (status, body) = post_raw(
        operator_router,
        "/api/webchat/v2/extensions/import",
        importable_extension_zip(extension_id),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator import response: {body}");
    assert_eq!(body["success"], true);

    let alice_id = UserId::new("alice").expect("alice id");
    let bob_id = UserId::new("bob").expect("bob id");
    let carol_id = UserId::new("carol").expect("carol id");
    let caller_for = |user_id: UserId| {
        ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
            tenant_id.clone(),
            user_id,
            Some(agent_id.clone()),
            None,
        )
    };
    let install_request = serde_json::json!({
        "package_ref": {"kind": "extension", "id": extension_id}
    });

    // 1: alice installs -> private install created.
    let (status, body) = post_json(
        mount_webui_v2_router(Arc::clone(&webui.api), caller_for(alice_id.clone())),
        "/api/webchat/v2/extensions/install",
        install_request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "alice install response: {body}");
    assert_eq!(body["success"], true, "alice install response: {body}");

    // 2: bob installs the SAME id -> joins the membership, not a duplicate error.
    let (status, body) = post_json(
        mount_webui_v2_router(Arc::clone(&webui.api), caller_for(bob_id.clone())),
        "/api/webchat/v2/extensions/install",
        install_request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bob install response: {body}");
    assert_eq!(body["success"], true, "bob join response: {body}");

    // 3: both members see a PRIVATE entry.
    for (name, user_id) in [("alice", alice_id.clone()), ("bob", bob_id.clone())] {
        let (status, body) = get_json(
            mount_webui_v2_router(Arc::clone(&webui.api), caller_for(user_id)),
            "/api/webchat/v2/extensions",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{name} extensions response: {body}");
        let entry = body["extensions"]
            .as_array()
            .and_then(|extensions| {
                extensions
                    .iter()
                    .find(|extension| extension["package_ref"]["id"] == extension_id)
            })
            .unwrap_or_else(|| panic!("{name} should see private {extension_id}: {body}"));
        assert_eq!(entry["install_scope"], "private", "{name} scope: {body}");
    }

    // 4: carol, never a member, does not see the entry at all (masked visibility).
    let (status, body) = get_json(
        mount_webui_v2_router(Arc::clone(&webui.api), caller_for(carol_id.clone())),
        "/api/webchat/v2/extensions",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "carol extensions response: {body}");
    assert!(
        body["extensions"].as_array().is_some_and(|extensions| {
            !extensions
                .iter()
                .any(|extension| extension["package_ref"]["id"] == extension_id)
        }),
        "carol must not see a member-private entry: {body}"
    );

    // 5: carol attempting to remove the member-private id gets the masked
    // "is not installed" denial (`ProductWorkflowError::InvalidBindingRequest`
    // via `install_policy::ensure_caller_may_operate`, mapped to 400 by
    // `map_lifecycle_error` in `lifecycle_setup.rs`) rather than a 403/404 that
    // would let a non-member distinguish "not installed" from "not yours".
    let (status, body) = post_json(
        mount_webui_v2_router(Arc::clone(&webui.api), caller_for(carol_id.clone())),
        &format!("/api/webchat/v2/extensions/{extension_id}/remove"),
        serde_json::json!({}),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "carol must not be able to remove a private install she is not a member of: {body}"
    );
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "masked denial maps InvalidBindingRequest to 400: {body}"
    );
    let body_text = body.to_string();
    assert!(
        !body_text.contains("alice") && !body_text.contains("bob"),
        "masked denial must not leak member identities: {body}"
    );

    // 6: operator installs the same id -> evicts both members to Tenant.
    let (status, body) = post_json(
        mount_webui_v2_router(Arc::clone(&webui.api), operator_caller.clone()),
        "/api/webchat/v2/extensions/install",
        install_request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator install response: {body}");
    assert_eq!(body["success"], true, "operator eviction response: {body}");

    // 7: everyone now sees the SHARED (tenant) entry.
    for (name, user_id) in [
        ("alice", alice_id.clone()),
        ("bob", bob_id.clone()),
        ("carol", carol_id.clone()),
    ] {
        let (status, body) = get_json(
            mount_webui_v2_router(Arc::clone(&webui.api), caller_for(user_id)),
            "/api/webchat/v2/extensions",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{name} extensions response: {body}");
        let entry = body["extensions"]
            .as_array()
            .and_then(|extensions| {
                extensions
                    .iter()
                    .find(|extension| extension["package_ref"]["id"] == extension_id)
            })
            .unwrap_or_else(|| panic!("{name} should see shared {extension_id}: {body}"));
        assert_eq!(entry["install_scope"], "shared", "{name} scope: {body}");
    }

    // 8: a former member cannot remove the now-tenant row; the operator can.
    let (status, body) = post_json(
        mount_webui_v2_router(Arc::clone(&webui.api), caller_for(alice_id.clone())),
        &format!("/api/webchat/v2/extensions/{extension_id}/remove"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "alice must not be able to remove the tenant-shared row: {body}"
    );

    let (status, body) = post_json(
        mount_webui_v2_router(Arc::clone(&webui.api), operator_caller),
        &format!("/api/webchat/v2/extensions/{extension_id}/remove"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator remove response: {body}");

    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");
}

async fn post_raw(router: Router, path: &str, body: Vec<u8>) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("content-type", "application/zip")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "response body is not valid JSON ({error}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, body)
}

fn importable_extension_zip(id: &str) -> Vec<u8> {
    let manifest = format!(
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "WebUI Imported Tool"
version = "0.1.0"
description = "Production WebUI import integration fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run the imported tool"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/run.input.json"
output_schema_ref = "schemas/run.output.json"
"#
    );
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (path, bytes) in [
        ("manifest.toml", manifest.as_bytes()),
        ("wasm/tool.wasm", b"\0asm\x0d\0\x01\0".as_slice()),
        ("schemas/run.input.json", b"{}".as_slice()),
        ("schemas/run.output.json", b"{}".as_slice()),
    ] {
        writer.start_file(path, options).expect("start zip entry");
        writer.write_all(bytes).expect("write zip entry");
    }
    writer.finish().expect("finish zip").into_inner()
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

/// W5-WEBUI-API-2: a browser refresh mid-gate must let the user rediscover
/// and resolve a pending approval gate. Mounts the real `webui_v2` router
/// over a hand-built `RebornServices` facade wired with the harness's own
/// turn-state-converged `ApprovalInteractionService`
/// (`local_dev_approval_interaction_service_with_turn_state_for_test`, the
/// same seam `RebornIntegrationGroupBuilder::with_real_gate_dispatch_services`
/// wires into `DefaultProductWorkflow`) and the production event-stream
/// recipe `sse_activity_stream_replay_and_reconnect` above already pins.
///
/// "Refresh" is simulated the same way that precedent does: a fresh
/// `stream_events` drain with `after_cursor: None` — the SSE handler is a
/// polling wrapper over the same drain (W5-WEBUI-SPIKE), so this is
/// behaviorally equivalent to a browser opening a brand new `EventSource`
/// after a cold reload, without the fragility of reading a chunked HTTP body
/// through `tower::ServiceExt::oneshot`.
#[tokio::test]
async fn approval_gate_rediscovered_and_resolved_after_refresh() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-webui-api2-approval-refresh")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                serde_json::json!({"path": "/workspace/api2_refresh_approved.txt", "content": "API2_REFRESH_PAYLOAD"}),
            ),
            RebornScriptedReply::text("file written after the post-refresh approval"),
        ])
        .build()
        .await
        .expect("thread builds");

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the api2 refresh file")
        .await
        .expect("blocks on a real approval gate");

    // Wire the REAL approval interaction service over the group's own shared
    // turn-state store — same test-support seam
    // `with_real_gate_dispatch_services` uses for `DefaultProductWorkflow`,
    // applied here directly to a webui-level `RebornServices` instead.
    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");

    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("webui-api2-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );
    let caller = webui_caller_for(&h.binding);
    let thread_id = h.binding.thread_id.as_str().to_string();

    // --- simulate a cold browser refresh: fresh drain, after_cursor: None ---
    let replayed = services
        .stream_events(
            caller.clone(),
            RebornStreamEventsRequest {
                thread_id: thread_id.clone(),
                after_cursor: None,
            },
        )
        .await
        .expect("post-refresh drain succeeds");
    let gate_prompt = replayed
        .events
        .iter()
        .find_map(|envelope| match &envelope.payload {
            ProductOutboundPayload::GatePrompt(view) if view.gate_ref == gate_ref.as_str() => {
                Some(view)
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "expected the replayed cold-refresh drain to surface a GatePrompt for {gate_ref:?}: {:?}",
                replayed.events
            )
        });
    assert_eq!(
        gate_prompt.turn_run_id, run_id,
        "replayed gate prompt must be for the actual blocked run"
    );

    // --- resolve via the REAL route, not a direct-resume test shortcut ---
    let (status, body) = post_json(
        mount_webui_v2_router(services.clone(), caller),
        &format!(
            "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{}/resolve",
            gate_ref.as_str()
        ),
        serde_json::json!({
            "client_action_id": "webui-api2-approve-after-refresh",
            "resolution": "approved",
            "always": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resolve_gate response body: {body}");

    h.wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("run completes after the real resolve_gate route resumes it");
    h.assert_workspace_file_contains("api2_refresh_approved.txt", "API2_REFRESH_PAYLOAD")
        .await
        .expect("the approved write actually re-dispatched and persisted");
}
