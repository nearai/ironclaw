//! Caller-level contract for Routine final-reply presentation.
//!
//! The scripted model deliberately tries to repeat a raw cron expression,
//! internal field name, and capability id after a real `trigger_create` call.
//! The assertion reads the finalized transcript, proving the capability-owned
//! safe presentation survives the runtime/result-writer/agent-loop chain and
//! replaces the model-authored leak at the product boundary.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const RAW_CRON: &str = "*/17 * * * *";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let harness = g
        .thread("routine-final-reply-boundary")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": "Boundary check routine",
                    "prompt": "Perform the boundary check",
                    "schedule": {
                        "kind": "cron",
                        "expression": RAW_CRON,
                        "timezone": "UTC"
                    }
                }),
            ),
            RebornScriptedReply::text(format!(
                "Created trigger_id=secret-id with {RAW_CRON} via builtin.trigger_create"
            )),
        ])
        .build()
        .await?;

    harness
        .submit_turn("create the boundary check routine")
        .await?;
    harness
        .assert_tool_invoked("builtin.trigger_create")
        .await?;

    let reply = harness.final_reply_text().await?;
    if !reply.contains("Routine created: Boundary check routine") {
        return Err(format!("expected deterministic Routine reply, got {reply:?}").into());
    }
    for internal in [
        RAW_CRON,
        "trigger_id",
        "secret-id",
        "builtin.trigger_create",
    ] {
        if reply.contains(internal) {
            return Err(format!("finalized Routine reply leaked {internal:?}: {reply:?}").into());
        }
    }

    Ok(())
}
