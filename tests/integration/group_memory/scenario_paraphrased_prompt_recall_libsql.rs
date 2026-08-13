//! Production-path regression for #7185: a fact saved as one sentence is
//! recalled by a differently-worded question that shares only SOME of its
//! content words.
//!
//! Retrieval used to require EVERY non-stopword term of the raw user message
//! (`Filter::Fts` is an AND), so conversational recall essentially never
//! matched: the reader's question below shares `sarah` / `standup` /
//! `scheduled` with the stored sentence but adds `like`, which the stored
//! sentence does not contain. One absent term was enough to return nothing.
//! Ranked OR retrieval (`Filter::FtsRanked`, bm25 on the shipping libSQL
//! backend) matches on any content term and orders by relevance.
//!
//! The fact is deliberately written to a NON-standing document
//! (`notes/standup.md`), not `MEMORY.md`: the always-on standing-document lane
//! added in #7365 prepends `MEMORY.md` regardless of the query, so writing
//! there would make this assertion pass without the search path working at
//! all.

use ironclaw_host_runtime::MEMORY_WRITE_CAPABILITY_ID;
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// Never written anywhere. Guards the positive assertion against a prompt that
/// simply carries everything.
const UNWRITTEN_MARKER: &str = "wombat-31";

pub async fn run(group: &RebornIntegrationGroup) -> HarnessResult<()> {
    let writer = group
        .thread("conv-memory-paraphrase-writer")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "target": "notes/standup.md",
                    "content": "Sarah prefers the standup meeting scheduled early on Thursday mornings",
                    "append": false
                }),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await?;
    writer
        .submit_turn("Remember when Sarah wants the standup.")
        .await?;
    writer
        .assert_tool_invoked(MEMORY_WRITE_CAPABILITY_ID)
        .await?;
    drop(writer);

    // A different conversation, so only the host-managed proactive prompt lane
    // can satisfy the assertion — the reader makes no memory tool call.
    //
    // Content terms of this question: sarah, like, standup, scheduled. The
    // stored sentence carries three of the four and NOT `like`, so every-term
    // (AND) matching returns nothing and this assertion fails.
    let reader = group
        .thread("conv-memory-paraphrase-reader")
        .script([RebornScriptedReply::text("answered")])
        .build()
        .await?;
    reader
        .submit_turn("when does Sarah like her standup scheduled")
        .await?;
    reader
        .assert_system_prompt_contains("Thursday mornings")
        .await?;
    reader
        .assert_system_prompt_excludes(UNWRITTEN_MARKER)
        .await?;

    Ok(())
}
