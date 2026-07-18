//! Triggered-delivery outcome seam at int tier.
//!
//! `TriggeredRunDeliveryOutcomeKind` (Delivered/Denied/Skipped/Failed/
//! NoDefaultConfigured/TargetUnavailable) was previously observable ONLY at
//! crate tier (`ironclaw_reborn_composition::slack_delivery`'s `#[cfg(test)]`
//! module), which constructs `TriggeredRunDeliveryDriver` directly via `::new`,
//! never through the composition factory a real host binds. This proves the
//! REAL public factory — `build_triggered_run_delivery_hook(&runtime, &config,
//! delivery_store)` — assembles a working driver over a REAL local-dev
//! `RebornRuntime` (not a hand-built driver), and that the caller-supplied
//! `delivery_store` is genuinely the one it records through: a project-scoped
//! `TriggerFire` synchronously records `Denied` (`on_trigger_submitted`'s first
//! check, before any Slack egress/adapter is touched), read back via the exact
//! `Arc<InMemoryTriggeredRunDeliveryStore>` this test injected.
//!
//! Does not drive a live trigger-poller fire (that full path — pairing,
//! seeding a due `TriggerRecord`, polling for the poller to claim it — is
//! already proven at crate tier,
//! `build_slack_host_beta_mounts_wires_trigger_delivery_hook_writes_record`).
//! This test's marginal value is the FACTORY construction path
//! (`build_triggered_run_delivery_hook` over a real `RebornRuntime`), not the
//! poller.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_outbound::{
    InMemoryTriggeredRunDeliveryStore, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryStore,
};
use ironclaw_reborn_composition::{
    PostSubmitDeliveryHook, RebornBuildInput, RebornRuntimeInput, SlackHostBetaChannelRoute,
    SlackHostBetaConfig, SlackInstallationSelector, SlackTeamId, build_reborn_runtime,
    build_triggered_run_delivery_hook, local_dev_runtime_policy,
};
use ironclaw_triggers::{TriggerFire, TriggerFireIdentity, TriggerId};
use ironclaw_turns::{TurnRunId, TurnScope};
use secrecy::SecretString;

fn slack_host_beta_config(
    tenant_id: TenantId,
    agent_id: AgentId,
    user_id: UserId,
) -> SlackHostBetaConfig {
    SlackHostBetaConfig {
        tenant_id,
        agent_id,
        project_id: None,
        installation_id: ironclaw_product_adapters::AdapterInstallationId::new(
            "triggered-delivery-outcome-install",
        )
        .expect("installation id"),
        team_id: SlackTeamId::new("T-TRIGGERED-DELIVERY"),
        installation_selector: SlackInstallationSelector::team("T-TRIGGERED-DELIVERY"),
        user_id,
        shared_subject_user_id: None,
        channel_routes: Vec::<SlackHostBetaChannelRoute>::new(),
        signing_secret: SecretString::from("test-signing-secret"),
        bot_token: SecretString::from("test-bot-token"),
    }
}

#[tokio::test]
async fn build_triggered_run_delivery_hook_over_real_runtime_records_denied_for_project_scoped_fire()
 {
    let root = tempfile::tempdir().expect("tempdir");
    let policy = local_dev_runtime_policy().expect("local-dev runtime policy resolves");
    let input = RebornBuildInput::local_dev(
        "triggered-delivery-outcome-owner",
        root.path().join("local-dev"),
    )
    .with_runtime_policy(policy);
    let runtime = build_reborn_runtime(RebornRuntimeInput::from_services(input))
        .await
        .expect("local-dev runtime builds");

    let tenant_id = TenantId::new("tenant-triggered-delivery").expect("tenant");
    let agent_id = AgentId::new("agent-triggered-delivery").expect("agent");
    let user_id = UserId::new("user-triggered-delivery").expect("user");
    let config = slack_host_beta_config(tenant_id.clone(), agent_id.clone(), user_id.clone());

    // The real public factory, given OUR OWN injected store — the same
    // caller-supplied-store seam a real host binds, not a test-only shortcut.
    let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
    let driver = build_triggered_run_delivery_hook(&runtime, &config, delivery_store.clone())
        .expect("real driver builds over a real local-dev RebornRuntime");

    let run_id = TurnRunId::new();
    let trigger_id = TriggerId::new();
    let identity = TriggerFireIdentity::new(tenant_id.clone(), trigger_id, Utc::now());
    let project_id = ProjectId::new("some-project").expect("project id");
    let fire = TriggerFire {
        identity,
        creator_user_id: user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: Some(project_id.clone()),
        prompt: "triggered-delivery-outcome-seam".to_string(),
        delivery_target: None,
    };
    let scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        Some(project_id),
        ThreadId::new("thread-triggered-delivery-outcome").expect("thread id"),
        Some(user_id),
    );

    PostSubmitDeliveryHook::on_trigger_submitted(driver.as_ref(), fire, run_id, scope)
        .await
        .expect("managed hook persists the terminal outcome");

    let record = delivery_store
        .load_triggered_run_delivery(run_id)
        .await
        .expect("load succeeds")
        .expect("a record was written through OUR injected store");
    assert_eq!(
        record.outcome,
        TriggeredRunDeliveryOutcomeKind::Denied,
        "a project-scoped trigger fire must record Denied, read back through the exact store \
         this test supplied to the real composition factory"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}
