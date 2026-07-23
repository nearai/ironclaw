//! Regression contract for the two extension state machines exposed by WebUI:
//!
//! - tenant administrator configuration is deployment state, and
//! - installation/setup/removal is caller-owned user state.
//!
//! These journeys intentionally use the production WebUI router and composed
//! lifecycle facade. The imported extension is only a provider-neutral fixture;
//! none of the assertions depend on Slack, Telegram, or Notion behavior.

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
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use ironclaw_extensions::{ExtensionInstallation, ExtensionInstallationStore, InstallationOwner};
use ironclaw_host_api::ProductSurfaceCaller;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_reborn_composition::test_support::BudgetTestGateway;
use ironclaw_reborn_composition::{
    RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput, RebornWebuiBundle,
    build_reborn_runtime, build_webui_services, local_dev_build_input, local_dev_runtime_policy,
};
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use reborn_support::webui_mount::{get_json, mount_webui_v2_router, post_json};
use serde_json::Value;
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;

const EXTENSION_ID: &str = "user-lifecycle-fixture";

#[tokio::test]
async fn admin_configuration_does_not_install_an_extension_for_any_user() {
    let fixture = LifecycleIsolationFixture::new("admin-config").await;

    let (save_status, save_body) = put_json(
        fixture.operator_router(),
        "/api/webchat/v2/operator/extension-configuration/extension.slack",
        serde_json::json!({
            "values": slack_admin_configuration_values(),
            "expected_revision": 0,
            "idempotency_key": "admin-config-is-not-installation",
        }),
    )
    .await;
    assert_eq!(save_status, StatusCode::OK, "save response: {save_body}");

    for (name, caller) in [("Alice", fixture.alice()), ("Bob", fixture.bob())] {
        let (status, body) =
            get_json(fixture.member_router(caller), "/api/webchat/v2/extensions").await;
        assert_eq!(status, StatusCode::OK, "{name} list response: {body}");
        assert_extension_absent(&body, "slack", name);
    }

    fixture.shutdown().await;
}

#[tokio::test]
async fn users_install_and_remove_the_same_extension_independently() {
    let fixture = LifecycleIsolationFixture::new("independent-users").await;
    let (save_status, save_body) = put_json(
        fixture.operator_router(),
        "/api/webchat/v2/operator/extension-configuration/extension.slack",
        serde_json::json!({
            "values": slack_admin_configuration_values(),
            "expected_revision": 0,
            "idempotency_key": "independent-user-lifecycle-config",
        }),
    )
    .await;
    assert_eq!(save_status, StatusCode::OK, "save response: {save_body}");
    fixture.import_fixture_extension().await;

    for (name, caller) in [("Alice", fixture.alice()), ("Bob", fixture.bob())] {
        let (status, body) = post_json(
            fixture.member_router(caller),
            "/api/webchat/v2/extensions/install",
            serde_json::json!({
                "package_ref": {"kind": "extension", "id": EXTENSION_ID},
                "client_action_id": format!("isolation-install-{name}")
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{name} install response: {body}");
    }
    // Installation is the only user action for an extension without personal
    // setup requirements. Runtime publication/readiness is an internal
    // checkpoint and must complete before the install response returns.
    fixture.assert_user_phase(fixture.alice(), "active").await;
    fixture.assert_user_phase(fixture.bob(), "active").await;

    let (status, body) = post_json(
        fixture.member_router(fixture.alice()),
        &format!("/api/webchat/v2/extensions/{EXTENSION_ID}/remove"),
        serde_json::json!({"client_action_id": "isolation-remove-alice"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Alice remove response: {body}");
    fixture.assert_user_absent(fixture.alice()).await;
    fixture.assert_user_phase(fixture.bob(), "active").await;

    let (status, body) = post_json(
        fixture.member_router(fixture.bob()),
        &format!("/api/webchat/v2/extensions/{EXTENSION_ID}/remove"),
        serde_json::json!({"client_action_id": "isolation-remove-bob"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Bob remove response: {body}");
    fixture.assert_user_absent(fixture.bob()).await;

    let (status, body) = get_json(
        fixture.operator_router(),
        "/api/webchat/v2/operator/extension-configuration",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin config response: {body}");
    let slack = configuration_group(&body, "extension.slack");
    assert_eq!(slack["revision"], 1, "admin config response: {body}");
    assert_eq!(slack["complete"], true, "admin config response: {body}");

    fixture.shutdown().await;
}

#[tokio::test]
async fn legacy_tenant_owned_installation_migrates_to_operator_private_state() {
    let fixture = LifecycleIsolationFixture::new("legacy-tenant-row").await;
    fixture.import_fixture_extension().await;

    let (status, body) = post_json(
        fixture.member_router(fixture.alice()),
        "/api/webchat/v2/extensions/install",
        serde_json::json!({
            "package_ref": {"kind": "extension", "id": EXTENSION_ID},
            "client_action_id": "isolation-legacy-seed-install"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "seed install response: {body}");

    let LifecycleIsolationFixture {
        root,
        runtime,
        webui,
        storage_root,
        tenant_id,
        agent_id,
        operator_id,
        ..
    } = fixture;
    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");

    let store = ironclaw_reborn_composition::test_support::open_local_dev_extension_installation_store_for_test(
        &storage_root,
    )
    .await
    .expect("open installation store");
    let installation = store
        .list_installations()
        .await
        .expect("list installations")
        .into_iter()
        .find(|installation| installation.extension_id().as_str() == EXTENSION_ID)
        .expect("fixture installation exists");
    store
        .upsert_installation(
            ExtensionInstallation::new(
                installation.installation_id().clone(),
                installation.extension_id().clone(),
                installation.manifest_ref().clone(),
                installation.credential_bindings().to_vec(),
                Utc::now(),
                InstallationOwner::Tenant,
            )
            .expect("legacy tenant installation"),
        )
        .await
        .expect("replace fixture with legacy tenant row");
    drop(store);

    let rebuilt =
        LifecycleIsolationFixture::reopen(root, storage_root, tenant_id, agent_id, operator_id)
            .await;

    rebuilt
        .assert_user_phase(rebuilt.operator(), "active")
        .await;
    let (status, body) = get_json(
        rebuilt.member_router(rebuilt.operator()),
        "/api/webchat/v2/extensions",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "operator list response: {body}");
    assert_eq!(
        extension_entry(&body, EXTENSION_ID, "operator")["install_scope"],
        "private",
        "legacy tenant row must be narrowed to the operator's private state: {body}"
    );

    rebuilt.assert_user_absent(rebuilt.alice()).await;
    rebuilt.assert_user_absent(rebuilt.bob()).await;

    let (status, body) = post_json(
        rebuilt.member_router(rebuilt.operator()),
        &format!("/api/webchat/v2/extensions/{EXTENSION_ID}/remove"),
        serde_json::json!({"client_action_id": "isolation-remove-operator"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the compatibility owner must be able to remove its private install: {body}"
    );
    rebuilt.assert_user_absent(rebuilt.operator()).await;

    rebuilt.shutdown().await;
}

struct LifecycleIsolationFixture {
    root: TempDir,
    runtime: RebornRuntime,
    webui: RebornWebuiBundle,
    storage_root: std::path::PathBuf,
    tenant_id: TenantId,
    agent_id: AgentId,
    operator_id: UserId,
    alice_id: UserId,
    bob_id: UserId,
}

impl LifecycleIsolationFixture {
    async fn new(name: &str) -> Self {
        let root = tempdir().expect("runtime storage tempdir");
        let storage_root = root.path().join("local-dev");
        let tenant_id = TenantId::new(format!("lifecycle-{name}-tenant")).expect("tenant id");
        let agent_id = AgentId::new(format!("lifecycle-{name}-agent")).expect("agent id");
        let operator_id = UserId::new(format!("lifecycle-{name}-operator")).expect("operator id");
        Self::build(root, storage_root, tenant_id, agent_id, operator_id).await
    }

    async fn reopen(
        root: TempDir,
        storage_root: std::path::PathBuf,
        tenant_id: TenantId,
        agent_id: AgentId,
        operator_id: UserId,
    ) -> Self {
        Self::build(root, storage_root, tenant_id, agent_id, operator_id).await
    }

    async fn build(
        root: TempDir,
        storage_root: std::path::PathBuf,
        tenant_id: TenantId,
        agent_id: AgentId,
        operator_id: UserId,
    ) -> Self {
        let input = local_dev_build_input(operator_id.as_str(), storage_root.clone())
            // Root-test packages compile composition with `test-support`, where
            // `local_dev_build_input`'s cfg(test)-only first-party injection is
            // off — supply the bundled surface like the binary does (the
            // `extension.slack` admin group and the fixture installs need it).
            .with_bundled_first_party_for_test()
            .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
            .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
            .with_network_http_egress_for_test(Arc::new(
                reborn_support::harness::RecordingNetworkHttpEgress::with_body(Vec::new()),
            ));
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_build_input(input)
                .with_identity(RebornRuntimeIdentity {
                    tenant_id: tenant_id.as_str().to_string(),
                    agent_id: agent_id.as_str().to_string(),
                    source_binding_id: format!("{operator_id}-source"),
                    reply_target_binding_id: format!("{operator_id}-reply"),
                })
                .with_model_gateway_override(Arc::new(BudgetTestGateway::with_constant(
                    "unused", 0, 0,
                ))),
        )
        .await
        .expect("production runtime builds");
        let webui = build_webui_services(&runtime, None).expect("production WebUI facade builds");
        Self {
            root,
            runtime,
            webui,
            storage_root,
            tenant_id,
            agent_id,
            operator_id,
            alice_id: UserId::new("alice").expect("alice id"),
            bob_id: UserId::new("bob").expect("bob id"),
        }
    }

    fn caller(&self, user_id: UserId) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            self.tenant_id.clone(),
            user_id,
            Some(self.agent_id.clone()),
            None,
        )
    }

    fn operator(&self) -> UserId {
        self.operator_id.clone()
    }

    fn alice(&self) -> UserId {
        self.alice_id.clone()
    }

    fn bob(&self) -> UserId {
        self.bob_id.clone()
    }

    fn member_router(&self, user_id: UserId) -> Router {
        mount_webui_v2_router(
            Arc::clone(&self.webui.product_surface),
            self.caller(user_id),
        )
    }

    fn operator_router(&self) -> Router {
        webui_v2_router(WebUiV2State::new(
            Arc::clone(&self.webui.product_surface),
            DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
        ))
        .layer(axum::Extension(
            self.caller(self.operator()).with_operator_config(true),
        ))
        .layer(axum::Extension(WebUiV2Capabilities {
            operator_webui_config: true,
        }))
    }

    async fn import_fixture_extension(&self) {
        let (status, body) = post_raw(
            self.operator_router(),
            "/api/webchat/v2/extensions/import",
            importable_extension_zip(EXTENSION_ID),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "import response: {body}");
    }

    async fn assert_user_phase(&self, user_id: UserId, expected: &str) {
        let label = user_id.as_str().to_string();
        let (status, body) =
            get_json(self.member_router(user_id), "/api/webchat/v2/extensions").await;
        assert_eq!(status, StatusCode::OK, "{label} list response: {body}");
        assert_eq!(
            extension_entry(&body, EXTENSION_ID, &label)["installation_state"],
            expected,
            "{label} must independently remain {expected}: {body}"
        );
    }

    async fn assert_user_absent(&self, user_id: UserId) {
        let label = user_id.as_str().to_string();
        let (status, body) =
            get_json(self.member_router(user_id), "/api/webchat/v2/extensions").await;
        assert_eq!(status, StatusCode::OK, "{label} list response: {body}");
        assert_extension_absent(&body, EXTENSION_ID, &label);
    }

    async fn shutdown(self) {
        let Self { runtime, webui, .. } = self;
        drop(webui);
        runtime.shutdown().await.expect("runtime shuts down");
    }
}

fn extension_entry<'a>(body: &'a Value, extension_id: &str, label: &str) -> &'a Value {
    body["extensions"]
        .as_array()
        .and_then(|extensions| {
            extensions
                .iter()
                .find(|extension| extension["package_ref"]["id"] == extension_id)
        })
        .unwrap_or_else(|| panic!("{label} must see {extension_id}: {body}"))
}

fn assert_extension_absent(body: &Value, extension_id: &str, label: &str) {
    assert!(
        body["extensions"].as_array().is_some_and(|extensions| {
            !extensions
                .iter()
                .any(|extension| extension["package_ref"]["id"] == extension_id)
        }),
        "{label} must not inherit installation state for {extension_id}: {body}"
    );
}

fn configuration_group<'a>(body: &'a Value, group_id: &str) -> &'a Value {
    body["groups"]
        .as_array()
        .and_then(|groups| groups.iter().find(|group| group["group_id"] == group_id))
        .unwrap_or_else(|| panic!("configuration group {group_id} missing: {body}"))
}

async fn put_json(router: Router, path: &str, body: Value) -> (StatusCode, Value) {
    request_with_content_type(
        router,
        Method::PUT,
        path,
        "application/json",
        Body::from(body.to_string()),
    )
    .await
}

async fn post_raw(router: Router, path: &str, body: Vec<u8>) -> (StatusCode, Value) {
    request_with_content_type(
        router,
        Method::POST,
        path,
        "application/zip",
        Body::from(body),
    )
    .await
}

async fn request_with_content_type(
    router: Router,
    method: Method,
    path: &str,
    content_type: &str,
    body: Body,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", content_type)
                .body(body)
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

fn slack_admin_configuration_values() -> Value {
    serde_json::json!([
        {"handle": "slack_bot_token", "value": "xoxb-fixture"},
        {"handle": "slack_signing_secret", "value": "signing-secret"},
        {"handle": "slack_team_id", "value": "T-FIXTURE"},
        {"handle": "slack_api_app_id", "value": "A-FIXTURE"},
        {"handle": "slack_installation_id", "value": "I-FIXTURE"},
        {"handle": "slack_bot_user_id", "value": "U-FIXTURE"},
        {"handle": "slack_oauth_client_id", "value": "oauth-client"},
        {"handle": "slack_oauth_client_secret", "value": "oauth-secret"}
    ])
}

fn importable_extension_zip(id: &str) -> Vec<u8> {
    let manifest = format!(
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "User Lifecycle Fixture"
version = "0.1.0"
description = "Provider-neutral user lifecycle integration fixture"
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
description = "Run the fixture"
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
