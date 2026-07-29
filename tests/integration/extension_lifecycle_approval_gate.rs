//! Proves `RebornIntegrationGroup::extension_lifecycle_no_auto_approve` (the
//! auto-approve-OFF sibling of `extension_lifecycle`) actually gates a
//! lifecycle operation: `builtin.extension_install` is `PermissionMode::Ask`,
//! so with auto-approve off it must raise a real `BlockedApproval` gate —
//! and, for discrimination, the SAME operation on the ordinary
//! `extension_lifecycle()` group (auto-approve ON) must complete without
//! ever raising a gate at all.
//!
//! This scenario keeps its own `[[test]]` target rather than folding into
//! `group_extensions/`: `tests/integration/CLAUDE.md` treats one binary per
//! scenario file as the norm, and this is a single-thread submit+assert
//! scenario, not a multi-thread group scenario. It is also the designated
//! home for auto-approve-OFF gate coverage, which will grow as more gates
//! land here (safety-scan rejection, the register gate) — folding it into an
//! unrelated tool-dispatch file would be the wrong seam. The alternative flat
//! files are already ~883 lines.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

const NEARAI_EXTENSION_ID: &str = "nearai";

#[tokio::test]
async fn extension_install_blocks_pending_approval_when_auto_approve_is_off() {
    // Discriminating arm: auto-approve ON (the existing `extension_lifecycle`
    // group) — the SAME install dispatches straight through, no gate raised.
    let auto_approve_group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("auto-approve extension-lifecycle group builds");
    let auto_approve_thread = auto_approve_group
        .thread("lifecycle-auto-approve-completes")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                serde_json::json!({"extension_id": NEARAI_EXTENSION_ID}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("auto-approve thread builds");
    auto_approve_thread
        .seed_capability_credential_account(NEARAI_EXTENSION_ID, "NEAR AI integration account", &[])
        .await
        .expect("NEAR AI account seeded under the dispatching user");
    auto_approve_thread
        .submit_turn("install NEAR AI search")
        .await
        .expect("turn completes gate-free under auto-approve");
    auto_approve_thread
        .assert_tool_result_contains(r#""installed":true"#)
        .await
        .expect("install dispatched straight through with auto-approve on");

    // Subject arm: same operation, same profile, auto-approve OFF — must
    // block pending a real approval instead of auto-completing.
    let no_auto_approve_group = RebornIntegrationGroup::extension_lifecycle_no_auto_approve()
        .await
        .expect("no-auto-approve extension-lifecycle group builds");
    let gated_thread = no_auto_approve_group
        .thread("lifecycle-blocks-pending-approval")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                serde_json::json!({"extension_id": NEARAI_EXTENSION_ID}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("gated thread builds");
    gated_thread
        .seed_capability_credential_account(NEARAI_EXTENSION_ID, "NEAR AI integration account", &[])
        .await
        .expect("NEAR AI account seeded — so blocking is due to the approval gate, not a missing credential");

    let (run_id, gate_ref) = gated_thread
        .submit_turn_until_blocked("install NEAR AI search")
        .await
        .expect("install blocks pending a real approval gate when auto-approve is off");

    // Resolve the gate to prove the block is a real, resumable approval gate
    // (not a terminal failure): approving lets the same install complete.
    gated_thread
        .approve_gate(run_id, &gate_ref)
        .await
        .expect("approving the gate resumes the run");
    gated_thread
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("run completes once the gate is approved");
    gated_thread
        .assert_tool_result_contains(r#""installed":true"#)
        .await
        .expect("install dispatched once approval was granted");
}
