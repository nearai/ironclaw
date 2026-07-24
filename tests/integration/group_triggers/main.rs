//! Group integration tests for the trigger-management verbs at int tier.
//!
//! A [`RebornIntegrationGroup::triggers`] owns one shared
//! `HostRuntimeCapabilityHarness` (one trigger repository). The five verbs
//! (`trigger_create`/`list`/`pause`/`resume`/`remove`) are dispatched through
//! the real agent-loop turn → capability path — the only int-tier coverage of
//! these handlers (composition-tier `trigger_poller_e2e.rs` invokes only
//! `trigger_create` directly, and the one-shot fire → `Completed` derivation is
//! already covered there + in `repository_contract.rs`, so this binary does NOT
//! re-cover firing/completion/outbound — it fills the verb-dispatch gap).
//!
//! ## Why one sequential `#[tokio::test]`
//!
//! The scenario spans two threads over the SAME trigger scope: thread A mints a
//! `trigger_id` the static script cannot know ahead of time; thread B must run
//! after A to pause/resume/remove that id over the shared repo. One
//! orchestrating function gives deterministic ordering for free.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod scenario_delivery_target_fail_closed;
mod scenario_external_source_trigger_captures_delivery;
mod scenario_trigger_persists_after_reopen;
mod scenario_trigger_self_create_denied;
mod scenario_triggered_chained_gate;
mod scenario_triggered_gate;
mod scenario_triggered_gate_hold_visible;
mod scenario_verbs_lifecycle;
mod scenario_webui_automations_list;
mod scenario_webui_automations_rename;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn triggers_group_e2e() {
    let g = RebornIntegrationGroup::triggers()
        .await
        .expect("group builds");
    let mut report = ScenarioReport::new();

    // HEADLINE: create a one-shot Once trigger + list it in thread A, then
    // pause → resume → remove it by id in thread B over the shared repo.
    report.record("verbs_lifecycle", scenario_verbs_lifecycle::run(&g).await);
    // C-DURABLE: independent of `verbs_lifecycle` (its own trigger name/id) —
    // the trigger repository is always on-disk regardless of the group's
    // `StorageMode` (a separate capability-harness filesystem).
    report.record(
        "trigger_persists_after_reopen",
        scenario_trigger_persists_after_reopen::run(&g).await,
    );
    // W5-WEBUI-API-1: independent of `verbs_lifecycle` — mints its own
    // trigger, then lists it back through the real WebUI automations facade
    // over the group's shared trigger repository.
    report.record(
        "webui_automations_list",
        scenario_webui_automations_list::run(&g).await,
    );
    // W5-WEBUI-API-2: create a trigger, rename it through the real WebUI
    // automations route, then list it back from the shared trigger repo.
    report.record(
        "webui_automations_rename",
        scenario_webui_automations_rename::run(&g).await,
    );

    // Triggered-turn coverage map (E-TRIGGERED-SUBMIT via `submit_triggered_turn`)
    // — do NOT duplicate any of this here:
    //   - origin propagation: `tests/reborn_integration_triggered_submit.rs`
    //   - gate raise/approve/deny/resume: `triggered_gate_group` below
    //   - one-shot fire -> Completed: `trigger_poller_e2e.rs` + `repository_contract.rs`
    //   - reply persists in trigger's own thread: `reborn_integration_triggered_submit.rs`
    //   - push leg (trigger -> channel outbound delivery):
    //     `trigger_poller_e2e.rs::scheduled_trigger_results_reach_exact_slack_targets_once_across_restart`
    //     joins the production poller/run graph to the generic post-submit
    //     hook and real Slack adapter; `channel_host/e2e_tests.rs` retains
    //     the focused channel-host contracts.
    //
    // This grouped int-tier harness still does not expose the detached
    // post-submit hook directly; the composition whole-runtime test above
    // covers that asynchronous boundary through its durable outcome store.

    // C-DENYEDGE row 4: a scheduled-trigger fire must not be able to create
    // its own follow-up trigger. Uses THIS group's `triggers()` capability
    // port (trigger_create is wired) driven through a triggered-origin run
    // (`submit_triggered_turn_scripted`), independent of `verbs_lifecycle`'s
    // own trigger name/thread.
    report.record(
        "trigger_self_create_denied",
        scenario_trigger_self_create_denied::run(&g).await,
    );

    // Per-trigger delivery routing fails closed on a host with no outbound
    // delivery target providers: routed create rejected, nothing persisted.
    // Accept path is dispatch-tier + composition-tier (see scenario doc).
    report.record(
        "delivery_target_fail_closed",
        scenario_delivery_target_fail_closed::run(&g).await,
    );

    // A trigger created from an external product conversation must not be
    // persisted with no route back to that conversation. The host owns the
    // current sealed reply target; correctness cannot depend on the model
    // remembering to list targets and copy an id into its arguments.
    report.record(
        "external_source_trigger_captures_delivery",
        scenario_external_source_trigger_captures_delivery::run(&g).await,
    );

    report.assert_all_passed();
}

/// Triggered-origin runs raise, park on, and resume from REAL approval gates,
/// exactly like interactive runs. Lives in this binary (not
/// `reborn_group_approvals`) because the subject is the TRIGGERED submit wire.
/// Separate `#[tokio::test]` (own `live_approvals` group per arm) because the
/// `triggers()` group above has auto-approve ENABLED — a gate can never raise
/// there.
#[tokio::test]
async fn triggered_gate_group() {
    let mut report = ScenarioReport::new();

    let g_approve = RebornIntegrationGroup::live_approvals()
        .await
        .expect("approve-arm group builds");
    report.record(
        "triggered_gate_approve",
        scenario_triggered_gate::run_approve(&g_approve).await,
    );

    let g_deny = RebornIntegrationGroup::live_approvals()
        .await
        .expect("deny-arm group builds");
    report.record(
        "triggered_gate_deny",
        scenario_triggered_gate::run_deny(&g_deny).await,
    );

    // C-DENYEDGE row 1: a resume for the right run_id but a mutated
    // (wrong-tenant) TurnScope must be rejected with ScopeNotFound, gate still
    // live/resolvable after. Own group: isolates the mutation from the arms above.
    let g_wrong_scope = RebornIntegrationGroup::live_approvals()
        .await
        .expect("wrong-scope-arm group builds");
    report.record(
        "triggered_gate_wrong_scope_resume_rejected",
        scenario_triggered_gate::run_wrong_scope_resume_rejected(&g_wrong_scope).await,
    );

    // C-JOURNEY: a triggered fire raises a gate, resolves, then CHAINS into a
    // SECOND gate in the SAME run — pins ScheduledTrigger origin across BOTH
    // resume hops plus reply persistence. Own group: isolates the two-gate case
    // from the single-gate arms above.
    let g_chained = RebornIntegrationGroup::live_approvals()
        .await
        .expect("chained-gate-arm group builds");
    report.record(
        "triggered_gate_chained_approve",
        scenario_triggered_chained_gate::run_chained_approve(&g_chained).await,
    );

    // #5886 RED: a gate-parked triggered fire must surface a derived
    // active_hold on both read surfaces. Own combined group: trigger verbs
    // need auto-approve ON while write_file gates via an AskEachTime override
    // — neither `triggers()` nor `live_approvals()` offers both.
    let g_hold = RebornIntegrationGroup::triggers_with_gated_write()
        .await
        .expect("hold-visibility group builds");
    report.record(
        "triggered_gate_hold_visible",
        scenario_triggered_gate_hold_visible::run(&g_hold).await,
    );

    report.assert_all_passed();
}
