//! MCP-registration spec test #2 (per-user isolation): actor A registers an
//! MCP server descriptor and can search/install it; actor B must not see it
//! in search or be able to install it. T1 ships only the read side (the
//! register verb — the production writer — lands with T3), so the descriptor
//! is seeded by writing `manifest.toml` directly onto the harness's on-disk
//! tenant-scoped `/system/extensions/registered/<tenant>/<owner>/<id>/` tree —
//! the exact layout `RegisteredExtensionStore::list_for_scope` reads. Drives the
//! AGENT path (`builtin.extension_search`/`builtin.extension_install` via real
//! `submit_turn`s) through `ExtensionLifecycleToolHandler::dispatch`
//! (`extension_lifecycle_capabilities.rs`), the load-bearing caller the T1
//! review flagged as easy to miss.
//!
//! `ExtensionList`/`ExtensionProject` have no `builtin.*` capability id (they
//! are WebUI-facade-only) and this harness never builds the `RebornRuntime`
//! `build_webui_services` requires, so they cannot be driven from here; their
//! owner-masking is pinned at the crate tier instead
//! (`extension_list_shows_owner_registered_install_only_to_owner` and
//! `project_of_registered_package_masks_foreign_owner` in
//! `extension_lifecycle.rs`, the only tier that can call `list_installed`/
//! `project` directly).

use super::reborn_support::assertions::ToolErrorClass;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
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
    // T1 ships only the read side; the register verb (T3) is the production
    // writer for this tenant-scoped filesystem seam.
    let capability_harness = g.capability_harness().ok_or(
        "multiuser_extension_lifecycle_tools group must wire a HostRuntime capability harness",
    )?;
    let owner_user_id = a
        .binding
        .subject_user_id
        .as_ref()
        .ok_or("actor A's resolved binding has no subject_user_id")?;
    let manifest_dir = capability_harness.storage_root_for_test().join(format!(
        "system/extensions/registered/{}/{}/{}",
        a.binding.tenant_id.as_str(),
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
    // Owner A's search must surface its own registered descriptor via the
    // registered-store overlay in `search()`.
    a.assert_tool_result_contains(REGISTERED_EXTENSION_ID)
        .await
        .map_err(|e| {
            format!(
                "owner A's search did not surface its own registered extension \
                 {REGISTERED_EXTENSION_ID}: {e}"
            )
        })?;

    // ── A installs its own registered server: the successful owner path ─────
    // Not just "the cross-owner path is denied" (below) — the SAME owner must
    // actually be able to search-then-install their own registered
    // descriptor, and the install must never materialize the owner-registered
    // manifest under the shared `/system/extensions/<id>/` root (that would
    // let the next boot's first-party catalog scan re-adopt it — see
    // `is_owner_registered` in extension_lifecycle.rs).
    let a_install = g
        .thread("conv-ext-reg-iso-a-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({ "extension_id": REGISTERED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    a_install
        .submit_turn(&format!("install {REGISTERED_EXTENSION_ID}"))
        .await
        .map_err(|e| format!("[A install submit] {e}"))?;
    a_install
        .assert_tool_invoked("builtin.extension_install")
        .await
        .map_err(|e| format!("[A install invoked] {e}"))?;
    a_install
        .assert_tool_result_contains("\"installed\":true")
        .await
        .map_err(|e| format!("[A install must succeed for its own registered extension] {e}"))?;

    let shared_extension_root = capability_harness
        .storage_root_for_test()
        .join(format!("system/extensions/{REGISTERED_EXTENSION_ID}"));
    if shared_extension_root.exists() {
        return Err(format!(
            "owner-registered install must not materialize under the shared \
             /system/extensions/{REGISTERED_EXTENSION_ID} root: found {shared_extension_root:?}"
        )
        .into());
    }

    // ── Row-owner pin: the durable installation row is THE visibility
    // predicate for installed registered packages (InstallationOwner::user),
    // so assert it directly on the persisted state, not just via list/search
    // behavior above.
    let state_path = capability_harness
        .storage_root_for_test()
        .join("system/extensions/.installations/state.json");
    let state_raw = std::fs::read_to_string(&state_path)
        .map_err(|e| format!("[row pin] read installation state {state_path:?}: {e}"))?;
    let state: serde_json::Value = serde_json::from_str(&state_raw)
        .map_err(|e| format!("[row pin] parse installation state: {e}"))?;
    let row = state["installations"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["extension_id"] == REGISTERED_EXTENSION_ID)
        })
        .ok_or("[row pin] registered installation row missing from persisted state")?;
    let owner = &row["owner"];
    if owner["kind"] != "users"
        || !owner["user_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == owner_user_id.as_str()))
    {
        return Err(format!(
            "[row pin] registered install row must carry InstallationOwner::user(<owner>), got {owner}"
        )
        .into());
    }

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

    Ok(())
}
