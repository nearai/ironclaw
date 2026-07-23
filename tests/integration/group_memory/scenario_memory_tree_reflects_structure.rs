//! Scenario 3: seed a nested document in thread A, then assert `memory_tree`
//! reflects that directory structure when listed from thread B.
//!
//! `memory_tree` walks the shared workspace filesystem and returns a compact
//! JSON array where directories render as `"<name>/"` (with children nested as
//! `{"<name>/": [..]}`) and files render as plain strings. Seeding a path with
//! intermediate directories and then listing the root proves the tree surface
//! materializes the real structure end to end at int tier.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: writer ────────────────────────────────────────────────────
    // Write to a nested path so the tree must materialize an intermediate
    // directory (`atlas/`) and the leaf file (`runbook.md`).
    let writer = g
        .thread("conv-memory-tree-writer")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.memory_write",
                json!({
                    "target": "projects/atlas/runbook.md",
                    "content": "atlas service rollback runbook",
                    "append": false
                }),
            ),
            IronClawScriptedReply::text("seeded"),
        ])
        .build()
        .await?;
    writer.submit_turn("save the atlas runbook").await?;
    writer.assert_tool_invoked("builtin.memory_write").await?;

    // ── Thread B: lister (DIFFERENT conversation, SAME shared store) ────────
    // List from the root with enough depth to reach the leaf
    // (projects=1, atlas=2, runbook.md=3).
    let lister = g
        .thread("conv-memory-tree-lister")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.memory_tree",
                json!({"path": "", "depth": 3}),
            ),
            IronClawScriptedReply::text("listed"),
        ])
        .build()
        .await?;
    lister.submit_turn("show the memory tree").await?;
    lister.assert_tool_invoked("builtin.memory_tree").await?;
    // The serialized tree array must contain both the intermediate directory
    // and the leaf file, proving the structure was reflected (not dropped).
    lister.assert_tool_result_contains("atlas/").await?;
    lister.assert_tool_result_contains("runbook.md").await?;

    // Non-vacuity: an uncreated directory must be ABSENT, so the positive
    // assertions discriminate rather than pass unconditionally.
    if lister.assert_tool_result_contains("phantom/").await.is_ok() {
        return Err("negative guard failed: tree must not contain an uncreated directory".into());
    }

    Ok(())
}
