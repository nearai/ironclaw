//! MCP-registration spec test #2 (per-user isolation), RED for T1
//! (docs/plans/2026-07-08-mcp-reg-t1-plan.md): actor A registers an MCP server
//! descriptor; actor B must not see it in search or be able to install it.
//! `RegisteredExtensionStore::put()` doesn't exist yet, so the descriptor is
//! seeded by writing `manifest.toml` directly onto the harness's on-disk
//! `/system/extensions/registered/<owner>/<id>/` tree — the same physical
//! layout `load_filesystem_packages` reads for `/system/extensions/<id>/`,
//! one level shallower (`extension_host/available_extensions.rs`). Drives the
//! AGENT path (`builtin.extension_search`/`builtin.extension_install` via real
//! `submit_turn`s) through `ExtensionLifecycleToolHandler::dispatch`
//! (`extension_lifecycle_capabilities.rs`), the load-bearing caller the T1
//! review flagged as easy to miss.

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

    Ok(())
}
