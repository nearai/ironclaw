//! MCP-registration spec test #2 (per-user isolation). T1 parts (search
//! overlay, install rejection) below are unchanged. T3-iso
//! (`docs/plans/2026-07-08-mcp-reg-t3-plan.md`) extends this scenario with
//! three more owner-scoping seams: AC1b (`search_installation` must not leak
//! install status across owners on a bare-id collision — realistic pre-T3-reg,
//! since ids aren't owner-minted yet), AC2 (a `UserRegistered` extension's
//! capabilities must not appear in another owner's model toolbox), and
//! correction 10 (the SAME rule on the independent operator-tool-config
//! reader/writer). Seeding an ENABLED `UserRegistered` install directly via
//! the installation store exercises a state only T3-reg's register verb can
//! produce in production — acceptable only because T3-reg is stacked directly
//! behind this slice.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId, ExtensionManifest,
    ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage, ManifestSource,
};
use ironclaw_host_api::{ExtensionId, HostPortCatalog, VirtualPath};

use super::reborn_support::assertions::ToolErrorClass;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::reborn_support::webui_mount::{
    get_json, mount_webui_v2_router, post_json, webui_caller_for,
};
use serde_json::json;

/// Distinctive id/name only the seeded fixture carries — a search hit is
/// unambiguous evidence the registered descriptor was surfaced.
const REGISTERED_EXTENSION_ID: &str = "acme-mcp-registered";

const REGISTERED_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-registered"
name = "Acme Support MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (T1 fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// A model-visible capability under the SAME extension id, for the T3-iso
/// AC2 disclosure seam. Registered MCP servers publish zero capabilities
/// until T3-disc opens the discovery gate; this manifest bypasses discovery
/// (not the isolation filter under test) to give `active_model_visible_capabilities`
/// something to filter, exactly as T3-disc will once it wires real MCP tool
/// discovery through the same registry-publish step.
const REGISTERED_CAPABILITY_ID: &str = "acme-mcp-registered.search";
const REGISTERED_CAPABILITY_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-registered"
name = "Acme Support MCP"
version = "0.1.0"
description = "Registered MCP capability probe fixture (T3-iso)"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/acme.wasm"

[[capabilities]]
id = "acme-mcp-registered.search"
description = "Registered MCP search capability (fixture)"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (default actor) ──────────────────────────────────────────────
    let a = g
        .thread("conv-ext-reg-iso-a-search")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({ "query": REGISTERED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;

    // ── Seed: write A's registered manifest onto the shared on-disk tree ─────
    // Production `RegisteredExtensionStore::put()` (T1) does not exist yet;
    // this is the filesystem seam it will write through.
    let capability_harness = g.capability_harness().ok_or(
        "multiuser_extension_lifecycle_tools group must wire a HostRuntime capability harness",
    )?;
    let owner_user_id = a
        .binding
        .subject_user_id
        .as_ref()
        .ok_or("actor A's resolved binding has no subject_user_id")?;
    let manifest_dir = capability_harness.storage_root_for_test().join(format!(
        "system/extensions/registered/{}/{}",
        owner_user_id.as_str(),
        REGISTERED_EXTENSION_ID
    ));
    std::fs::create_dir_all(&manifest_dir)
        .map_err(|e| format!("[seed] create registered manifest dir: {e}"))?;
    std::fs::write(manifest_dir.join("manifest.toml"), REGISTERED_MANIFEST_TOML)
        .map_err(|e| format!("[seed] write registered manifest: {e}"))?;

    // ── A searches: the owner's own registered server must be discoverable ──
    a.submit_turn("search for my registered acme MCP server")
        .await
        .map_err(|e| format!("[A search submit] {e}"))?;
    a.assert_tool_invoked("builtin.extension_search")
        .await
        .map_err(|e| format!("[A search invoked] {e}"))?;
    // RED: nothing today reads `/system/extensions/registered/...` — this is
    // the missing overlay `search()` (T1) must add.
    a.assert_tool_result_contains(REGISTERED_EXTENSION_ID)
        .await
        .map_err(|e| {
            format!(
                "[RED, expected until T1] owner A's search did not surface its own \
                 registered extension {REGISTERED_EXTENSION_ID}: {e}"
            )
        })?;

    // ── Actor B (DISTINCT actor, SAME shared backend) ────────────────────────
    // Built AFTER A's search so B's capability-result baseline
    // (`baseline_result_count`, captured at `.build()`) starts past A's search
    // result. The group's capability recorder is shared and the isolation
    // assertion below reads the `[baseline..]` delta; building B first would
    // fold A's result into B's slice and mask real isolation. Mirrors the
    // sibling `scenario_memory_isolation_across_actors`, which likewise builds
    // actor B only after actor A's write.
    let b_search = g
        .thread("conv-ext-reg-iso-b-search")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({ "query": REGISTERED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    // Non-vacuity: if `with_actor_id` regressed to a no-op, both actors would
    // share one owner and the isolation pins below would trivially "pass".
    if a.binding.subject_user_id == b_search.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both actors resolved the same owner".into());
    }

    // ── B searches: must NOT see A's registered server ───────────────────────
    b_search
        .submit_turn("search for my registered acme MCP server")
        .await
        .map_err(|e| format!("[B search submit] {e}"))?;
    b_search
        .assert_tool_invoked("builtin.extension_search")
        .await
        .map_err(|e| format!("[B search invoked] {e}"))?;
    if b_search
        .assert_tool_result_contains(REGISTERED_EXTENSION_ID)
        .await
        .is_ok()
    {
        return Err(format!(
            "isolation failure: actor B's extension_search surfaced actor A's \
             registered extension {REGISTERED_EXTENSION_ID}"
        )
        .into());
    }

    // ── B installs A's package id: must fail, not silently succeed ──────────
    let b_install = g
        .thread("conv-ext-reg-iso-b-install")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({ "extension_id": REGISTERED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("install attempted"),
        ])
        .build()
        .await?;
    b_install
        .submit_turn(&format!("install {REGISTERED_EXTENSION_ID}"))
        .await
        .map_err(|e| format!("[B install submit] {e}"))?;
    b_install
        .assert_tool_invoked("builtin.extension_install")
        .await
        .map_err(|e| format!("[B install invoked] {e}"))?;
    // Same `catalog.resolve()`-miss reason `scenario_install_unknown_extension_id_fails_safely`
    // pins for an unknown id — B's view of A's registered id must resolve the
    // same way (not found), never silently install.
    b_install
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .map_err(|e| format!("[B install must fail, not silently succeed] {e}"))?;

    // ── T3-iso seed: an ENABLED UserRegistered install owned by A ────────────
    // Seeded directly through the installation store (T3-reg's write side
    // doesn't exist yet) so the three filters below have real enabled state to
    // scope: AC1b's `search_installation`, AC2's `active_model_visible_capabilities`,
    // and correction 10's operator-tool-config catalog.
    let services = capability_harness
        .reborn_services_for_test()
        .ok_or("harness must be built via new_with_options (RebornServices captured)")?;
    let installation_store = services
        .extension_installation_store_for_test()
        .ok_or("local-dev extension management must be wired")?;
    let owner_manifest_record = ExtensionManifestRecord::from_toml(
        REGISTERED_MANIFEST_TOML,
        ManifestSource::UserRegistered {
            owner: owner_user_id.clone(),
        },
        &HostPortCatalog::empty(),
        None,
    )
    .map_err(|e| format!("[seed] parse owner manifest record: {e}"))?;
    installation_store
        .upsert_manifest(owner_manifest_record)
        .await
        .map_err(|e| format!("[seed] upsert owner manifest record: {e}"))?;
    let registered_extension_id = ExtensionId::new(REGISTERED_EXTENSION_ID)
        .map_err(|e| format!("[seed] extension id: {e}"))?;
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new(REGISTERED_EXTENSION_ID.to_string())
            .map_err(|e| format!("[seed] installation id: {e}"))?,
        registered_extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(registered_extension_id.clone(), None),
        Vec::new(),
        Utc::now(),
    )
    .map_err(|e| format!("[seed] build installation: {e}"))?;
    installation_store
        .upsert_installation(installation)
        .await
        .map_err(|e| format!("[seed] upsert installation: {e}"))?;

    // ── AC1b: search_installation must not leak A's install status to B ──────
    // B independently has its OWN registered-store copy of the SAME bare id
    // (ids aren't owner-minted until T3-reg) — a realistic id collision. B's
    // search must show the id as available, never carrying A's `active` phase.
    let b_owner_user_id = b_search
        .binding
        .subject_user_id
        .as_ref()
        .ok_or("actor B's resolved binding has no subject_user_id")?;
    let b_manifest_dir = capability_harness.storage_root_for_test().join(format!(
        "system/extensions/registered/{}/{}",
        b_owner_user_id.as_str(),
        REGISTERED_EXTENSION_ID
    ));
    std::fs::create_dir_all(&b_manifest_dir)
        .map_err(|e| format!("[seed] create B's registered manifest dir: {e}"))?;
    std::fs::write(
        b_manifest_dir.join("manifest.toml"),
        REGISTERED_MANIFEST_TOML,
    )
    .map_err(|e| format!("[seed] write B's registered manifest: {e}"))?;

    let a_phase_search = g
        .thread("conv-ext-reg-iso-a-phase")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({ "query": REGISTERED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    a_phase_search
        .submit_turn("search for my registered acme MCP server")
        .await
        .map_err(|e| format!("[A phase search submit] {e}"))?;
    let a_output = a_phase_search
        .tool_result_output("builtin.extension_search")
        .await
        .map_err(|e| format!("[A phase search output] {e}"))?;
    let a_has_phase = installation_phase_present_for_id(&a_output, REGISTERED_EXTENSION_ID)
        .ok_or("owner A's search result is missing the seeded registered extension entry")?;
    if !a_has_phase {
        return Err(
            "owner A's own registered+enabled install should report an installation phase".into(),
        );
    }

    let b_phase_search = g
        .thread("conv-ext-reg-iso-b-phase")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({ "query": REGISTERED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    b_phase_search
        .submit_turn("search for my registered acme MCP server")
        .await
        .map_err(|e| format!("[B phase search submit] {e}"))?;
    let b_output = b_phase_search
        .tool_result_output("builtin.extension_search")
        .await
        .map_err(|e| format!("[B phase search output] {e}"))?;
    let b_has_phase = installation_phase_present_for_id(&b_output, REGISTERED_EXTENSION_ID)
        .ok_or("actor B's search result is missing B's own registered extension entry")?;
    if b_has_phase {
        return Err(format!(
            "isolation failure: actor B's search_installation leaked owner A's install \
             phase for a bare-id collision on {REGISTERED_EXTENSION_ID}"
        )
        .into());
    }

    // ── AC2 seed: publish a real capability descriptor under the SAME owner- ──
    // registered extension id, into the SAME shared registry, so correction
    // 10's operator-tool-config catalog below has something to filter.
    // AC2 itself (`active_model_visible_capabilities`'s owner filter on the
    // model-facing tool-disclosure path) is pinned at CRATE tier
    // (`extension_lifecycle.rs::active_model_visible_capabilities_is_owner_scoped`)
    // — not here: this harness's turn dispatch runs through
    // `create_recording_capability_port`, which grants capabilities from the
    // harness's STATIC `capability_ids` list fixed at construction, never
    // through the production `RefreshingLocalDevCapabilityPort` /
    // `active_model_visible_capabilities` local-dev path a dynamically
    // published, owner-scoped extension needs — so this integration tier
    // cannot reach that specific filter through a real `submit_turn`.
    let capability_package = registered_capability_probe_package()
        .map_err(|e| format!("[seed] build capability probe package: {e}"))?;
    let schema_dir = capability_harness.storage_root_for_test().join(format!(
        "system/extensions/{REGISTERED_EXTENSION_ID}/schemas"
    ));
    std::fs::create_dir_all(&schema_dir)
        .map_err(|e| format!("[seed] create capability probe schema dir: {e}"))?;
    std::fs::write(
        schema_dir.join("search.input.json"),
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    )
    .map_err(|e| format!("[seed] write capability probe input schema: {e}"))?;
    std::fs::write(
        schema_dir.join("search.output.json"),
        r#"{"type":"object"}"#,
    )
    .map_err(|e| format!("[seed] write capability probe output schema: {e}"))?;
    services
        .publish_bundled_extension_for_test(&capability_package)
        .ok_or("local-dev Reborn services missing extension management for test publish")?
        .map_err(|e| format!("[seed] publish capability probe package: {e}"))?;

    // ── Correction 10: the operator-tool-config surface has its own reader ───
    // ── (`list_operator_config`) and writer (`set_operator_config_key`) ───────
    let overrides = capability_harness
        .tool_permission_overrides_for_test()
        .ok_or("local-dev tool permission override store")?;
    let auto_approve = capability_harness
        .auto_approve_settings_for_test()
        .ok_or("local-dev auto-approve store")?;
    let persistent_policies = capability_harness
        .persistent_approval_policies_for_test()
        .ok_or("local-dev persistent approval-policy store")?;
    let tool_catalog = services
        .local_dev_operator_tool_catalog_for_test()
        .ok_or("local-dev operator tool catalog")?;
    // Non-vacuity: prove the catalog mechanism itself surfaces the seeded
    // capability at all (to its owner) before trusting B's absence below —
    // this harness activates no OTHER extension capability, so an empty
    // catalog for B is only meaningful once we know A's is non-empty.
    let a_visible_tool_ids: Vec<String> = tool_catalog
        .list_operator_tools(owner_user_id)
        .await
        .into_iter()
        .map(|tool| tool.capability_id.as_str().to_string())
        .collect();
    if !a_visible_tool_ids.contains(&REGISTERED_CAPABILITY_ID.to_string()) {
        return Err(format!(
            "owner A's own operator tool catalog should list its registered capability \
             {REGISTERED_CAPABILITY_ID}; saw {a_visible_tool_ids:?}"
        )
        .into());
    }
    let product_services: Arc<dyn ironclaw_product_workflow::RebornServicesApi> = Arc::new(
        ironclaw_product_workflow::RebornServices::new(
            b_phase_search.thread_harness.service.clone(),
            b_phase_search.coordinator.clone(),
        )
        .with_operator_approval_config(
            overrides,
            auto_approve,
            persistent_policies,
            tool_catalog,
        ),
    );
    let caller = webui_caller_for(&b_phase_search.binding);

    let (status, body) = get_json(
        mount_webui_v2_router(Arc::clone(&product_services), caller.clone()),
        "/api/webchat/v2/settings/tools",
    )
    .await;
    if status != axum::http::StatusCode::OK {
        return Err(format!("B's list_operator_config failed: {status} {body}").into());
    }
    let entries = body["entries"]
        .as_array()
        .ok_or("list_operator_config response missing entries array")?;
    if entries
        .iter()
        .any(|entry| entry["key"] == format!("tool.{REGISTERED_CAPABILITY_ID}"))
    {
        return Err(format!(
            "isolation failure: B's list_operator_config surfaced owner A's registered \
             capability {REGISTERED_CAPABILITY_ID}: {body}"
        )
        .into());
    }

    // Write path: B must not be able to set a permission override against A's
    // capability id (`find_operator_tool`'s owner filter).
    let (set_status, set_body) = post_json(
        mount_webui_v2_router(Arc::clone(&product_services), caller),
        &format!("/api/webchat/v2/settings/tools/{REGISTERED_CAPABILITY_ID}"),
        json!({"state": "disabled"}),
    )
    .await;
    if set_status != axum::http::StatusCode::BAD_REQUEST
        || set_body["error"] != "invalid_request"
        || set_body["validation_code"] != "unknown_key"
    {
        return Err(format!(
            "isolation failure: B's write against owner A's registered capability should \
             be rejected as an unknown tool key; got {set_status} {set_body}"
        )
        .into());
    }

    Ok(())
}

/// Whether the seeded extension's search-result entry carries an
/// `installation_phase` field. `None` means the entry itself was not found in
/// the response (a seeding/query bug, not an isolation result).
fn installation_phase_present_for_id(output: &serde_json::Value, id: &str) -> Option<bool> {
    let extensions = output
        .get("payload")
        .and_then(|payload| payload.get("extensions"))
        .and_then(|extensions| extensions.as_array())?;
    let entry = extensions
        .iter()
        .find(|entry| entry["package_ref"]["id"] == id)?;
    Some(entry.get("installation_phase").is_some())
}

fn registered_capability_probe_package() -> Result<ExtensionPackage, Box<dyn std::error::Error>> {
    let manifest = ExtensionManifest::parse(
        REGISTERED_CAPABILITY_MANIFEST_TOML,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
    )?;
    Ok(ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{REGISTERED_EXTENSION_ID}"))?,
    )?)
}
