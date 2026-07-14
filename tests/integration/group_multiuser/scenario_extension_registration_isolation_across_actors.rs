//! MCP-registration spec test #2 (per-user isolation): actor A registers an
//! MCP server descriptor and can search/install/activate/remove it; actor B
//! must not see it in search or be able to install, activate, or remove it.
//! T1 ships only the read side (the register verb — the production writer —
//! lands with T3), so the descriptor is seeded by writing `manifest.toml`
//! directly onto the harness's on-disk tenant-scoped
//! `/system/extensions/registered/<tenant>/<owner>/<id>/` tree — the exact
//! layout `RegisteredExtensionStore::list_for_scope` reads. Drives the AGENT
//! path (`builtin.extension_search`/`builtin.extension_install`/
//! `builtin.extension_activate`/`builtin.extension_remove` via real
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
//!
//! Tenant-axis seam limitation: the group harness resolves every actor's
//! binding through ONE `ProductInstallationScope`, so every thread —
//! `with_actor_id` included — shares the group's single run tenant; a second
//! tenant_id cannot be expressed without a second product harness + runtime,
//! which the one-runtime group deliberately does not support. Cross-tenant
//! isolation (same user id, different tenant) is pinned at the crate tier
//! instead: `registered_mutations_reject_same_user_in_foreign_tenant_scope`
//! in `extension_lifecycle.rs` drives search/install/activate/remove on the
//! facade with a second-tenant `ResourceScope`.

use super::reborn_support::assertions::ToolErrorClass;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Search term only the seeded fixture's name carries — a search hit is
/// unambiguous evidence the registered descriptor was surfaced. Distinct from
/// the extension id: R1's `registered_package_has_minted_id` gate means the id
/// itself must be minted per-owner (see `minted_extension_id` below), so the
/// search/isolation assertions key off this stable name term instead.
const REGISTERED_EXTENSION_SEARCH_TERM: &str = "acme";

/// Hosted MCP URL the fixture registers against; part of the mint input
/// alongside (tenant, owner), so it must match between the minted id and the
/// manifest's `[runtime].url`.
const REGISTERED_MCP_URL: &str = "http://127.0.0.1:9/mcp";

/// Registered-extension manifest TOML, parameterized by the minted id — R1
/// gates every registered-store read on recomputing
/// `HostedMcpExtensionId::mint(tenant, owner, url, "")` and matching it
/// against the descriptor's id, so a bare literal id here would silently fail
/// to round-trip on every list/search/install call.
fn registered_manifest_toml(minted_id: &str) -> String {
    format!(
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "{minted_id}"
name = "Acme Support MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (T1 fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "{REGISTERED_MCP_URL}"
"#
    )
}

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (default actor) ──────────────────────────────────────────────
    let a = g
        .thread("conv-ext-reg-iso-a-search")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({ "query": REGISTERED_EXTENSION_SEARCH_TERM }),
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
    // R1 gates every registered-store read on recomputing this exact mint and
    // matching it against the descriptor's id (`registered_package_has_minted_id`
    // in `registered_extension_store.rs`) — a bare literal id would silently
    // fail to round-trip through search/install/activate/remove.
    let extension_id =
        ironclaw_reborn_composition::test_support::mint_registered_mcp_extension_id_for_test(
            &a.binding.tenant_id,
            owner_user_id,
            REGISTERED_MCP_URL,
        );
    let extension_id_str = extension_id.as_str();
    let manifest_dir = capability_harness.storage_root_for_test().join(format!(
        "system/extensions/registered/{}/{}/{}",
        a.binding.tenant_id.as_str(),
        owner_user_id.as_str(),
        extension_id_str
    ));
    std::fs::create_dir_all(&manifest_dir)
        .map_err(|e| format!("[seed] create registered manifest dir: {e}"))?;
    std::fs::write(
        manifest_dir.join("manifest.toml"),
        registered_manifest_toml(extension_id_str),
    )
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
    a.assert_tool_result_contains(extension_id_str)
        .await
        .map_err(|e| {
            format!(
                "owner A's search did not surface its own registered extension \
                 {extension_id_str}: {e}"
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
                json!({ "extension_id": extension_id_str }),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    a_install
        .submit_turn(&format!("install {extension_id_str}"))
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
        .join(format!("system/extensions/{extension_id_str}"));
    if shared_extension_root.exists() {
        return Err(format!(
            "owner-registered install must not materialize under the shared \
             /system/extensions/{extension_id_str} root: found {shared_extension_root:?}"
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
                .find(|row| row["extension_id"] == extension_id_str)
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
                json!({ "query": REGISTERED_EXTENSION_SEARCH_TERM }),
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
        .assert_tool_result_contains(extension_id_str)
        .await
        .is_ok()
    {
        return Err(format!(
            "isolation failure: actor B's extension_search surfaced actor A's \
             registered extension {extension_id_str}"
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
                json!({ "extension_id": extension_id_str }),
            ),
            RebornScriptedReply::text("install attempted"),
        ])
        .build()
        .await?;
    b_install
        .submit_turn(&format!("install {extension_id_str}"))
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

    // ── A activates its own registered install: the successful owner path ──
    let a_activate = g
        .thread("conv-ext-reg-iso-a-activate")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({ "extension_id": extension_id_str }),
            ),
            RebornScriptedReply::text("activated"),
        ])
        .build()
        .await?;
    a_activate
        .submit_turn(&format!("activate {extension_id_str}"))
        .await
        .map_err(|e| format!("[A activate submit] {e}"))?;
    a_activate
        .assert_tool_invoked("builtin.extension_activate")
        .await
        .map_err(|e| format!("[A activate invoked] {e}"))?;
    a_activate
        .assert_tool_result_contains("\"activated\":true")
        .await
        .map_err(|e| format!("[A activate must succeed for its own registered extension] {e}"))?;

    // ── B activates A's package id: must fail, masked as not-found ──────────
    let b_activate = g
        .thread("conv-ext-reg-iso-b-activate")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({ "extension_id": extension_id_str }),
            ),
            RebornScriptedReply::text("activate attempted"),
        ])
        .build()
        .await?;
    b_activate
        .submit_turn(&format!("activate {extension_id_str}"))
        .await
        .map_err(|e| format!("[B activate submit] {e}"))?;
    b_activate
        .assert_tool_invoked("builtin.extension_activate")
        .await
        .map_err(|e| format!("[B activate invoked] {e}"))?;
    b_activate
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .map_err(|e| format!("[B activate must fail, not silently succeed] {e}"))?;

    // ── B removes A's package id: must fail, masked as not-found ────────────
    // Driven BEFORE A's own remove below, so A's install still exists for
    // this denial to be meaningful (a genuinely-absent row would also
    // deny, making the assertion vacuous).
    let b_remove = g
        .thread("conv-ext-reg-iso-b-remove")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({ "extension_id": extension_id_str }),
            ),
            RebornScriptedReply::text("remove attempted"),
        ])
        .build()
        .await?;
    b_remove
        .submit_turn(&format!("remove {extension_id_str}"))
        .await
        .map_err(|e| format!("[B remove submit] {e}"))?;
    b_remove
        .assert_tool_invoked("builtin.extension_remove")
        .await
        .map_err(|e| format!("[B remove invoked] {e}"))?;
    b_remove
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .map_err(|e| format!("[B remove must fail, not silently succeed] {e}"))?;

    // ── A removes its own registered install: the successful owner path ────
    let a_remove = g
        .thread("conv-ext-reg-iso-a-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({ "extension_id": extension_id_str }),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    a_remove
        .submit_turn(&format!("remove {extension_id_str}"))
        .await
        .map_err(|e| format!("[A remove submit] {e}"))?;
    a_remove
        .assert_tool_invoked("builtin.extension_remove")
        .await
        .map_err(|e| format!("[A remove invoked] {e}"))?;
    a_remove
        .assert_tool_result_contains("\"removed\":true")
        .await
        .map_err(|e| format!("[A remove must succeed for its own registered extension] {e}"))?;

    Ok(())
}
