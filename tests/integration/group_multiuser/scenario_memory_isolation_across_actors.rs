//! C-MULTIUSER scenario: per-actor MEMORY isolation over the group's ONE shared
//! capability backend. Actor A writes a private memory; a DISTINCT actor B
//! cannot read or search it, while A still can — the tool-tier proof that
//! Reborn memory is scoped by the run's owner (`MemoryDocumentScope` keys the
//! path on the caller's user id, `crates/ironclaw_memory_native/src/path.rs`).
//!
//! Related QA: issue #5460 ("memories visible to every user in the workspace")
//! — the reporter noted the TOOL path is already isolated; this scenario pins
//! that tool-path isolation in-process, independent of the WebUI read surface
//! #5460 targets.
//!
//! Seam: `RebornIntegrationGroup::multiuser_memory_tools` builds the backend
//! with `with_run_owner_scoped_capability_dispatch`, so each actor's `memory_*`
//! call dispatches under its OWN `(tenant, user)` scope instead of the single
//! fixed capability user `builtin_tools` uses (which collapses all actors onto
//! one shared memory subtree — see `tests/reborn_group_memory/`).

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_host_runtime::{MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID};
use serde_json::json;

/// Distinctive marker only actor A ever writes.
const MARKER: &str = "osprey-owner-a-secret-42";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (the group's default actor): write a private memory ──────────
    let a = g
        .thread("conv-mem-iso-a-write")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "target": "memory",
                    "content": format!("remember the private launch codename {MARKER}"),
                    "append": false
                }),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await?;
    a.submit_turn("remember my secret codename")
        .await
        .map_err(|e| format!("[A write submit] {e}"))?;
    a.assert_tool_invoked(MEMORY_WRITE_CAPABILITY_ID)
        .await
        .map_err(|e| format!("[A write invoked] {e}"))?;

    // ── Actor B (DISTINCT actor, SAME shared backend): must NOT see A's memory
    let b = g
        .thread("conv-mem-iso-b-search")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({ "query": "private launch codename" }),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    // Non-vacuity: if `with_actor_id` regressed to a no-op, both actors would
    // share one owner and this scenario would degrade to the same-owner case.
    if a.binding.subject_user_id == b.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both actors resolved the same owner".into());
    }
    b.submit_turn("find the launch codename")
        .await
        .map_err(|e| format!("[B search submit] {e}"))?;
    b.assert_tool_invoked(MEMORY_SEARCH_CAPABILITY_ID)
        .await
        .map_err(|e| format!("[B search invoked] {e}"))?;
    // ISOLATION: B's search over its OWN owner subtree finds nothing of A's.
    if b.assert_tool_result_contains(MARKER).await.is_ok() {
        return Err(
            "isolation failure (#5460): actor B's memory_search surfaced actor A's private memory"
                .into(),
        );
    }

    // ── Actor A again (SAME owner, new conversation): still finds its own memory
    // Proves B's miss is genuine isolation, not an empty/broken store.
    let a_reader = g
        .thread("conv-mem-iso-a-read")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({ "query": "private launch codename" }),
            ),
            RebornScriptedReply::text("recalled"),
        ])
        .build()
        .await?;
    a_reader
        .submit_turn("what was my codename")
        .await
        .map_err(|e| format!("[A read submit] {e}"))?;
    a_reader
        .assert_tool_invoked(MEMORY_SEARCH_CAPABILITY_ID)
        .await
        .map_err(|e| format!("[A read invoked] {e}"))?;
    a_reader
        .assert_tool_result_contains(MARKER)
        .await
        .map_err(|e| format!("[A read] owner must still see its own memory: {e}"))?;

    Ok(())
}
