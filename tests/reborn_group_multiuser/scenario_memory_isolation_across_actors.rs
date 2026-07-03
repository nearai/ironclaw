//! C-MULTIUSER scenario: per-actor MEMORY isolation over the group's ONE shared
//! capability backend.
//!
//! Actor A writes a private memory; a DISTINCT actor B on the same group cannot
//! read or search it, while A still can. This is the tool/capability-tier proof
//! that Reborn memory is scoped by the run's owner — production keys the memory
//! document path on the caller's user id (`MemoryDocumentScope` →
//! `/memory/tenants/<tenant>/users/<user>/agents/<agent>/projects/<project>/…`,
//! `crates/ironclaw_memory_native/src/path.rs`), so two users in one workspace
//! never share a memory subtree.
//!
//! Related QA: issue #5460 ("Memories in the WebUI workspace are visible to
//! every user in the workspace"). The reporter noted the memory TOOL path is
//! already isolated ("the memories are isolated if you ask to expose other's
//! through tools") — this scenario pins exactly that tool-path isolation so a
//! regression at the capability tier (e.g. dropping the per-user path scope)
//! is caught in-process, independent of the WebUI read surface #5460 targets.
//!
//! The seam that makes this observable at int tier is
//! `RebornIntegrationGroup::multiuser_memory_tools` — it builds the capability
//! backend with `with_run_owner_scoped_capability_dispatch`, so each actor's
//! `memory_*` call dispatches under its OWN `(tenant, user)` scope instead of
//! the harness's single fixed capability user (which
//! `RebornIntegrationGroup::builtin_tools` uses, collapsing all actors onto one
//! shared memory subtree — see `tests/reborn_group_memory/`).

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Distinctive marker only actor A ever writes.
const MARKER: &str = "osprey-owner-a-secret-42";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (the group's default actor): write a private memory ──────────
    let a = g
        .thread("conv-mem-iso-a-write")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.memory_write",
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
    a.assert_tool_invoked("builtin.memory_write")
        .await
        .map_err(|e| format!("[A write invoked] {e}"))?;

    // ── Actor B (DISTINCT actor, SAME shared backend): must NOT see A's memory
    let b = g
        .thread("conv-mem-iso-b-search")
        .with_actor_id("reborn-actor-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.memory_search",
                json!({ "query": "private launch codename" }),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    // Non-vacuity: the seam must resolve a genuinely DISTINCT owner for B. If
    // `with_actor_id` regressed to a no-op, both actors would share one owner
    // and this scenario would degrade to the already-covered same-owner case.
    if a.binding.subject_user_id == b.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both actors resolved the same owner".into());
    }
    b.submit_turn("find the launch codename")
        .await
        .map_err(|e| format!("[B search submit] {e}"))?;
    b.assert_tool_invoked("builtin.memory_search")
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
    // Proves the write persisted and B's miss is genuine per-owner isolation,
    // not an empty/broken store (the banana-99 discrimination pattern applied to
    // the positive side).
    let a_reader = g
        .thread("conv-mem-iso-a-read")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.memory_search",
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
        .assert_tool_result_contains(MARKER)
        .await
        .map_err(|e| format!("[A read] owner must still see its own memory: {e}"))?;

    Ok(())
}
