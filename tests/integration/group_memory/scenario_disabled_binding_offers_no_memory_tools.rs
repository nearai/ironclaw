//! Registration regression (F7 rework): the `Disabled` memory-binding shape —
//! a runtime registry with NO memory package — advertises ZERO
//! `ironclaw.memory.*` tools to the model. The tools are absent from the tool
//! surface entirely, not advertised-and-failing at call time. The positive
//! control (the default binding DOES offer `ironclaw__memory__search`) lives
//! in `scenario_memory_search_finds_seeded` on the shared group.
//!
//! Builds its own group: package registration is a per-group construction
//! input.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

pub async fn run() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools_without_memory().await?;
    let harness = group
        .thread("conv-memory-disabled-binding")
        .script([RebornScriptedReply::text("no memory here")])
        .build()
        .await?;
    harness.submit_turn("what tools do you have?").await?;
    harness.assert_reply_contains("no memory here").await?;

    for tool in [
        "ironclaw__memory__search",
        "ironclaw__memory__write",
        "ironclaw__memory__read",
        "ironclaw__memory__tree",
    ] {
        harness.assert_model_tool_not_offered(tool)?;
    }
    // The registry-lane surface is otherwise intact: a builtin tool is still
    // offered, so the memory absence is not a degenerate empty tool list.
    harness.assert_model_tool_offered("builtin__time")?;
    Ok(())
}
