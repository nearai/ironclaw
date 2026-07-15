//! Reborn integration test — operator LLM-provider CRUD through the real
//! `webui_v2` router + real `RebornLlmConfigService`.
//!
//! Tier-2 audit finding (docs/plans/2026-07-15-reborn-tier2-extension-plan.md
//! §5b, "multi-user/admin/misc"): "Operator API has zero tier-2 coverage:
//! LLM-provider CRUD (including the `api_key` never echoed back redaction
//! invariant) ... no `tests/integration/` file touches any of it." This is a
//! new, dedicated bin rather than an addition to `webui_v2_product_api.rs`
//! (already 1300+ lines covering a different set of route families) —
//! operator LLM config is its own first-class, zero-to-one area.
//!
//! Same real-runtime construction as `webui_v2_product_api.rs`
//! (`RebornBuildInput::local_dev` -> `build_reborn_runtime` ->
//! `build_webui_services`), plus `.with_boot_config(...)`: the WebUI facade
//! only composes the real `RebornLlmConfigService` when the runtime carries a
//! boot config (`crates/ironclaw_reborn_composition/src/webui/facade.rs`'s
//! `build_llm_config_service`), so this is the one extra step beyond that
//! file's existing pattern needed to reach the LLM-config routes for real.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use ironclaw_reborn_composition::test_support::BudgetTestGateway;
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
    build_webui_services, local_dev_runtime_policy,
};
use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};
use ironclaw_webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use reborn_support::webui_mount::{get_json, post_json};
use serde_json::json;
use tempfile::tempdir;

/// A real operator caller over the real `webui_v2_router`, backed by a
/// production `RebornRuntime` built WITH a boot config so the LLM-config
/// settings service is actually wired (mirrors
/// `webui_v2_product_api.rs`'s operator-route construction, plus
/// `.with_boot_config(..)`).
async fn operator_router_with_llm_config() -> Router {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let reborn_home = root.path().join("reborn-home");
    let tenant_id = TenantId::new("operator-llm-tenant").expect("tenant id");
    let agent_id = AgentId::new("operator-llm-agent").expect("agent id");
    let operator_id = UserId::new("operator-llm-operator").expect("operator id");

    let home = RebornHome::resolve_from_env_parts(Some(reborn_home.into_os_string()), None, None)
        .expect("valid reborn home");
    let boot = RebornBootConfig::new(home, RebornProfile::LocalDev);

    let input = RebornBuildInput::local_dev(operator_id.as_str(), storage_root)
        .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"));
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: tenant_id.as_str().to_string(),
                agent_id: agent_id.as_str().to_string(),
                source_binding_id: "operator-llm-source".to_string(),
                reply_target_binding_id: "operator-llm-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant("unused", 0, 0)))
            .with_boot_config(boot),
    )
    .await
    .expect("production Reborn runtime builds");
    let webui = build_webui_services(&runtime, None).expect("production WebUI facade builds");

    let operator_caller =
        WebUiAuthenticatedCaller::new(tenant_id, operator_id, Some(agent_id), None)
            .with_operator_webui_config(true);

    webui_v2_router(WebUiV2State::new(
        Arc::clone(&webui.api),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(operator_caller))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }))
}

/// Full LLM-provider CRUD path through the real router + real
/// `RebornLlmConfigService`: upsert a custom provider with an API key, assert
/// the key value never appears in the response and `api_key_set` flips true,
/// re-fetch via GET to prove the redaction invariant holds on read (not just
/// on the write response), then delete and confirm the provider is gone.
/// `UpsertLlmProviderRequest` derives `Deserialize` only (no `Serialize`) and
/// `LlmProviderView` carries only `api_key_set: bool` — this test pins that
/// structural invariant end-to-end through the real HTTP contract, not a
/// unit test of a redaction function (there isn't one; the type shape IS the
/// guarantee).
#[tokio::test]
async fn upsert_llm_provider_never_echoes_api_key_then_get_then_delete() {
    let secret_key = "sk-test-should-never-appear-in-any-response";
    let router = operator_router_with_llm_config().await;

    let (status, snapshot) = post_json(
        router.clone(),
        "/api/webchat/v2/llm/providers",
        json!({
            "id": "acme",
            "name": "Acme",
            "adapter": "open_ai_completions",
            "base_url": "https://api.acme.test/v1",
            "default_model": "acme-1",
            "api_key": secret_key,
            "set_active": true,
            "model": "acme-1",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upsert response: {snapshot:?}");
    assert_redacted_snapshot_carries_acme(&snapshot, secret_key);

    // Re-fetch via GET on a fresh request against the SAME router/runtime —
    // the redaction invariant must hold on read, not just on the upsert
    // response, and the write must actually have persisted (not merely
    // echoed back in-memory).
    let (status, refetched) = get_json(router.clone(), "/api/webchat/v2/llm/providers").await;
    assert_eq!(status, StatusCode::OK, "get response: {refetched:?}");
    assert_redacted_snapshot_carries_acme(&refetched, secret_key);

    let (status, after_delete) = post_json(
        router.clone(),
        "/api/webchat/v2/llm/providers/acme/delete",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete response: {after_delete:?}");
    let providers_after_delete = after_delete["providers"]
        .as_array()
        .expect("providers array");
    assert!(
        providers_after_delete
            .iter()
            .all(|provider| provider["id"] != "acme"),
        "acme provider must be gone after delete: {after_delete:?}"
    );
    assert!(
        !after_delete.to_string().contains(secret_key),
        "api_key value must never appear in the delete response either: {after_delete:?}"
    );

    // Deleting the same (now-gone) provider id again is the same code path an
    // unknown-provider-id delete takes: `delete_async` reports nothing
    // removed -> `LlmConfigServiceError::NotFound` -> HTTP 404. No existing
    // suite drives this branch (crate-tier unit tests only cover the
    // key-cleanup-failure NotFound path; the contract suite only exercises
    // 403-unauthorized and happy-delete for this handler).
    let (status, not_found_body) = post_json(
        router,
        "/api/webchat/v2/llm/providers/acme/delete",
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "deleting an already-deleted (unknown) provider id must 404: {not_found_body:?}"
    );
}

/// Shared assertions for both the upsert response and the GET re-fetch: the
/// literal key value is absent everywhere in the body, the provider carries
/// `api_key_set: true`, and no `api_key` field exists on the provider view at
/// all (the type only has `api_key_set`).
fn assert_redacted_snapshot_carries_acme(snapshot: &serde_json::Value, secret_key: &str) {
    let body_text = snapshot.to_string();
    assert!(
        !body_text.contains(secret_key),
        "api_key value must never appear anywhere in the response body: {body_text}"
    );
    let acme = snapshot["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .find(|provider| provider["id"] == "acme")
        .unwrap_or_else(|| panic!("acme provider present in snapshot: {snapshot:?}"));
    assert_eq!(
        acme["api_key_set"], true,
        "api_key_set must be true after a keyed upsert: {acme:?}"
    );
    assert!(
        acme.get("api_key").is_none(),
        "no api_key field may exist on the provider view: {acme:?}"
    );
}
