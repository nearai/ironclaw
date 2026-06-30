//! Scenario 1 (HEADLINE): write MEMORY.md in thread A, read it in thread B.
//!
//! Both threads share the same `HostRuntimeCapabilityHarness` (via the group's
//! `Arc`), so they read and write to the same in-memory filesystem. This test
//! proves that cross-thread state written by one conversation is visible to a
//! completely different conversation over the shared store.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: writer ────────────────────────────────────────────────────
    // Write a distinctive marker string to MEMORY.md via `target: "memory"`.
    let writer = g
        .thread("conv-memory-writer")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.memory_write",
                json!({
                    "target": "memory",
                    "content": "the launch code is plum-42",
                    "append": false
                }),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await?;
    writer.submit_turn("remember the code").await?;
    writer.assert_tool_invoked("builtin.memory_write").await?;

    // ── Thread B: reader (DIFFERENT conversation, SAME shared store) ────────
    // A distinct `conversation_id` produces a distinct thread/binding, but the
    // underlying `HostRuntimeCapabilityHarness` is Arc-cloned, so the reader
    // sees the exact bytes the writer committed.
    let reader = g
        .thread("conv-memory-reader")
        .script([
            RebornScriptedReply::tool_call("builtin.memory_read", json!({"path": "MEMORY.md"})),
            RebornScriptedReply::text("recalled"),
        ])
        .build()
        .await?;
    reader.submit_turn("what was the code").await?;
    reader.assert_tool_invoked("builtin.memory_read").await?;
    // The tool result JSON includes `"content": "the launch code is plum-42"`;
    // asserting on the marker proves thread B reads thread A's write.
    reader.assert_tool_result_contains("plum-42").await?;

    // Committed negative guard (non-vacuity): a marker that was never written
    // must be ABSENT from the same read result, so `assert_tool_result_contains`
    // is proven to discriminate rather than pass unconditionally.
    if reader
        .assert_tool_result_contains("banana-99")
        .await
        .is_ok()
    {
        return Err(
            "negative guard failed: read result must not contain an unwritten marker".into(),
        );
    }

    Ok(())
}
