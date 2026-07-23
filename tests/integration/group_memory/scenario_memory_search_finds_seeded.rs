//! Scenario 2: seed a document in thread A, locate it via `memory_search` in
//! thread B over the shared store.
//!
//! `memory_search` projects each written document into FTS chunk records inside
//! the shared `RootFilesystem` (see `ironclaw_memory_native`'s repository chunk
//! projection). Because the group's threads share one underlying filesystem, a
//! search issued from a *different* conversation hits the chunks the writer
//! committed — exercising both the write→reindex path and the search-surface
//! path end to end at int tier.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: writer ────────────────────────────────────────────────────
    // Seed a short, distinctive sentence so the FTS snippet returned by search
    // contains the marker token verbatim.
    let writer = g
        .thread("conv-memory-search-writer")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.memory_write",
                json!({
                    "target": "memory",
                    "content": "remember that the staging rollback codename is osprey-meridian-7",
                    "append": false
                }),
            ),
            IronClawScriptedReply::text("seeded"),
        ])
        .build()
        .await?;
    writer.submit_turn("note the rollback codename").await?;
    writer.assert_tool_invoked("builtin.memory_write").await?;

    // ── Thread B: searcher (DIFFERENT conversation, SAME shared store) ──────
    // Query overlaps the seeded sentence; the matched chunk's snippet must
    // surface the marker token, proving search located the doc (not empty).
    let searcher = g
        .thread("conv-memory-searcher")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.memory_search",
                json!({"query": "staging rollback codename", "limit": 5}),
            ),
            IronClawScriptedReply::text("found"),
        ])
        .build()
        .await?;
    searcher
        .submit_turn("what is the rollback codename")
        .await?;
    searcher
        .assert_tool_invoked("builtin.memory_search")
        .await?;
    // The hit's snippet includes the marker → search located the seeded doc.
    searcher
        .assert_tool_result_contains("osprey-meridian-7")
        .await?;

    // Non-vacuity: an unwritten marker must be ABSENT, so the assertion above
    // is proven to discriminate rather than pass unconditionally.
    if searcher
        .assert_tool_result_contains("tungsten-mirage-88")
        .await
        .is_ok()
    {
        return Err(
            "negative guard failed: search result must not contain an unwritten marker".into(),
        );
    }

    Ok(())
}
