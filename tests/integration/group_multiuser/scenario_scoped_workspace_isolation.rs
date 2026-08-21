//! C-MULTIUSER scenario over a PER-CALLER-SCOPED workspace deployment — the
//! configuration `serve` composes unconditionally (PR #7062), driven through
//! real production turns:
//!
//! 1. A fresh caller whose `tenants/{tenant}/users/{user}` subtree has never
//!    been written errors with the pinned coding engines' `Path not found`
//!    text on read/glob/grep — the pinned surface does NOT invent an empty
//!    workspace for a caller nothing has ever written for (see
//!    `crates/app/ironclaw_composition/src/runtime/capability_host/workspace_scoping_tests.rs`
//!    `fresh_caller_reads_an_empty_workspace_then_writes_into_it`).
//! 2. A GATED `write` is approved and lands on disk under the caller's
//!    OWN subtree — the approval lease minted for the gate confines to the
//!    gate's own caller (per-caller `PolicyApprovalLeaseTermsProvider` over
//!    the runtime built with the scoping raise), and the resumed write
//!    creates the missing parent directories.
//! 3. Nothing lands at the shared workspace root.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-scoped-ws")
        .script([
            RebornScriptedReply::tool_call("builtin.glob", json!({"path": "/workspace"})),
            RebornScriptedReply::text("listed"),
            RebornScriptedReply::tool_call("builtin.glob", json!({"path": "**/*"})),
            RebornScriptedReply::text("globbed"),
            RebornScriptedReply::tool_call("builtin.grep", json!({"pattern": "anything"})),
            RebornScriptedReply::text("grepped"),
            RebornScriptedReply::tool_call(
                "builtin.write",
                json!({"path": "/workspace/notes/scoped.txt", "content": "scoped-body"}),
            ),
            RebornScriptedReply::text("wrote after gate"),
            RebornScriptedReply::tool_call("builtin.glob", json!({"path": "/workspace"})),
            RebornScriptedReply::text("listed after write"),
            RebornScriptedReply::tool_call("builtin.grep", json!({"pattern": "scoped-body"})),
            RebornScriptedReply::text("grepped after write"),
            RebornScriptedReply::tool_call(
                "builtin.glob",
                json!({"path": "/workspace/never-created"}),
            ),
            RebornScriptedReply::text("listed missing sub"),
        ])
        .build()
        .await?;
    let tenant = h.binding.tenant_id.as_str().to_string();
    let owner = h.binding.actor_user_id.clone();
    let scoped = |name: &str| format!("tenants/{tenant}/users/{}/{name}", owner.as_str());

    // ── Fresh caller: a never-written subtree errors with the pinned
    // `Path not found` text (the pinned engines do not fabricate an empty
    // workspace for a caller nothing has written for) ──
    h.submit_turn("list my fresh workspace")
        .await
        .map_err(|e| format!("[fresh glob must fail recoverably, not error the run] {e}"))?;
    h.assert_tool_error(
        super::reborn_support::assertions::ToolErrorClass::Failed,
        "operation_failed",
    )
    .await
    .map_err(|e| format!("[fresh glob must fail recoverably with the pinned diagnostic] {e}"))?;

    h.submit_turn("glob my fresh workspace")
        .await
        .map_err(|e| format!("[fresh glob must fail recoverably] {e}"))?;
    h.assert_tool_error(
        super::reborn_support::assertions::ToolErrorClass::Failed,
        "operation_failed",
    )
    .await
    .map_err(|e| format!("[fresh glob must fail recoverably with the pinned diagnostic] {e}"))?;

    h.submit_turn("grep my fresh workspace")
        .await
        .map_err(|e| format!("[fresh grep must fail recoverably] {e}"))?;
    h.assert_tool_error(
        super::reborn_support::assertions::ToolErrorClass::Failed,
        "operation_failed",
    )
    .await
    .map_err(|e| format!("[fresh grep must fail recoverably with the pinned diagnostic] {e}"))?;

    // ── Gated write approved → per-caller lease → lands in the OWN subtree ──
    // Global auto-approve defaults ON; force it OFF for this owner so the
    // write raises a REAL gate and the approval lease path is exercised.
    g.disable_auto_approve_for_owner(&owner)
        .await
        .map_err(|e| format!("[disable auto-approve] {e}"))?;
    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the nested scoped file")
        .await
        .map_err(|e| format!("[gated write must raise a real gate] {e}"))?;
    h.approve_gate(run_id, &gate_ref)
        .await
        .map_err(|e| format!("[approve] {e}"))?;
    h.wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .map_err(|e| format!("[resumed write completes] {e}"))?;
    h.assert_workspace_file_contains(&scoped("notes/scoped.txt"), "scoped-body")
        .await
        .map_err(|e| format!("[approved write must land in the caller's own subtree] {e}"))?;
    h.assert_workspace_file_absent("notes/scoped.txt")
        .await
        .map_err(|e| format!("[write must NOT land at the shared root] {e}"))?;

    // ── Once written, the subtree reads back as a REAL directory ───────────
    // Re-enable auto-approve so the remaining read turns dispatch ungated.
    g.enable_auto_approve_for_owner(&owner)
        .await
        .map_err(|e| format!("[re-enable auto-approve] {e}"))?;
    h.submit_turn("list my workspace after the write").await?;
    let listed_after = h.tool_result_output("builtin.glob").await?;
    let listed_after_text = listed_after["output"]
        .as_str()
        .ok_or("glob output must be an output text string")?;
    if !listed_after_text.contains("scoped.txt") {
        return Err(
            format!("written workspace must list the written file, got {listed_after}").into(),
        );
    }

    h.submit_turn("grep my workspace after the write").await?;
    let grepped_after = h.tool_result_output("builtin.grep").await?;
    let grepped_after_text = grepped_after["output"]
        .as_str()
        .ok_or("grep output must be an output text string")?;
    if !grepped_after_text.contains("scoped-body") {
        return Err(format!("grep must find the written file, got {grepped_after}").into());
    }

    // ── A missing SUB-path stays a hard error — only the root reads empty ──
    let baseline = h.history_len().await?;
    h.submit_turn("list a missing subdirectory").await?;
    h.assert_tool_error_since(
        baseline,
        super::reborn_support::assertions::ToolErrorClass::Failed,
        "operation_failed",
    )
    .await
    .map_err(|e| format!("[missing sub-path must stay a hard error] {e}"))?;
    Ok(())
}
